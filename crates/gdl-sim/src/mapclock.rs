//! The map screen's clock (do_mapscreen): when the dashes appear, when the glow
//! blinks and fades, and when the screen is over. The engine draws what it says.

use gdl_formats::wdata::{MapDef, Worlds};

/// init_mapscreen's timer argument: ticks the screen lasts without a map.
pub const TIMER: i32 = 120;
/// Ticks between dashes of the path.
pub const DOT_TICKS: i32 = 30;
/// map_fade_a / map_fade_b: the glow's blink ramp and pause (sdata 0x80343B50).
pub const FADE_A: i32 = 60;
pub const FADE_B: i32 = 10;
/// map_load_len: ticks the glow blinks before the fade (0x80343B58).
pub const LOAD_LEN: i32 = 210;
/// map_load_step: ticks of fade before the screen may end (0x80343B5C).
pub const LOAD_STEP: i32 = 180;
/// map_load_state once loading has finished.
const LOADED: u32 = 99;

/// The realm letter LevelLetter(0) gives for a wave ('T' plays as 'G').
fn level_letter(worlds: &Worlds, wave: u32) -> Option<char> {
    let l = worlds.realm(wave >> 8)?.letter;
    Some(if l == 'T' { 'G' } else { l })
}

/// MapMusicStart: the map stinger `S_MAP_<letter>` (bank MAP_<letter>); none for
/// secret levels and the tower.
pub fn map_music(worlds: &Worlds, wave: u32) -> Option<String> {
    let l = level_letter(worlds, wave)?;
    (l != 'S' && l != 'L').then(|| format!("S_MAP_{l}"))
}

/// AudioMapDot: the dash sound for a realm type (lbl_8012341C, S_MAPDOTA..K for
/// realms 1..11; none for test and secret realms).
pub fn dot_sound(worlds: &Worlds, wave: u32) -> Option<String> {
    let realm = worlds.realm(wave >> 8)?;
    (1..=11)
        .contains(&realm.realm_type)
        .then(|| format!("S_MAPDOT{}", realm.letter))
}

#[derive(Clone, Debug)]
pub struct MapClock {
    /// Sound names to play this tick (AudioMapDot), drained by the caller.
    pub sfx: Vec<String>,
    /// The bank sound to start as music (MapMusicStart).
    pub music: Option<String>,
    /// The map shown; None for the TRANSITION_SCREEN fallback.
    pub def: Option<MapDef>,
    /// The level's name, which names its map art.
    pub name: String,
    dot_sound: Option<String>,
    /// map_load_state / map_load_delay: the next level's load steps.
    load_state: u32,
    load_delay: i32,
    /// map_load_timer: counts down once the map has fully faded.
    timer: i32,
    /// map_load_progress: ticks since the start, which reveal the dashes.
    progress: i32,
    /// map_fade_frame: ticks since the last dash.
    fade_frame: i32,
    pub dots_shown: usize,
    /// The glow blit's MB fade, once it is shown.
    pub glow_fade: Option<i32>,
    /// The loading copy's MB fade (255 = invisible).
    pub ldmap_fade: i32,
    /// DrawGlowText "Loading..." this frame.
    pub loading: bool,
    finished: bool,
}

impl MapClock {
    /// init_mapscreen for the level `wave` is about to load. Whether it shows a
    /// map is decided from the game's files, so every peer agrees.
    pub fn new(fs: &crate::fs::GameFs, worlds: &Worlds, wave: u32) -> MapClock {
        let def = worlds.map(wave).zip(worlds.level(wave)).and_then(|(def, l)| {
            let dir = format!("MAPS/level{}", l.name);
            (fs.exists(&format!("{dir}/objects.ngc")) && fs.exists(&format!("{dir}/textures.ngc")))
                .then_some(*def)
        });
        MapClock::with_def(worlds, wave, def)
    }

    pub fn with_def(worlds: &Worlds, wave: u32, def: Option<MapDef>) -> MapClock {
        MapClock {
            sfx: Vec::new(),
            music: map_music(worlds, wave),
            def,
            name: worlds.level(wave).map(|l| l.name.clone()).unwrap_or_default(),
            dot_sound: dot_sound(worlds, wave),
            load_state: 0,
            load_delay: 2,
            timer: TIMER,
            progress: 0,
            fade_frame: 0,
            dots_shown: 0,
            glow_fade: None,
            ldmap_fade: 255,
            loading: false,
            finished: false,
        }
    }

