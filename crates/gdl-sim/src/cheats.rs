//! Name codes (player.c set_hidden_player): a hero named after one of the
//! `Hidden[]` codes plays as that hidden character (a base class with its own
//! model, PLAYERS/<class>/<dir>), and one named after a `Cheats[]` code gets
//! that power-up, gold, keys or potions each time a level loads.
//!
//! Not ported: the codes that also need both triggers held as the name is
//! taken (`any_level(0x100000)` and `any_level(0x400000)`): the MNTHRX /
//! ARIENT / AAAAAA prompt menu (Fly, Unlimited, NoDamage, Shards, Runes,
//! Cheats, Select a character, Worlds) and the random picks ADMBLY, ZANZI,
//! NICO and SAHARA.

/// One `Hidden[]` row (0x80120618, 27 x 0x24).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hidden {
    /// class_id (costume colour) given to the player.
    pub colour: usize,
    /// character given to the player (a base class, WAR..JES).
    pub class: usize,
    /// The 6-character name code.
    pub name: &'static str,
    /// The model directory under PLAYERS/<class>/ (hidden_code).
    pub dir: &'static str,
    /// 0: available; 1: only with more than one hidden-character level
    /// (lbl_80344828 > 1, which the retail game never sets).
    pub disable: i32,
}

const fn h(colour: usize, class: usize, name: &'static str, dir: &'static str) -> Hidden {
    Hidden {
        colour,
        class,
        name,
        dir,
        disable: 0,
    }
}

/// Hidden (player.c:754).
pub const HIDDEN: [Hidden; 27] = [
    h(2, 4, "ICE600", "GEI"),
    h(1, 4, "NUD069", "SNM"),
    h(1, 7, "STX222", "STK"),
    h(3, 7, "KJH105", "KJH"),
    h(0, 7, "PNK666", "PNK"),
    h(3, 5, "BAT900", "GEB"),
    h(2, 5, "TAK118", "NIN"),
    h(3, 5, "STG333", "STG"),
    h(2, 5, "KAO292", "WTR"),
    h(0, 5, "CSS222", "CSS"),
    h(1, 5, "RIZ721", "RIZ"),
    h(1, 5, "ARV984", "ARV"),
    h(2, 5, "DIB626", "DIB"),
    h(1, 5, "SJB964", "SJB"),
    Hidden {
        disable: 1,
        ..h(1, 1, "NAK069", "NUD")
    },
    h(3, 1, "TWN300", "GET"),
    h(1, 1, "AYA555", "SCH"),
    h(3, 1, "CEL721", "CEL"),
    h(1, 0, "CAS400", "GEC"),
    h(2, 0, "MTN200", "GEM"),
    h(2, 0, "RAT333", "RAT"),
    h(0, 2, "GARM99", "GA2"),
    h(3, 2, "GARM00", "GAM"),
    h(0, 2, "DES700", "GED"),
    h(3, 2, "SKY100", "GEP"),
    h(0, 2, "SUM224", "SUM"),
    h(3, 5, "DARTHC", "DCY"),
];

/// lbl_80344828: hidden-character level (gamemain.c sets 1).
const HIDDEN_LEVEL: i32 = 1;

/// One `Cheats[]` row (0x801209E4, 18 x 0x14).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PupCheat {
    pub name: &'static str,
    /// 1 gold, 2 keys, 4 potions (set to `add`), else a power-up type.
    pub kind: u8,
    pub add: f32,
    pub flags: u32,
}

const fn c(name: &'static str, kind: u8, add: f32, flags: u32) -> PupCheat {
    PupCheat {
        name,
        kind,
        add,
        flags,
    }
}

/// Cheats (player.c:784).
pub const CHEATS: [PupCheat; 18] = [
    c("INVULN", 6, 0.0, 65536),
    c("SSHOTS", 5, -1.0, 1048576),
    c("EGG911", 9, 0.0, 1024),
    c("1ANGEL", 9, 0.0, 1),
    c("1ANGEL", 6, 0.0, 524288),
    c("DELTA1", 9, 0.0, 768),
    c("000000", 9, 0.0, 4),
    c("PEEKIN", 9, 0.0, 2),
    c("PURPLE", 9, 0.0, 524288),
    c("XSPEED", 9, 4.0, 65536),
    c("QCKSHT", 5, 0.0, 536870912),
    c("MENAGE", 5, 0.0, 524288),
    c("REFLEX", 5, 0.0, 2097152),
    c("ALLFUL", 2, 9.0, 0),
    c("ALLFUL", 4, 9.0, 0),
    c("10000K", 1, 10000.0, 0),
    c("NOVATO", 9, 0.0, 8),
    c("MEBERT", 9, 0.0, 128),
];

/// `strncmp(save.name, code, 6) == 0`: the names as C strings, up to 6
/// characters.
pub fn name_is(name: &str, code: &str) -> bool {
    let n = name.as_bytes().iter().take(6);
    let c = code.as_bytes().iter().take(6);
    n.eq(c)
}

