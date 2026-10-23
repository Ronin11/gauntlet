//! Monster types (enemy.c e_e_tpye), their base stats, and how a level's generic
//! names (GRU, RAT, KNI, SPECIAL) become the realm's real monsters (GetEnemyType).

/// Per-type data from the DOL tables (enemy.c:133-330).
#[derive(Clone, Copy, Debug)]
pub struct TypeInfo {
    /// Name used in WORLDS items and MONSTERS directories.
    pub desc: &'static str,
    /// Height (the collision half-height is half of it) and radius.
    pub height: f32,
    pub radius: f32,
    /// Move per 60 Hz tick (x ene_speed).
    pub speed: f32,
    pub health: f32,
    pub fight: f32,
    pub armor: f32,
}

const fn t(
    desc: &'static str,
    height: f32,
    radius: f32,
    speed: f32,
    health: f32,
    fight: f32,
) -> TypeInfo {
    TypeInfo {
        desc,
        height,
        radius,
        speed,
        health,
        fight,
        armor: 0.0,
    }
}

pub const GRUNT: usize = 4;
pub const KNIGHT: usize = 5;
pub const RAT: usize = 3;
pub const ACID: usize = 21;
pub const GOLEM: usize = 29;
pub const DEATH: usize = 30;
pub const IT: usize = 31;
/// Types up to here are "regular": their health scales with the tier.
pub const REGULAR: usize = 28;

