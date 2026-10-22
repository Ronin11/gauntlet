//! Realm data: `WDATA/<realm>.WAD`, the per-realm level tables (gauntworld.c
//! LoadWorldData / ResolveWorldData), and the realm list `sWorldDataTypes`.
//!
//! A level is addressed by a *wave*, `(realm type << 8) | index`, where the index
//! is the position in the realm's LEVL table (play order). Its directory comes
//! from the level's 2-character name: wave 0x101 is castle index 1, "A6".

use anyhow::{bail, Context, Result};

/// sWorldDataTypes (0x8011C4A8): (type, WAD name, letter), in table order.
pub const REALMS: [(u32, &str, char); 14] = [
    (1, "CASTLE", 'A'),
    (2, "MOUNT", 'B'),
    (3, "DESERT", 'C'),
    (4, "FOREST", 'D'),
    (5, "TEMPLE", 'E'),
    (6, "HELL", 'F'),
    (7, "TOWN", 'G'),
    (8, "BATTLE", 'H'),
    (9, "ICE", 'I'),
    (10, "DREAM", 'J'),
    (11, "SKY", 'K'),
    (13, "TOWER", 'L'),
    (12, "SECRET", 'S'),
    (0, "TEST", 'T'),
];

/// The tower: realm 13, index 0 (sWorldDataConst).
pub const TOWER_WAVE: u32 = 0xD00;

/// One LEVL record (0x10C bytes).
#[derive(Clone, Debug, Default)]
pub struct LevelDef {
    /// Directory code ("A6" -> LEVELS/levelA6).
    pub name: String,
    pub flags: i32,
    pub enabled: i16,
    /// Seconds a secret level lasts.
    pub wavetime: i16,
    /// "IN", "ON" or "AT", for the map screen's "<prep> <title>".
    pub prep: String,
    pub title: String,
    pub audio: String,
    /// Intro movie played before the level (VQMOVIES), if any.
    pub movie: String,
    /// Boss type for the realm's last level, else 0.
    pub boss: i32,
    pub map_index: i16,
    /// Index into the realm's AUDS table.
    pub audio_index: i16,
    pub max_enemies: i16,
    pub rune: i16,
    pub legend: i16,
    pub player_level: f32,
    pub xp_mul: f32,
    pub damage: f32,
    pub difficulty: f32,
    /// The tally's pile scales: max gold, kills and experience.
    pub shop_max: [i32; 3],
}

/// One MAPS record (0x48 bytes, the level_data `mapdata` the map screen draws):
/// where the route glow sits and the dashes of the path to the level, in the
/// 512x384 virtual screen. Unused dash slots hold x = -1.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MapDef {
    /// Top-left of the `map_<name>glow` blit; the glow exists only when x >= 0.
    pub offset: [f32; 2],
    /// Top-left of each `dash_<name>_<i+1>` blit.
    pub dash: [[f32; 2]; 8],
}

impl MapDef {
    /// The dashes the map screen draws: do_mapscreen stops at the first x < 0.
    pub fn dots(&self) -> impl Iterator<Item = [f32; 2]> + '_ {
        self.dash.iter().copied().take_while(|d| d[0] >= 0.0)
    }

    /// Does the route glow exist (init_mapscreen: `offset.x >= 0`)?
    pub fn has_glow(&self) -> bool {
        self.offset[0] >= 0.0
    }
}

/// One AUDS record (audio_data, 60 bytes): the level's sound bank and music.
#[derive(Clone, Debug, Default)]
pub struct AudioDef {
    pub bank: String,
    /// Music stream name, or prefix when there are several areas or parts.
    pub stream: String,
    /// Music areas (streams `<stream>a`, `<stream>b`, ...).
    pub areas: i16,
    pub channels: i16,
    /// Sequential parts per area (`<name>_1`, `<name>_2`, ...).
    pub parts: [i16; 8],
}

impl AudioDef {
    /// AudioSetupLevelStreams: the stream files of an area, in play order.
    pub fn streams(&self, area: usize) -> Vec<String> {
        if self.stream.is_empty() {
            return Vec::new();
        }
        let area = area.min(self.areas.max(1) as usize - 1);
        let base = if self.areas <= 1 {
            self.stream.clone()
        } else {
            format!("{}{}", self.stream, (b'a' + area as u8) as char)
        };
        let parts = self.parts.get(area).copied().unwrap_or(1);
        if parts > 1 {
            (1..=parts).map(|n| format!("{base}_{n}")).collect()
        } else {
            vec![base]
        }
    }
}

