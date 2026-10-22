//! The map screen shown while the next level loads (MG_MAPSCREEN, auxscreen.c
//! init_mapscreen / do_mapscreen).
//!
//! The realm map (`MAPS/level<name>`) is drawn in four tiles; the path to the level
//! appears one dash every 30 ticks, then the level's glow blinks, then the glow
//! fades out while the dimmed "loading" copy of the map fades in under a
//! "Loading..." glow text. Levels without map data (secret levels) show
//! TRANSITION_SCREEN for two seconds instead. No input is read: it cannot be
//! skipped.
//!
//! The caller plays [`MapScreen::music`] with `Audio::play_bank_music` when the
//! screen starts, plays the names queued in [`MapScreen::sfx`] each tick, and stops
//! the music (AudioStopMusicB) when [`MapScreen::update`] returns true. The
//! original also draws the player panels (WritePlayerInfo) on top every frame.

use crate::fs::GameFs;
use crate::gpu::Gpu;
use crate::hud::{Hud, TexBank};
use crate::screen::Scr;
use crate::ui::Tex;
use gdl_formats::wdata::{MapDef, Worlds};

/// init_mapscreen's timer argument: ticks the screen lasts without a map.
const TIMER: i32 = 120;
/// Ticks between dashes of the path.
const DOT_TICKS: i32 = 30;
/// map_fade_a / map_fade_b: the glow's blink ramp and pause (sdata 0x80343B50).
const FADE_A: i32 = 60;
const FADE_B: i32 = 10;
/// map_load_len: ticks the glow blinks before the fade (0x80343B58).
const LOAD_LEN: i32 = 210;
/// map_load_step: ticks of fade before the screen may end (0x80343B5C).
const LOAD_STEP: i32 = 180;
/// The tiles' top-left corners (0x80118250).
const TILES: [(f32, f32); 4] = [(0.0, 0.0), (256.0, 0.0), (0.0, 256.0), (256.0, 256.0)];
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

/// The map art of one level.
struct MapArt {
    bank: TexBank,
    name: String,
    def: MapDef,
}

impl MapArt {
    fn tex(&self, name: String) -> Option<Tex> {
        self.bank.get(&name)
    }
}

pub struct MapScreen {
    /// Sound names to play this tick (AudioMapDot), drained by the caller.
    pub sfx: Vec<String>,
    /// The bank sound to start as music (MapMusicStart), taken by the caller.
    pub music: Option<String>,
    art: Option<MapArt>,
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
    dots_shown: usize,
    /// The glow blit's MB fade, once it is shown.
    glow_fade: Option<i32>,
    /// The loading copy's MB fade (255 = invisible).
    ldmap_fade: i32,
    /// DrawGlowText "Loading..." this frame.
    loading: bool,
    finished: bool,
}

impl MapScreen {
    /// init_mapscreen for the level `wave` is about to load: its map art is
    /// uploaded through `hud` (release it with [`MapScreen::release`]).
    pub fn new(fs: &GameFs, gpu: &Gpu, hud: &mut Hud, worlds: &Worlds, wave: u32) -> MapScreen {
        let art = worlds
            .map(wave)
            .zip(worlds.level(wave))
            .and_then(|(def, l)| {
                let dir = format!("MAPS/level{}", l.name);
                match hud.load_bank(&gpu.device, &gpu.queue, fs, &dir) {
                    Ok(bank) => Some((bank, l.name.clone(), *def)),
                    Err(e) => {
                        log::warn!("map screen {dir}: {e:#}");
                        None
                    }
                }
            });
        MapScreen::with_art(worlds, wave, art)
    }

