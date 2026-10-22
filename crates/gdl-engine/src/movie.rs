//! The game's VQ movies (VQMOVIES/*.avi), played as PlayVQMovie does: 30 frames a
//! second with the soundtrack, any newly pressed button ending the movie a couple
//! of frames later.

use crate::fs::GameFs;
use crate::input::PlayerInput;
use anyhow::{Context, Result};
use gdl_formats::movie::{Avi, MovieAudio, VideoDecoder};

pub struct Movie {
    bytes: Vec<u8>,
    avi: Avi,
    dec: VideoDecoder,
    /// Video chunks decoded so far.
    shown: usize,
    time: f32,
    /// Frames left after a skip (PlayVQMovie stops 2-3 frames after the press).
    stopping: Option<u32>,
    /// The soundtrack, taken by the caller to start playback.
    pub audio: Option<MovieAudio>,
    /// A button was held last tick (skips need a fresh press).
    held: bool,
    /// The picture changed since the caller last uploaded it.
    pub dirty: bool,
}

impl Movie {
    /// Open `VQMOVIES/<name>.avi`. Movies that aren't 512 wide are refused, as
    /// the game does.
    pub fn open(fs: &GameFs, name: &str) -> Result<Movie> {
        let bytes = fs.read(&format!("VQMOVIES/{name}.avi"))?;
        let avi = Avi::parse(&bytes).with_context(|| format!("{name}.avi"))?;
        anyhow::ensure!(avi.width() == 512, "{name}.avi is not 512 wide");
        let dec = VideoDecoder::for_avi(&avi)?;
        let audio = avi.decode_audio(&bytes).ok().flatten();
        Ok(Movie {
            bytes,
            avi,
            dec,
            shown: 0,
            time: 0.0,
            stopping: None,
            audio,
            held: true,
            dirty: false,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.dec.width() as u32, self.dec.height() as u32)
    }

    pub fn rgba(&self) -> Vec<u8> {
        self.dec.rgba()
    }

    /// Advance by `dt` seconds. Returns true once the movie has ended.
    pub fn update(&mut self, dt: f32, inputs: &[PlayerInput]) -> bool {
        let pressed = inputs.iter().any(|i| {
            i.attack
                || i.attack_pressed
                || i.magic_pressed
                || i.b_pressed
                || i.start_pressed
                || i.turbo
        });
        if pressed && !self.held && self.stopping.is_none() {
            self.stopping = Some(3);
        }
        self.held = pressed;
        self.time += dt;
        let want =
            ((self.time * self.avi.frame_rate()) as usize + 1).min(self.avi.video_chunks.len());
        while self.shown < want {
            let r = self.avi.video_chunks[self.shown].clone();
            if let Err(e) = self.dec.decode(&self.bytes[r]) {
                log::warn!("movie frame {}: {e:#}", self.shown);
            }
            self.shown += 1;
            self.dirty = true;
            if let Some(n) = &mut self.stopping {
                *n -= 1;
                if *n == 0 {
                    return true;
                }
            }
        }
        self.shown >= self.avi.video_chunks.len()
            && self.time * self.avi.frame_rate() >= self.avi.video_chunks.len() as f32
    }
}