#[derive(Clone, Debug)]
pub struct Realm {
    pub realm_type: u32,
    pub letter: char,
    pub levels: Vec<LevelDef>,
    pub audio: Vec<AudioDef>,
    /// MAPS records, indexed by [`LevelDef::map_index`].
    pub maps: Vec<MapDef>,
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

impl Realm {
    /// Parse a realm WAD. The directory is a run of 16-byte entries at the end of
    /// the file, each a reversed 4-character tag, an offset and a count.
    pub fn parse(d: &[u8], realm_type: u32, letter: char) -> Result<Realm> {
        let u32_at = |o: usize| -> Result<u32> {
            Ok(u32::from_le_bytes(
                d.get(o..o + 4).context("WAD truncated")?.try_into()?,
            ))
        };
        let mut dir = Vec::new();
        let mut i = d.len().checked_sub(16).context("WAD too short")?;
        loop {
            let tag: String = d[i..i + 4].iter().rev().map(|&c| c as char).collect();
            if !tag.chars().all(|c| c.is_ascii_alphanumeric()) {
                break;
            }
            dir.push((tag, u32_at(i + 4)? as usize, u32_at(i + 8)? as usize));
            if i < 16 {
                break;
            }
            i -= 16;
        }
        let find = |t: &str| dir.iter().find(|(n, _, _)| n == t).map(|(_, o, _)| *o);
        let (Some(w), Some(lo)) = (find("WRLD"), find("LEVL")) else {
            bail!("WAD has no WRLD/LEVL chunks");
        };
        let i16_at = |o: usize| i16::from_le_bytes([d[o], d[o + 1]]);
        let f32_at = |o: usize| f32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let i32_at = |o: usize| i32::from_le_bytes(d[o..o + 4].try_into().unwrap());
        let count = i16_at(w + 0x18).max(0) as usize;
        let mut levels = Vec::with_capacity(count);
        for k in 0..count {
            let o = lo + k * 0x10C;
            if o + 0x10C > d.len() {
                bail!("LEVL record {k} past end of WAD");
            }
            levels.push(LevelDef {
                flags: i32_at(o),
                enabled: i16_at(o + 4),
                name: cstr(&d[o + 8..o + 12]),
                wavetime: i16_at(o + 12),
                prep: cstr(&d[o + 0x10..o + 0x14]),
                title: cstr(&d[o + 0x14..o + 0x24]),
                audio: cstr(&d[o + 0x24..o + 0x34]),
                movie: cstr(&d[o + 0x34..o + 0x44]),
                boss: i32_at(o + 0x44),
                map_index: i16_at(o + 0x5C),
                audio_index: i16_at(o + 0x5A),
                max_enemies: i16_at(o + 0x8E),
                rune: i16_at(o + 0x90),
                legend: i16_at(o + 0x92),
                player_level: f32_at(o + 0x9C),
                xp_mul: f32_at(o + 0xA0),
                damage: f32_at(o + 0xA4),
                difficulty: f32_at(o + 0xA8),
                shop_max: [i32_at(o + 0xE0), i32_at(o + 0xE4), i32_at(o + 0xE8)],
            });
        }
        let mut audio = Vec::new();
        if let Some(ao) = find("AUDS") {
            let n = dir
                .iter()
                .find(|(t, _, _)| t == "AUDS")
                .map_or(0, |_| levels.len().max(1));
            for k in 0..n {
                let o = ao + k * 60;
                if o + 60 > d.len() {
                    break;
                }
                let mut parts = [0i16; 8];
                for (j, p) in parts.iter_mut().enumerate() {
                    *p = i16_at(o + 0x2C + 2 * j);
                }
                audio.push(AudioDef {
                    bank: cstr(&d[o..o + 16]),
                    stream: cstr(&d[o + 0x18..o + 0x28]),
                    areas: i16_at(o + 0x28),
                    channels: i16_at(o + 0x2A),
                    parts,
                });
            }
        }
        let mut maps = Vec::new();
        if let Some(&(_, mo, n)) = dir.iter().find(|(t, _, _)| t == "MAPS") {
            for k in 0..n {
                let o = mo + k * 0x48;
                if o + 0x48 > d.len() {
                    bail!("MAPS record {k} past end of WAD");
                }
                let xy = |j: usize| [f32_at(o + j * 8), f32_at(o + j * 8 + 4)];
                maps.push(MapDef {
                    offset: xy(0),
                    dash: std::array::from_fn(|j| xy(j + 1)),
                });
            }
        }
        Ok(Realm {
            realm_type,
            letter,
            levels,
            audio,
            maps,
        })
    }
}

/// Every realm's level table, for resolving waves.
#[derive(Clone, Debug, Default)]
pub struct Worlds {
    pub realms: Vec<Realm>,
}

impl Worlds {
    /// Load `WDATA/<name>.WAD` for every realm via `read`.
    pub fn load(mut read: impl FnMut(&str) -> Result<Vec<u8>>) -> Result<Worlds> {
        let mut realms = Vec::new();
        for (t, name, letter) in REALMS {
            let d = read(&format!("WDATA/{name}.WAD"))?;
            realms.push(Realm::parse(&d, t, letter).with_context(|| format!("{name}.WAD"))?);
        }
        Ok(Worlds { realms })
    }