    /// The map screen with already loaded art: (bank, level name, map record), or
    /// None for the TRANSITION_SCREEN fallback.
    pub fn with_art(worlds: &Worlds, wave: u32, art: Option<(TexBank, String, MapDef)>) -> Self {
        MapScreen {
            sfx: Vec::new(),
            music: map_music(worlds, wave),
            art: art.map(|(bank, name, def)| MapArt { bank, name, def }),
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

    /// Does the screen show a realm map (rather than TRANSITION_SCREEN)?
    pub fn has_map(&self) -> bool {
        self.art.is_some()
    }

    /// Free the map's textures.
    pub fn release(&mut self, hud: &mut Hud) {
        if let Some(art) = self.art.take() {
            hud.free_bank(art.bank);
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
        if let Some(art) = &self.art {
            let n = art.def.dots().count();
            while self.dots_shown < n && self.progress > DOT_TICKS * self.dots_shown as i32 {
                self.dots_shown += 1;
                self.sfx.extend(self.dot_sound.clone());
            }
            if art.def.has_glow() && self.dots_shown == n {
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

    pub fn draw(&self, s: &mut Scr) {
        s.bars(1.0);
        s.rect(0.0, 0.0, 512.0, 384.0, [0.0, 0.0, 0.0, 1.0]);
        let Some(art) = &self.art else {
            if let Some(t) = s.ui.get("TRANSITION_SCREEN") {
                s.blit(t, 0.0, 0.0, 512.0, 320.0, 0);
            }
            return;
        };
        let blit = |s: &mut Scr, t: Option<Tex>, (x, y): (f32, f32), fade: i32| {
            if let Some(t) = t {
                s.blit(t, x.trunc(), y.trunc(), t.w as f32, t.h as f32, fade);
            }
        };
        let name = &art.name;
        // Back to front: map (depth 64100), glow (64080), dashes (64060), loading
        // copy (63950).
        for (i, &p) in TILES.iter().enumerate() {
            blit(s, art.tex(format!("MAP_{name}_{i:02}")), p, 0);
        }
        if let Some(fade) = self.glow_fade {
            let [x, y] = art.def.offset;
            blit(s, art.tex(format!("MAP_{name}GLOW")), (x, y), fade);
        }
        for (i, [x, y]) in art.def.dots().take(self.dots_shown).enumerate() {
            blit(s, art.tex(format!("DASH_{name}_{}", i + 1)), (x, y), 0);
        }
        for (i, &p) in TILES.iter().enumerate() {
            blit(
                s,
                art.tex(format!("LDMAP_{name}_{i:02}")),
                p,
                self.ldmap_fade,
            );
        }
        if self.loading {
            s.glow_text(1.0, 340.0, 320.0, "Loading...");
        }
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

    fn worlds() -> Option<(GameFs, Worlds)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = GameFs::open(&root).ok()?;
        let w = Worlds::load(|p| fs.read(p)).ok()?;
        Some((fs, w))
    }

    fn a6(w: &Worlds) -> MapScreen {
        let def = *w.map(0x101).unwrap();
        MapScreen::with_art(w, 0x101, Some((TexBank::default(), "A6".into(), def)))
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
        let mut m = MapScreen::with_art(&w, 0xC00, None);
        assert_eq!(m.music, None);
        let end = (1..=1000).find(|_| m.update()).unwrap();
        assert_eq!(end, TIMER as u32);
        assert!(m.sfx.is_empty());
    }

    #[test]
    fn sounds_exist() {
        let Some((fs, w)) = worlds() else { return };
        let mut names = Vec::new();
        for r in &w.realms {
            let wave = r.realm_type << 8;
            names.extend(map_music(&w, wave));
            names.extend(dot_sound(&w, wave));
        }
        assert_eq!(names.len(), 23);
        let found = crate::audio::check_sounds(&fs, &names);
        assert_eq!(found.len(), names.len());
        for (req, found) in found {
            assert!(found.is_some(), "{req} missing");
        }
    }

    #[test]
    fn map_banks_have_the_textures() {
        let Some((fs, w)) = worlds() else { return };
        for r in &w.realms {
            for (i, l) in r.levels.iter().enumerate() {
                let Some(def) = w.map((r.realm_type << 8) | i as u32) else {
                    continue;
                };
                let imgs = crate::ui::load_bank_images(&fs, &format!("MAPS/level{}", l.name))
                    .unwrap_or_else(|e| panic!("level{}: {e:#}", l.name));
                let has = |n: String| imgs.iter().any(|(k, _)| *k == n);
                let n = &l.name;
                for t in 0..4 {
                    assert!(has(format!("MAP_{n}_{t:02}")), "{n} map {t}");
                    assert!(has(format!("LDMAP_{n}_{t:02}")), "{n} ldmap {t}");
                }
                assert!(has(format!("MAP_{n}GLOW")), "{n} glow");
                for d in 1..=def.dots().count() {
                    assert!(has(format!("DASH_{n}_{d}")), "{n} dash {d}");
                }
            }
        }
    }
}
