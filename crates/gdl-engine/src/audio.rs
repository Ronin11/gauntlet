//! Audio output: level music streamed from the disc's `.ads` files.

use crate::fs::GameFs;
use gdl_formats::audio::Ads;
use std::sync::mpsc;

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

enum Cmd {
    Play(Vec<i16>, u16, u32),
    Stop,
}

/// Owns the audio thread. Dropping it stops playback.
pub struct Audio {
    tx: Option<mpsc::Sender<Cmd>>,
    current: String,
}

impl Audio {
    pub fn new(muted: bool) -> Self {
        if muted {
            return Audio { tx: None, current: String::new() };
        }
        let (tx, rx) = mpsc::channel::<Cmd>();
        let spawned = std::thread::Builder::new().name("audio".into()).spawn(move || {
            let Ok((_stream, handle)) = rodio::OutputStream::try_default() else {
                log::warn!("no audio output device");
                return;
            };
            // Keep the current sink alive; replacing it stops the previous track.
            let mut _sink: Option<rodio::Sink> = None;
            while let Ok(cmd) = rx.recv() {
                match cmd {
                    Cmd::Play(pcm, ch, rate) => {
                        use rodio::Source;
                        if let Ok(s) = rodio::Sink::try_new(&handle) {
                            s.set_volume(0.6);
                            s.append(rodio::buffer::SamplesBuffer::new(ch, rate, pcm).repeat_infinite());
                            _sink = Some(s);
                        }
                    }
                    Cmd::Stop => _sink = None,
                }
            }
        });
        Audio { tx: spawned.ok().map(|_| tx), current: String::new() }
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
                    log::info!("music: {name} ({} Hz, {} ch, {:.0} s)", ads.sample_rate, ads.channels, pcm.len() as f32 / ads.channels as f32 / ads.sample_rate as f32);
                    let _ = tx.send(Cmd::Play(pcm, ads.channels as u16, ads.sample_rate));
                }
                Err(e) => log::warn!("music {name}: {e:#}"),
            });
            return;
        }
        log::warn!("no music found for {level}");
    }

    pub fn stop(&mut self) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Cmd::Stop);
        }
        self.current.clear();
    }
}