    /// do_mapscreen: one 60 Hz tick. Returns true once the screen is over.
    pub fn update(&mut self) -> bool {
        if self.finished {
            return true;
        }
        // The next level's load steps (AudioEnterNextStage, bank registration);
        // each finishes at once here.
        self.load_state = match self.load_state {
            1 if self.load_delay > 0 => {
                self.load_delay -= 1;
                1
            }
            s @ 0..=4 => s + 1,
            _ => LOADED,
        };
        self.progress += 1;
        let mut done = true;
        if let Some(def) = &self.def {
            let n = def.dots().count();
            while self.dots_shown < n && self.progress > DOT_TICKS * self.dots_shown as i32 {
                self.dots_shown += 1;
                self.sfx.extend(self.dot_sound.clone());
            }
            if def.has_glow() && self.dots_shown == n {
                self.fade_frame += 1;
                if self.fade_frame > LOAD_LEN {
                    let over = self.fade_frame - LOAD_LEN;
                    if over < LOAD_STEP {
                        done = false;
                    }
                    let fade = (over << 2).clamp(0, 255);
                    self.glow_fade = Some(fade);
                    self.ldmap_fade = 255 - fade;
                    self.loading = fade == 255;
                    if !self.loading {
                        self.timer = 60;
                    }
                } else {
                    self.glow_fade = Some(255 - blink_alpha(self.fade_frame));
                    self.timer = 60;
                }
            } else {
                self.timer = 60;
            }
        }
        self.timer -= 1;
        self.finished = self.load_state == LOADED && done && self.timer <= 0;
        self.finished
    }
}

/// The glow's blink while the map waits: a 0..60..0 ramp over 120 ticks and a
/// 10-tick pause, as a 4..250 alpha (the blit fade is 255 minus this).
fn blink_alpha(frame: i32) -> i32 {
    let mut rem = frame % (FADE_B + FADE_A * 2);
    if rem > FADE_A * 2 {
        rem = 0;
    } else if rem > FADE_A {
        rem = FADE_A * 2 - rem;
    }
    ((FADE_A + rem * 255 - 1) / FADE_A).clamp(4, 250)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::GameFs;

    fn worlds() -> Option<(GameFs, Worlds)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = GameFs::open(&root).ok()?;
        let w = Worlds::load(|p| fs.read(p)).ok()?;
        Some((fs, w))
    }

    fn a6(w: &Worlds) -> MapClock {
        let def = *w.map(0x101).unwrap();
        MapClock::with_def(w, 0x101, Some(def))
    }

    #[test]
    fn blink_ramp() {
        assert_eq!(blink_alpha(0), 4);
        assert_eq!(blink_alpha(60), 250);
        assert_eq!(blink_alpha(120), 4);
        assert_eq!(blink_alpha(125), 4);
        assert_eq!(blink_alpha(30), 128);
    }

    #[test]
    fn a6_timeline() {
        let Some((_, w)) = worlds() else {
            eprintln!("game data not found; skipping");
            return;
        };
        let mut m = a6(&w);
        assert_eq!(m.music.as_deref(), Some("S_MAP_A"));
        let mut dots_at = Vec::new();
        let mut end = 0;
        for tick in 1..=1000 {
            let fin = m.update();
            if m.sfx.drain(..).any(|s| s == "S_MAPDOTA") {
                dots_at.push(tick);
            }
            match tick {
                90 => assert_eq!(m.glow_fade, None),
                // The glow starts blinking with the last dash.
                91 => assert_eq!(m.glow_fade, Some(255 - 5)),
                // Fully faded 64 ticks into the fade, with "Loading...".
                363 => assert!(!m.loading && m.ldmap_fade == 3),
                364 => assert!(m.loading && m.ldmap_fade == 0 && m.glow_fade == Some(255)),
                _ => {}
            }
            if fin {
                end = tick;
                break;
            }
        }
        assert_eq!(dots_at, [1, 31, 61, 91]);
        // 390 fade frames from the last dash.
        assert_eq!(end, 480);
    }

    #[test]
    fn secret_level_falls_back() {
        let Some((_, w)) = worlds() else { return };
        assert!(w.map(0xC00).is_none());
        let mut m = MapClock::with_def(&w, 0xC00, None);
        assert_eq!(m.music, None);
        let end = (1..=1000).find(|_| m.update()).unwrap();
        assert_eq!(end, TIMER as u32);
        assert!(m.sfx.is_empty());
    }


}
