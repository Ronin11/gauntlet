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
//! The caller plays [`MapClock::music`] with `Audio::play_bank_music` when the
//! screen starts, plays the names queued in [`MapClock::sfx`] each tick, and stops
//! the music (AudioStopMusicB) when [`MapClock::update`] returns true. The
//! original also draws the player panels (WritePlayerInfo) on top every frame.

pub use gdl_sim::mapclock::{dot_sound, map_music, MapClock};

use crate::fs::GameFs;
use crate::gpu::Gpu;
use crate::hud::{Hud, TexBank};
use crate::screen::Scr;
use crate::ui::Tex;
use gdl_formats::wdata::MapDef;

/// The tiles' top-left corners (0x80118250).
const TILES: [(f32, f32); 4] = [(0.0, 0.0), (256.0, 0.0), (0.0, 256.0), (256.0, 256.0)];
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

/// The textures of a map screen; [`MapClock`] says what is showing.
pub struct MapScreen {
    art: Option<MapArt>,
}

impl MapScreen {
    /// The art of the map `clock` shows, uploaded through `hud` (release it with
    /// [`MapScreen::release`]).
    pub fn new(fs: &GameFs, gpu: &Gpu, hud: &mut Hud, clock: &MapClock) -> MapScreen {
        let art = clock.def.and_then(|def| {
            let dir = format!("MAPS/level{}", clock.name);
            match hud.load_bank(&gpu.device, &gpu.queue, fs, &dir) {
                Ok(bank) => Some(MapArt {
                    bank,
                    name: clock.name.clone(),
                    def,
                }),
                Err(e) => {
                    log::warn!("map screen {dir}: {e:#}");
                    None
                }
            }
        });
        MapScreen { art }
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

    pub fn draw(&self, clock: &MapClock, s: &mut Scr) {
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
        if let Some(fade) = clock.glow_fade {
            let [x, y] = art.def.offset;
            blit(s, art.tex(format!("MAP_{name}GLOW")), (x, y), fade);
        }
        for (i, [x, y]) in art.def.dots().take(clock.dots_shown).enumerate() {
            blit(s, art.tex(format!("DASH_{name}_{}", i + 1)), (x, y), 0);
        }
        for (i, &p) in TILES.iter().enumerate() {
            blit(
                s,
                art.tex(format!("LDMAP_{name}_{i:02}")),
                p,
                clock.ldmap_fade,
            );
        }
        if clock.loading {
            s.glow_text(1.0, 340.0, 320.0, "Loading...");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdl_formats::wdata::Worlds;

    fn worlds() -> Option<(GameFs, Worlds)> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let fs = GameFs::open(&root).ok()?;
        let w = Worlds::load(|p| fs.read(p)).ok()?;
        Some((fs, w))
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