/// set_hidden_player's hidden-character half: the `Hidden[]` row a name
/// selects. DBRNKR forces pick 16 (AYA555).
pub fn hidden_for(name: &str) -> Option<usize> {
    let pick = name_is(name, "DBRNKR").then_some(16);
    HIDDEN.iter().enumerate().position(|(j, h)| {
        (name_is(name, h.name) && (h.disable == 0 || HIDDEN_LEVEL > 1)) || pick == Some(j)
    })
}

/// set_hidden_player's power-up half (from load_player_geo, every level
/// load, for a hero who is not a hidden character): each `Cheats[]` row the
/// name matches.
pub fn cheats_for(name: &str) -> impl Iterator<Item = &'static PupCheat> + '_ {
    CHEATS.iter().filter(move |c| name_is(name, c.name))
}

/// Apply the power-up cheats a name gives to a hero entering a level.
pub fn apply_cheats(name: &str, p: &mut crate::game::Player) {
    for c in cheats_for(name) {
        match c.kind {
            1 => p.gold = c.add as u32,
            // item_body_lo: keys.
            2 => p.keys = c.add as u32,
            // item_body_hi: potions.
            4 => {
                let n = c.add as usize;
                p.potion_colours.resize(n, 0);
                p.potions = n as u32;
            }
            // PlayerAddPowerup(add, p, type, flags, -1): strength -1 is
            // -powerup_time, a negative (lasting) time.
            k => {
                let t = -crate::stats::powerup_time(p.class_id);
                p.powerups.add(k, c.flags, c.add, t);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_as_c_strings() {
        assert!(name_is("ICE600", "ICE600"));
        assert!(name_is("ICE6001", "ICE600"));
        assert!(!name_is("ICE60", "ICE600"));
        assert!(name_is("ZANZI", "ZANZI"));
    }

    #[test]
    fn hidden_codes() {
        assert_eq!(hidden_for("ICE600"), Some(0));
        let h = HIDDEN[hidden_for("SUM224").unwrap()];
        assert_eq!((h.class, h.colour, h.dir), (2, 0, "SUM"));
        // DBRNKR picks row 16.
        assert_eq!(hidden_for("DBRNKR"), Some(16));
        assert_eq!(HIDDEN[16].name, "AYA555");
        // NAK069 needs a hidden-character level above 1.
        assert_eq!(hidden_for("NAK069"), None);
        assert_eq!(hidden_for("BOB"), None);
        assert_eq!(hidden_for("10000K"), None);
    }

    #[test]
    fn cheat_codes() {
        assert_eq!(cheats_for("1ANGEL").count(), 2);
        assert_eq!(cheats_for("ALLFUL").count(), 2);
        assert_eq!(cheats_for("ICE600").count(), 0);
    }

    fn data() -> Option<crate::fs::GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        crate::fs::GameFs::open(&root).ok()
    }

    /// Cheats land on a hero entering a level.
    #[test]
    fn cheats_apply_on_level_load() {
        let Some(fs) = data() else { return };
        let mut g = crate::game::Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let p = &mut g.players[0];
        apply_cheats("ALLFUL", p);
        assert_eq!((p.keys, p.potions, p.potion_colours.len()), (9, 9, 9));
        apply_cheats("INVULN", p);
        let s = p
            .powerups
            .slots
            .iter()
            .find(|s| s.time != 0.0)
            .expect("a power-up");
        assert_eq!((s.kind, s.flags), (6, 65536));
        assert!(s.time < 0.0, "strength -1: a lasting power-up");
        apply_cheats("BOB", p);
        assert_eq!(p.gold, 0);
    }

    /// Every hidden character's model loads (PLAYERS/<class>/<dir>).
    #[test]
    fn hidden_models_load() {
        let Some(fs) = data() else { return };
        for h in &HIDDEN {
            let class = crate::heroes::CLASSES[h.class];
            crate::character::Model::load_player(&fs, class, h.dir)
                .unwrap_or_else(|e| panic!("{}: {e:#}", h.name));
        }
    }

    /// The tables match the DOL (Hidden 0x80120618, Cheats 0x801209E4).
    #[test]
    fn tables_match_the_dol() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let Ok(dol) = std::fs::read(root.join("main.dol")) else {
            return;
        };
        let dol = gdl_formats::dol::Dol::new(dol);
        let u32_at = |a: u32| dol.u32_at(a).unwrap();
        let str_at = |a: u32| dol.str_at(a).unwrap();
        for (j, h) in HIDDEN.iter().enumerate() {
            let a = 0x8012_0618 + 0x24 * j as u32;
            assert_eq!(u32_at(a) as usize, h.colour, "{}", h.name);
            assert_eq!(u32_at(a + 4) as usize, h.class, "{}", h.name);
            assert_eq!(str_at(a + 8), h.name);
            assert_eq!(str_at(a + 16).to_ascii_uppercase(), h.dir);
            assert_eq!(u32_at(a + 32) as i32, h.disable, "{}", h.name);
        }
        for (j, c) in CHEATS.iter().enumerate() {
            let a = 0x8012_09E4 + 0x14 * j as u32;
            assert_eq!(str_at(a), c.name);
            assert_eq!(u32_at(a + 8), c.kind as u32, "{}", c.name);
            assert_eq!(f32::from_bits(u32_at(a + 12)), c.add, "{}", c.name);
            assert_eq!(u32_at(a + 16), c.flags, "{}", c.name);
        }
    }
}