    pub fn realm(&self, realm_type: u32) -> Option<&Realm> {
        self.realms.iter().find(|r| r.realm_type == realm_type)
    }

    /// The level a wave names.
    pub fn level(&self, wave: u32) -> Option<&LevelDef> {
        self.realm(wave >> 8)?.levels.get((wave & 0xFF) as usize)
    }

    /// FindWave: a level-data code ("a2" = the realm's second level in play order).
    pub fn find_wave(&self, code: &str) -> Option<u32> {
        let mut c = code.chars();
        let letter = c.next()?.to_ascii_uppercase();
        let n = c.next()?.to_digit(10)? as u32;
        let realm = self.realms.iter().find(|r| r.letter == letter)?;
        let idx = n.checked_sub(1)?;
        ((idx as usize) < realm.levels.len()).then_some((realm.realm_type << 8) | idx)
    }

    /// The wave whose directory is `levelXN`, if any realm lists it.
    pub fn wave_of_dir(&self, dir: &str) -> Option<u32> {
        let code = dir.trim_start_matches("level");
        self.realms.iter().find_map(|r| {
            let i = r
                .levels
                .iter()
                .position(|l| l.name.eq_ignore_ascii_case(code))?;
            Some((r.realm_type << 8) | i as u32)
        })
    }

    /// The music stream files for a wave's first area, in play order.
    pub fn music(&self, wave: u32) -> Vec<String> {
        let Some(realm) = self.realm(wave >> 8) else {
            return Vec::new();
        };
        let Some(level) = realm.levels.get((wave & 0xFF) as usize) else {
            return Vec::new();
        };
        realm
            .audio
            .get(level.audio_index.max(0) as usize)
            .map(|a| a.streams(0))
            .unwrap_or_default()
    }

    /// The map-screen record of a wave's level (level_data.mapdata), if it has one.
    pub fn map(&self, wave: u32) -> Option<&MapDef> {
        let realm = self.realm(wave >> 8)?;
        let level = realm.levels.get((wave & 0xFF) as usize)?;
        realm.maps.get(usize::try_from(level.map_index).ok()?)
    }

    /// The level directory for a wave ("levelA6").
    pub fn dir(&self, wave: u32) -> Option<String> {
        Some(format!("level{}", self.level(wave)?.name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worlds() -> Option<Worlds> {
        let root = std::env::var_os("GDL_GAME_DATA").map_or_else(
            || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data"),
            std::path::PathBuf::from,
        );
        let dir = root.join("files/Gauntlet");
        if !dir.join("WDATA").is_dir() {
            eprintln!("skipping: no {}", dir.display());
            return None;
        }
        Some(Worlds::load(|p| Ok(std::fs::read(dir.join(p))?)).unwrap())
    }

    #[test]
    fn maps_records() {
        let Some(w) = worlds() else { return };
        // Castle index 1 is A6 (Dungeon): glow at (127,171), four dashes.
        assert_eq!(w.level(0x101).unwrap().name, "A6");
        let a6 = w.map(0x101).expect("A6 has mapdata");
        assert_eq!(a6.offset, [127.0, 171.0]);
        let dots: Vec<_> = a6.dots().collect();
        assert_eq!(
            dots,
            [[56.0, 197.0], [58.0, 228.0], [84.0, 231.0], [113.0, 231.0]]
        );
        // A1 has a glow but no path.
        let a1 = w.map(0x100).unwrap();
        assert!(a1.has_glow());
        assert_eq!(a1.dots().count(), 0);
        // Secret levels have no map.
        assert!(w.realm(12).unwrap().maps.is_empty());
        assert!((0..9).all(|i| w.map(0xC00 | i).is_none()));
        for r in &w.realms {
            for m in &r.maps {
                assert!(m.has_glow());
                for d in m.dots() {
                    assert!(d[0] < 512.0 && d[1] >= 0.0 && d[1] < 384.0, "{d:?}");
                }
            }
        }
    }
}
