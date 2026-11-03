//! The game's VQ movies (VQMOVIES/*.avi), played as PlayVQMovie does: 30 frames a
//! second with the soundtrack, any newly pressed button ending the movie a couple
//! of frames later.

use crate::fs::GameFs;
use crate::input::PlayerInput;
use crate::session::MovieClock;
use anyhow::{Context, Result};
use gdl_formats::movie::{Avi, MovieAudio, VideoDecoder};

pub struct Movie {
    bytes: Vec<u8>,
    avi: Avi,
    dec: VideoDecoder,
    /// Video chunks decoded so far.
    shown: usize,
    /// When frames are due and when the movie ends.
    clock: MovieClock,
    /// The soundtrack, taken by the caller to start playback.
    pub audio: Option<MovieAudio>,
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
        let clock = MovieClock::new(avi.video_chunks.len(), avi.frame_rate());
        Ok(Movie {
            bytes,
            avi,
            dec,
            shown: 0,
            clock,
            audio,
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
        let done = self.clock.update(dt, inputs);
        self.show(self.clock.shown);
        done
    }

    /// Decode up to frame `n`, as a clock kept elsewhere says.
    pub fn show(&mut self, n: usize) {
        while self.shown < n.min(self.avi.video_chunks.len()) {
            let r = self.avi.video_chunks[self.shown].clone();
            if let Err(e) = self.dec.decode(&self.bytes[r]) {
                log::warn!("movie frame {}: {e:#}", self.shown);
            }
            self.shown += 1;
            self.dirty = true;
        }
    }
}