pub const TYPES: [TypeInfo; 34] = [
    t("sco", 3.0, 1.5, 0.1, 21.0, 12.0),
    t("tro", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("dem", 6.0, 1.8, 0.12, 46.0, 18.0),
    t("rat", 3.0, 1.5, 0.1, 21.0, 12.0),
    t("gru", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("kni", 6.0, 1.8, 0.1, 46.0, 18.0),
    t("sna", 3.0, 1.5, 0.1, 21.0, 12.0),
    t("sor", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("mum", 6.0, 1.8, 0.1, 46.0, 18.0),
    t("spi", 3.0, 1.5, 0.1, 21.0, 12.0),
    t("liz", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("tre", 6.0, 1.8, 0.1, 46.0, 18.0),
    t("mag", 3.0, 1.5, 0.1, 21.0, 12.0),
    t("zom", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("pla", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("wol", 3.0, 1.5, 0.1, 21.0, 12.0),
    t("ice", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("wrm", 6.0, 1.8, 0.1, 46.0, 18.0),
    t("dog", 3.0, 1.5, 0.1, 21.0, 12.0),
    t("ske", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("gho", 6.0, 1.8, 0.1, 46.0, 18.0),
    t("aci", 3.0, 0.75, 0.02, 21.0, 12.0),
    t("han", 3.0, 0.75, 0.05, 21.0, 12.0),
    t("imp", 6.0, 2.0, 0.1, 30.0, 15.0),
    t("war", 6.0, 2.0, 0.1, 46.0, 15.0),
    t("sky", 6.0, 2.0, 0.1, 46.0, 15.0),
    t("wind", 6.0, 2.0, 0.1, 46.0, 15.0),
    t("grm", 10.0, 4.0, 0.1, 100.0, 20.0),
    t("", 6.0, 1.5, 0.1, 30.0, 15.0),
    t("golem", 12.0, 3.0, 0.09, 200.0, 20.0),
    TypeInfo {
        desc: "death",
        height: 6.0,
        radius: 1.5,
        speed: 0.125,
        health: 100.0,
        fight: 1.0,
        armor: 1.0,
    },
    t("it", 5.0, 1.5, 0.1, 9999.0, 0.0),
    t("gar", 10.0, 6.0, 0.1, 500.0, 30.0),
    t("general", 6.0, 2.0, 0.09, 200.0, 20.0),
];

/// Experience per hit (lbl_8011BB20) and per kill (lbl_8011BBA8); melee earns
/// twice the hit value, and its own kill values (lbl_8011BCB8).
const EXP_HIT: [u8; 34] = [
    1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 2, 1, 2, 3, 1, 2, 3, 1, 1, 2, 3, 3, 3, 1, 0, 2, 1, 1,
    2, 2,
];

pub fn exp_hit(etype: usize, melee: bool) -> f32 {
    let v = EXP_HIT.get(etype).copied().unwrap_or(1) as f32;
    if melee {
        v * 2.0
    } else {
        v
    }
}

pub fn exp_kill(etype: usize, melee: bool) -> f32 {
    match (etype, melee) {
        (27, false) => 15.0,
        (27, true) => 20.0,
        (29, false) | (33, false) => 30.0,
        (29, true) | (33, true) => 40.0,
        (31, false) => 2.0,
        (31, true) => 4.0,
        (32, _) => 300.0,
        (30, _) => 1.0,
        _ => exp_hit(etype, melee),
    }
}

/// Small monsters (height 3): DMG_LOW hits, wander when far.
pub fn is_small(etype: usize) -> bool {
    TYPES.get(etype).is_some_and(|t| t.height <= 4.0)
}

/// EnemyDescType: an item-info name's type, by desc.
pub fn type_of(name: &str) -> Option<usize> {
    TYPES
        .iter()
        .position(|t| !t.desc.is_empty() && t.desc.eq_ignore_ascii_case(name))
}

/// A level's monster table: each loaded type's ENMY subtype, and the type that
/// stands for each subtype (GetEnemyTypes' reverse table).
#[derive(Clone, Debug, Default)]
pub struct Roster {
    pub subtype: Vec<(usize, i32)>,
    reverse: [Option<usize>; 16],
}

impl Roster {
    pub fn new(loaded: &[(i32, i32)]) -> Roster {
        let mut r = Roster::default();
        for &(ty, sub) in loaded {
            if ty < 0 {
                continue;
            }
            let ty = ty as usize;
            // GetEnemySubtype's defaults when ENMY gives none.
            let sub = if sub != 0 {
                sub
            } else {
                match ty {
                    t if t < REGULAR && is_small(t) => 1,
                    t if t < REGULAR => 2,
                    29 | 32 | 33 => 5,
                    t if t >= 34 => 9,
                    // Death and IT stand for no generic name.
                    _ => 0,
                }
            };
            r.subtype.push((ty, sub));
            if (1..6).contains(&sub) {
                r.reverse[sub as usize] = Some(ty);
            }
        }
        r
    }

    /// GetEnemyType: RAT is the realm's small monster; GRU/KNI its medium one
    /// (large if it has none), or the aux model for placed strengths 4 and up.
    pub fn resolve(&self, ty: usize, level: i32) -> usize {
        match ty {
            RAT => self.reverse[1].unwrap_or(ty),
            GRUNT | KNIGHT => {
                if level >= 4 {
                    if let Some(t) = self.reverse[4] {
                        return t;
                    }
                }
                self.reverse[2].or(self.reverse[3]).unwrap_or(ty)
            }
            _ => ty,
        }
    }

    /// The MONSTERS directory a type loads from: `<desc>`, `<desc>aux` for the
    /// archer/bomber model, `<desc><n>` for Temple/Hell variants.
    pub fn dir(&self, ty: usize) -> String {
        let desc = TYPES.get(ty).map_or("gru", |t| t.desc).to_ascii_uppercase();
        match self.subtype.iter().find(|(t, _)| *t == ty).map(|s| s.1) {
            Some(4) => format!("{desc}AUX"),
            Some(s) if s > 10 => format!("{desc}{}", s - 10),
            _ => desc,
        }
    }
}

/// get_enemy_fight: the level's enemy damage x the type's fight, at full health;
/// two thirds below 2/3 health and a third below 1/3.
pub fn fight(etype: usize, hp: f32, ene_health: f32, ene_damage: f32) -> f32 {
    let t = &TYPES[etype.min(33)];
    let full = ene_health * t.health;
    let k = if hp > 0.667 * full || etype == DEATH {
        1.0
    } else if hp > 0.333 * full {
        0.667
    } else {
        0.333
    };
    ene_damage * t.fight * k
}

/// init_enemy: a type's starting health at a tier (1-3; higher counts as 2).
pub fn health(etype: usize, tier: i32, ene_health: f32) -> f32 {
    let t = &TYPES[etype.min(33)];
    let mut hp = t.health;
    if etype != DEATH {
        hp *= ene_health;
    }
    if etype < REGULAR {
        let l = if tier > 3 { 2 } else { tier.max(1) };
        hp = 0.333 * hp * l as f32;
    }
    hp
}

/// The Temple's and Hell's SPECIAL generators cycle through these.
pub const TEMPLE_SPECIAL: [usize; 4] = [16, 23, 14, 13];
pub const HELL_SPECIAL: [usize; 4] = [2, 24, 20, 25];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_and_damage() {
        assert!((health(GRUNT, 1, 1.0) - 9.99).abs() < 0.01);
        assert!((health(KNIGHT, 3, 1.0) - 45.95).abs() < 0.01);
        assert!((fight(GRUNT, 9.99, 1.0, 1.0) - 5.0).abs() < 0.05);
        assert!((fight(GRUNT, 29.97, 1.0, 1.0) - 15.0).abs() < 0.01);
    }

    #[test]
    fn mountain_remap() {
        // B1: troll (medium) and scorpion (small).
        let r = Roster::new(&[(1, 2), (0, 1)]);
        assert_eq!(r.resolve(GRUNT, 0), 1);
        assert_eq!(r.resolve(RAT, 0), 0);
        assert_eq!(r.dir(1), "TRO");
        // B4: demon (large), scorpion, troll aux: GRU generators make demons,
        // placed archers use the troll's aux model.
        let r = Roster::new(&[(2, 3), (0, 1), (1, 4)]);
        assert_eq!(r.resolve(GRUNT, 0), 2);
        assert_eq!(r.resolve(GRUNT, 4), 1);
        assert_eq!(r.dir(1), "TROAUX");
    }
}
