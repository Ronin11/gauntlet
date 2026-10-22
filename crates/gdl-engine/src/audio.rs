//! Audio output: level music streamed from the disc's `.ads` files, and sound effects /
//! announcer voices from the `.VBK` banks, looked up by name via `AUDATPS2.ROM`.

use crate::fs::GameFs;
use gdl_formats::audio::{parse_sound_names, Ads, Vbk, BANKS};
use std::collections::HashMap;
use std::sync::{mpsc, Arc};

/// Music track names for a level, best match first.
pub fn music_candidates(level: &str) -> Vec<String> {
    let code = level.trim_start_matches("level");
    let mut chars = code.chars();
    let world = chars.next().unwrap_or('A');
    let n: String = chars.collect();
    let prefix = match world {
        'A' => "CASTLE",
        'B' => "MOUNT",
        'C' => "DESERT",
        'D' => "FOREST",
        'E' | 'F' => "HELL",
        'G' => "ICE",
        'H' => "CATH",
        'I' => "TOWN",
        'J' => "DREAM",
        'K' => "SKY",
        'L' => "BATT",
        'S' => "SECRET",
        'T' => "tower",
        _ => "CASTLE",
    };
    let mut v = Vec::new();
    for suffix in ["", "A", "_1", "a", "b_1", "a_1", "A_1", "B"] {
        v.push(format!("{prefix}{n}{suffix}"));
    }
    v.push(format!("{prefix}1"));
    v.push(format!("{prefix}1A"));
    v
}

type Pcm = Arc<Vec<i16>>;

enum Cmd {
    Music(Vec<i16>, u16, u32),
    /// Streams played in order, the last one looping (a level's music parts).
    MusicParts(Vec<(Vec<i16>, u16, u32)>),
    /// Music volume (1.0 = the game's default setting).
    MusicVolume(f32),
    /// Stereo PCM played once, then looped from a frame index.
    MusicLoop(Vec<i16>, u32, Option<usize>),
    /// Played once on the music channel (a movie soundtrack).
    MusicOnce(Vec<i16>, u16, u32),
    StopMusic,
    /// Clips played back to back, at a sample rate and volume.
    Sfx(Vec<(Pcm, u32)>, f32),
    /// Announcer: queued after any line already playing.
    Voice(Vec<(Pcm, u32)>, f32),
}

/// Lazily loaded sound banks and decoded samples.
struct SoundLib {
    names: HashMap<String, (usize, usize, f32)>,
    banks: HashMap<usize, Option<(Vec<u8>, Vbk)>>,
    cache: HashMap<(usize, usize), (Pcm, u32)>,
}

impl SoundLib {
    fn load(fs: &GameFs) -> Option<Self> {
        let rom = fs.read("AUDIO/AUDATPS2.ROM").ok()?;
        let names = parse_sound_names(&rom);
        log::info!("sound effects: {} names", names.len());
        Some(SoundLib {
            names,
            banks: HashMap::new(),
            cache: HashMap::new(),
        })
    }

