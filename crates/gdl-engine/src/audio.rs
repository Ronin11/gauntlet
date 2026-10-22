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
    current: String,
    lib: Option<SoundLib>,
}

impl Audio {
    pub fn new(muted: bool) -> Self {
        // Android audio output isn't wired up yet.
        if muted || cfg!(target_os = "android") {
            return Audio {
                tx: None,
                current: String::new(),
                lib: None,
            };
        }
        let (tx, rx) = mpsc::channel::<Cmd>();
        #[cfg(target_os = "android")]
        let spawned: std::io::Result<()> = {
            drop(rx);
            Ok(())
        };
        #[cfg(not(target_os = "android"))]
        let spawned = std::thread::Builder::new()
            .name("audio".into())
            .spawn(move || {
                use rodio::Source;
                let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
                    log::warn!("no audio output device");
                    return;
                };
                let clips = |c: Vec<(Pcm, u32)>| {
                    c.into_iter()
                        .map(|(pcm, rate)| rodio::buffer::SamplesBuffer::new(1, rate, pcm.to_vec()))
                };
                // Keep the current music sink alive; replacing it stops the previous track.
                let mut _music: Option<rodio::Sink> = None;
                let voice = rodio::Sink::try_new(&handle).ok();
                while let Ok(cmd) = rx.recv() {
                    match cmd {
                        Cmd::Music(pcm, ch, rate) => {
                            if let Ok(s) = rodio::Sink::try_new(&handle) {
                                s.set_volume(0.5);
                                s.append(
                                    rodio::buffer::SamplesBuffer::new(ch, rate, pcm)
                                        .repeat_infinite(),
                                );
                                _music = Some(s);
                            }
                        }
                        Cmd::StopMusic => _music = None,
                        Cmd::Sfx(c, vol) => {
                            if let Ok(s) = rodio::Sink::try_new(&handle) {
                                s.set_volume(vol);
                                for b in clips(c) {
                                    s.append(b);
                                }
                                s.detach();
                            }
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
            Cmd::Sfx(all, vol * 0.8)
        });
    }

    /// Start the music for `level` (no-op if it is already playing).
    pub fn play_level_music(&mut self, fs: &GameFs, level: &str) {
        let Some(tx) = &self.tx else { return };
        for name in music_candidates(level) {
            let path = format!("STREAMS/{name}.ads");
            if !fs.exists(&path) {
                continue;
            }
            if self.current == name {
                return;
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
            return;
        }
        log::warn!("no music found for {level}");
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
