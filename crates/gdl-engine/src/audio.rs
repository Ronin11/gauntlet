//! Audio output: level music streamed from the disc's `.ads` files, and sound effects /
//! announcer voices from the `.VBK` banks, looked up by name via `AUDATPS2.ROM`.

use crate::fs::GameFs;
pub use crate::soundlib::check_sounds;
use crate::soundlib::{music_candidates, split_volume, Pcm, SoundLib};
use gdl_formats::audio::{Ads, BANKS};
use std::sync::mpsc;

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
    /// A looping ambient sound by id (a level's sound items), its volume, and
    /// stopping it / all of them.
    LoopStart(u64, Vec<(Pcm, u32)>, f32),
    LoopVolume(u64, f32),
    LoopStop(u64),
    LoopStopAll,
    /// Clips played back to back, at a sample rate and volume.
    Sfx(Vec<(Pcm, u32)>, f32),
    /// Announcer: queued after any line already playing.
    Voice(Vec<(Pcm, u32)>, f32),
}

/// Not in the original: everything plays at this share of the game's own
/// levels (the Audio options still scale on top), as full level is far too
/// loud on a PC or tablet. GDL_VOLUME=<0..1> overrides it.
pub fn master_gain() -> f32 {
    std::env::var("GDL_VOLUME")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map_or(0.2, |v| v.clamp(0.0, 1.0))
}

/// Owns the audio thread. Dropping it stops playback.
pub struct Audio {
    tx: Option<mpsc::Sender<Cmd>>,
    /// Sound-effect volume from the options (master_gain() = the default 128).
    sfx_vol: f32,
    /// Music volume from the options, and the fade applied on top of it.
    music_vol: f32,
    music_fade: f32,
    /// Ambient loops playing, by id, at their last volume.
    loops: std::collections::HashMap<u64, f32>,
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
                loops: Default::default(),
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
                let mut music_vol = master_gain();
                let voice = Some(new_sink());
                let mut loops: std::collections::HashMap<u64, rodio::Sink> = Default::default();
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Music(pcm, ch, rate) => {
                            let s = new_sink();
                            s.set_volume(0.5 * music_vol);
                            s.append(buf(ch, rate, &pcm).repeat_infinite());
                            _music = Some(s);
                        }
                        Cmd::MusicOnce(pcm, ch, rate) => {
                            // Movies follow the music volume like the tracks do.
                            let s = new_sink();
                            s.set_volume(0.5 * music_vol);
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
                        Cmd::LoopStart(id, c, vol) => {
                            let s = new_sink();
                            s.set_volume(vol);
                            // The clips back to back, looping.
                            let pcm: Vec<f32> = c
                                .iter()
                                .flat_map(|(p, _)| p.iter().map(|&v| v as f32 / 32768.0))
                                .collect();
                            let rate = c.first().map_or(22050, |c| c.1);
                            if !pcm.is_empty() {
                                s.append(
                                    rodio::buffer::SamplesBuffer::new(1, rate, pcm)
                                        .repeat_infinite(),
                                );
                                loops.insert(id, s);
                            }
                        }
                        Cmd::LoopVolume(id, vol) => {
                            if let Some(s) = loops.get(&id) {
                                s.set_volume(vol);
                            }
                        }
                        Cmd::LoopStop(id) => {
                            loops.remove(&id);
                        }
                        Cmd::LoopStopAll => loops.clear(),
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
            sfx_vol: master_gain(),
            music_vol: master_gain(),
            music_fade: 1.0,
            loops: Default::default(),
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

    /// Keep ambient sound `id` (`name`) looping at `vol` (0..1 before the
    /// effects volume); 0 stops it.
    pub fn ambient(&mut self, fs: &GameFs, id: u64, name: &str, vol: f32) {
        let (Some(tx), Some(lib)) = (&self.tx, &mut self.lib) else {
            return;
        };
        let v = vol * 0.8 * self.sfx_vol;
        match self.loops.get_mut(&id) {
            Some(_) if vol <= 0.0 => {
                self.loops.remove(&id);
                let _ = tx.send(Cmd::LoopStop(id));
            }
            Some(cur) => {
                if (*cur - v).abs() > 0.01 {
                    *cur = v;
                    let _ = tx.send(Cmd::LoopVolume(id, v));
                }
            }
            None if vol > 0.0 => {
                if let Some((clips, dv)) = lib.resolve(fs, name) {
                    let v = v * dv.max(0.3);
                    self.loops.insert(id, v);
                    let _ = tx.send(Cmd::LoopStart(id, clips, v));
                }
            }
            None => {}
        }
    }

    /// Stop every ambient sound (leaving a level).
    pub fn ambient_clear(&mut self) {
        if self.loops.is_empty() {
            return;
        }
        self.loops.clear();
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::LoopStopAll);
        }
    }

    /// Does a sound with this name exist?
    pub fn has(&self, name: &str) -> bool {
        self.lib.as_ref().is_some_and(|l| l.has(name))
    }

    /// Play a named sound effect (`S_PICKUPKEY`, or by id: `#2b0048`); `A|B` tries alternatives in order. Names prefixed with `VOICE:` (or a
    /// `+`-joined list, e.g. `VOICE:S_REDWAR1+S_NEEDSFOOD+S_BADLY`) queue as announcements.
    /// A trailing `@N` plays it at volume N of the game's 127 (`S_WARN@202`).
    pub fn play(&mut self, fs: &GameFs, name: &str) {
        let (Some(tx), Some(lib)) = (&self.tx, &mut self.lib) else {
            return;
        };
        let (name, gain) = split_volume(name);
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
        let vol = vol * gain;
        let _ = tx.send(if voice {
            Cmd::Voice(all, (vol * master_gain()).min(1.0))
        } else {
            Cmd::Sfx(all, (vol * 0.8 * self.sfx_vol).min(1.0))
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
        self.sfx_vol = master_gain() * prefs.sfx_vol as f32 / 128.0;
        self.music_vol = master_gain() * prefs.music_vol as f32 / 128.0;
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