    fn has(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// Resolve a sound name to its clips and volume.
    fn resolve(&mut self, fs: &GameFs, name: &str) -> Option<(Vec<(Pcm, u32)>, f32)> {
        let &(bank, idx, dur) = self.names.get(name)?;
        let entry = self.banks.entry(bank).or_insert_with(|| {
            let file = format!("AUDIO/{}.VBK", BANKS.get(bank)?);
            let bytes = fs.read(&file).ok()?;
            let vbk = Vbk::parse(&bytes)
                .map_err(|e| log::warn!("{file}: {e:#}"))
                .ok()?;
            Some((bytes, vbk))
        });
        let (bytes, vbk) = entry.as_ref()?;
        let def = vbk.sounds.get(idx)?.clone();
        let mut clips = Vec::new();
        for &s in &def.samples {
            let clip = match self.cache.get(&(bank, s)) {
                Some(c) => c.clone(),
                None => {
                    let (pcm, rate) = vbk.decode_sample(bytes, s)?;
                    let c = (Arc::new(pcm), rate);
                    self.cache.insert((bank, s), c.clone());
                    c
                }
            };
            clips.push(clip);
        }
        // Some entries carry unrelated leading words; keep the run of clips (always
        // ending with the final one) whose length best matches the listed duration.
        let len = |c: &[(Pcm, u32)]| {
            c.iter()
                .map(|(p, r)| p.len() as f32 / *r as f32)
                .sum::<f32>()
        };
        let best = (0..clips.len()).min_by(|&a, &b| {
            (len(&clips[a..]) - dur)
                .abs()
                .total_cmp(&(len(&clips[b..]) - dur).abs())
        })?;
        Some((clips[best..].to_vec(), def.volume as f32 / 127.0))
    }
}

/// Owns the audio thread. Dropping it stops playback.
pub struct Audio {
    tx: Option<mpsc::Sender<Cmd>>,
    /// Sound-effect volume from the options (1.0 = the default 128).
    sfx_vol: f32,
    /// Music volume from the options, and the fade applied on top of it.
    music_vol: f32,
    music_fade: f32,
    current: String,
    lib: Option<SoundLib>,
}

impl Audio {
    pub fn new(muted: bool) -> Self {
        if muted {
            return Audio {
                sfx_vol: 1.0,
                music_vol: 1.0,
                music_fade: 1.0,
                tx: None,
                current: String::new(),
                lib: None,
            };
        }
        let (tx, rx) = mpsc::channel::<Cmd>();
        let spawned = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                use rodio::Source;
                let Ok(mut stream) = rodio::OutputStreamBuilder::open_default_stream() else {
                    log::warn!("no audio output device");
                    return;
                };
                stream.log_on_drop(false);
                let handle = stream.mixer().clone();
                // rodio mixes f32 samples.
                fn buf(ch: u16, rate: u32, pcm: &[i16]) -> rodio::buffer::SamplesBuffer {
                    let f: Vec<f32> = pcm.iter().map(|&v| v as f32 / 32768.0).collect();
                    rodio::buffer::SamplesBuffer::new(ch, rate, f)
                }
                let new_sink = || rodio::Sink::connect_new(&handle);
                let clips =
                    |c: Vec<(Pcm, u32)>| c.into_iter().map(|(pcm, rate)| buf(1, rate, &pcm));
                // Keep the current music sink alive; replacing it stops the previous track.
                let mut _music: Option<rodio::Sink> = None;
                let mut music_vol = 1.0f32;
                let voice = Some(new_sink());
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Music(pcm, ch, rate) => {
                            let s = new_sink();
                            s.set_volume(0.5 * music_vol);
                            s.append(buf(ch, rate, &pcm).repeat_infinite());
                            _music = Some(s);
                        }
                        Cmd::MusicOnce(pcm, ch, rate) => {
                            let s = new_sink();
                            s.append(buf(ch, rate, &pcm));
                            _music = Some(s);
                        }
                        Cmd::MusicLoop(pcm, rate, loop_start) => {
                            let s = new_sink();
                            s.set_volume(0.5 * music_vol);
                            let split = loop_start.map_or(pcm.len(), |f| (2 * f).min(pcm.len()));
                            let (intro, body) = pcm.split_at(split);
                            if !intro.is_empty() {
                                s.append(buf(2, rate, intro));
                            }
                            if !body.is_empty() {
                                s.append(buf(2, rate, body).repeat_infinite());
                            }
                            _music = Some(s);
                        }
                        Cmd::MusicParts(parts) => {
                            let s = new_sink();
                            s.set_volume(0.5 * music_vol);
                            let n = parts.len();
                            for (i, (pcm, ch, rate)) in parts.into_iter().enumerate() {
                                let b = buf(ch, rate, &pcm);
                                if i + 1 == n {
                                    s.append(b.repeat_infinite());
                                } else {
                                    s.append(b);
                                }
                            }
                            _music = Some(s);
                        }
                        Cmd::MusicVolume(v) => {
                            music_vol = v;
                            if let Some(s) = &_music {
                                s.set_volume(0.5 * v);
                            }
                        }
                        Cmd::StopMusic => _music = None,
                        Cmd::Sfx(c, vol) => {
                            let s = new_sink();
                            s.set_volume(vol);
                            for b in clips(c) {
                                s.append(b);
                            }
                            s.detach();
                        }
                        Cmd::Voice(c, vol) => {
                            if let Some(v) = &voice {
                                // Don't let announcements pile up.
                                if v.len() < 6 {
                                    v.set_volume(vol);
                                    for b in clips(c) {
                                        v.append(b);
                                    }
                                }
                            }
                        }
                    }
                }
            });
        Audio {
            sfx_vol: 1.0,
            music_vol: 1.0,
            music_fade: 1.0,
            tx: spawned.ok().map(|_| tx),
            current: String::new(),
            lib: None,
        }
    }

    /// Load the sound-effect name table (call once data is available).
    pub fn load_sounds(&mut self, fs: &GameFs) {
        if self.tx.is_some() && self.lib.is_none() {
            self.lib = SoundLib::load(fs);
        }
    }

    /// Does a sound with this name exist?
    pub fn has(&self, name: &str) -> bool {
        self.lib.as_ref().is_some_and(|l| l.has(name))
    }

    /// Play a named sound effect (`S_PICKUPKEY`); `A|B` tries alternatives in order. Names prefixed with `VOICE:` (or a
    /// `+`-joined list, e.g. `VOICE:S_REDWAR1+S_NEEDSFOOD+S_BADLY`) queue as announcements.
    pub fn play(&mut self, fs: &GameFs, name: &str) {
        let (Some(tx), Some(lib)) = (&self.tx, &mut self.lib) else {
            return;
        };
        // `A|B|C`: play the first alternative that exists.
        let name = name.split('|').find(|n| {
            let n = n.strip_prefix("VOICE:").unwrap_or(n);
            n.split('+').all(|p| lib.has(p))
        });
        let Some(name) = name else { return };
        let (voice, list) = match name.strip_prefix("VOICE:") {
            Some(rest) => (true, rest),
            None => (false, name),
        };
        let mut all = Vec::new();
        let mut vol: f32 = 1.0;
        for n in list.split('+') {
            if let Some((c, v)) = lib.resolve(fs, n) {
                all.extend(c);
                vol = vol.min(v.max(0.3));
            }
        }
        if all.is_empty() {
            return;
        }
        let _ = tx.send(if voice {
            Cmd::Voice(all, vol)
        } else {
            Cmd::Sfx(all, vol * 0.8 * self.sfx_vol)
        });
    }

    /// Play a level's music streams (`STREAMS/<name>.ads`) one after another,
    /// looping the last, as AudioSetupLevelStreams does for an area's parts.
    /// No-op if the same music is already playing.
    pub fn play_streams(&mut self, fs: &GameFs, names: &[String]) -> bool {
        let Some(tx) = &self.tx else { return true };
        let key = names.join("+");
        if names.is_empty() {
            return false;
        }
        if self.current == key {
            return true;
        }
        let mut files = Vec::new();
        for n in names {
            match fs.read(&format!("STREAMS/{n}.ads")) {
                Ok(b) => files.push((n.clone(), b)),
                Err(_) => {
                    log::warn!("no music stream {n}");
                    return false;
                }
            }
        }
        self.current = key;
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut parts = Vec::new();
            for (name, bytes) in files {
                match Ads::parse(&bytes) {
                    Ok(ads) => {
                        let pcm = ads.decode_all(&bytes);
                        parts.push((pcm, ads.channels as u16, ads.sample_rate));
                    }
                    Err(e) => {
                        log::warn!("music {name}: {e:#}");
                        return;
                    }
                }
            }
            let _ = tx.send(Cmd::MusicParts(parts));
        });
        true
    }

    /// Start the music for `level` (no-op if it is already playing).
    pub fn play_level_music(&mut self, fs: &GameFs, level: &str) {
        if !self.play_music(fs, &music_candidates(level)) {
            log::warn!("no music found for {level}");
        }
    }

    /// Play the first of these `STREAMS/*.ads` tracks that exists (no-op if it is
    /// already playing). Returns false if none exist.
    pub fn play_music(&mut self, fs: &GameFs, names: &[String]) -> bool {
        let Some(tx) = &self.tx else { return true };
        for name in names.iter().cloned() {
            let path = format!("STREAMS/{name}.ads");
            if !fs.exists(&path) {
                continue;
            }
            if self.current == name {
                return true;
            }
            let Ok(bytes) = fs.read(&path) else { continue };
            self.current = name.clone();
            let tx = tx.clone();
            // Decoding a long track takes a moment; do it off the main thread.
            std::thread::spawn(move || match Ads::parse(&bytes) {
                Ok(ads) => {
                    let pcm = ads.decode_all(&bytes);
                    log::info!(
                        "music: {name} ({} Hz, {} ch, {:.0} s)",
                        ads.sample_rate,
                        ads.channels,
                        pcm.len() as f32 / ads.channels as f32 / ads.sample_rate as f32
                    );
                    let _ = tx.send(Cmd::Music(pcm, ads.channels as u16, ads.sample_rate));
                }
                Err(e) => log::warn!("music {name}: {e:#}"),
            });
            return true;
        }
        false
    }

    /// The title / select music: S_SELECTMUS, a looping chain of SELECT.VBK
    /// samples played by the DCS driver.
    pub fn play_select_music(&mut self, fs: &GameFs) {
        self.play_bank_music(fs, "S_SELECTMUS");
    }

    /// Render a sound-bank call (DCS sample chain) and loop it as the music; no-op
    /// if it is already playing.
    pub fn play_bank_music(&mut self, fs: &GameFs, name: &str) {
        let (Some(tx), Some(lib)) = (&self.tx, &self.lib) else {
            return;
        };
        if self.current == name {
            return;
        }
        let Some(&(bank, idx, _)) = lib.names.get(name) else {
            log::warn!("no music sound {name}");
            return;
        };
        let Some(file) = BANKS.get(bank) else { return };
        let Ok(bytes) = fs.read(&format!("AUDIO/{file}.VBK")) else {
            return;
        };
        self.current = name.to_string();
        let (tx, name) = (tx.clone(), name.to_string());
        std::thread::spawn(move || {
            use gdl_formats::sequence::{render_call, DcsBank, PlayParams};
            let r = DcsBank::parse(&bytes)
                .and_then(|b| render_call(&b, &bytes, idx, &PlayParams::default()));
            match r {
                Ok(r) => {
                    let _ = tx.send(Cmd::MusicLoop(r.pcm, r.sample_rate, r.loop_start));
                }
                Err(e) => log::warn!("music {name}: {e:#}"),
            }
        });
    }

    /// Play interleaved PCM once on the music channel, replacing the music.
    pub fn play_once(&mut self, pcm: Vec<i16>, channels: u16, rate: u32) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::MusicOnce(pcm, channels, rate));
        }
        self.current = "<movie>".into();
    }

    /// Apply the Audio options: music and sfx volumes (128 is the default).
    pub fn set_volumes(&mut self, prefs: &crate::options::Prefs) {
        self.sfx_vol = prefs.sfx_vol as f32 / 128.0;
        self.music_vol = prefs.music_vol as f32 / 128.0;
        self.send_music_volume();
    }

    /// AudioMusicVolUpdate's fade (1.0 = full), on top of the music volume.
    pub fn set_music_fade(&mut self, f: f32) {
        if (f - self.music_fade).abs() > 1e-4 {
            self.music_fade = f;
            self.send_music_volume();
        }
    }

    fn send_music_volume(&self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::MusicVolume(self.music_vol * self.music_fade));
        }
    }

    pub fn stop(&mut self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::StopMusic);
        }
        self.current.clear();
    }
}

/// For testing: which alternative (if any) each sound request resolves to, and its length.
pub fn check_sounds(fs: &GameFs, requests: &[String]) -> Vec<(String, Option<(String, f32)>)> {
    let Some(mut lib) = SoundLib::load(fs) else {
        return Vec::new();
    };
    requests
        .iter()
        .map(|req| {
            let found = req.split('|').find_map(|alt| {
                let list = alt.strip_prefix("VOICE:").unwrap_or(alt);
                let mut total = 0.0;
                for n in list.split('+') {
                    let (clips, _) = lib.resolve(fs, n)?;
                    total += clips
                        .iter()
                        .map(|(p, r)| p.len() as f32 / *r as f32)
                        .sum::<f32>();
                }
                Some((alt.to_string(), total))
            });
            (req.clone(), found)
        })
        .collect()
}
