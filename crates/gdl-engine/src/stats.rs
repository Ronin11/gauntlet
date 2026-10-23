//! Player attributes and experience (player.c PlayerUpdateAtts, player_scale_att,
//! LevelToExp / ExpToLevel, AddExp) with the PDATA class ranges.

/// PDATA PDAT per class id (WAR..HYE): fight, speed, armor, magic as (min, max),
/// and powerup_time (how long power-ups last for the class).
pub const CLASS_ATTS: [([(f32, f32); 4], f32); 16] = [
    (
        [(600., 999.), (350., 750.), (300., 700.), (100., 500.)],
        1.0,
    ),
    (
        [(300., 700.), (300., 700.), (450., 850.), (300., 700.)],
        1.1,
    ),
    (
        [(250., 650.), (350., 750.), (150., 550.), (600., 999.)],
        1.3,
    ),
    (
        [(300., 750.), (650., 999.), (250., 700.), (350., 800.)],
        1.1,
    ),
    (
        [(600., 999.), (300., 700.), (350., 750.), (100., 500.)],
        1.0,
    ),
    (
        [(350., 750.), (300., 700.), (450., 850.), (250., 650.)],
        1.1,
    ),
    (
        [(200., 600.), (400., 800.), (150., 550.), (600., 999.)],
        1.3,
    ),
    (
        [(200., 600.), (600., 999.), (200., 600.), (350., 750.)],
        1.1,
    ),
    (
        [(650., 999.), (400., 850.), (350., 800.), (150., 600.)],
        1.0,
    ),
    (
        [(350., 800.), (350., 800.), (500., 950.), (350., 800.)],
        1.1,
    ),
    (
        [(300., 750.), (400., 850.), (200., 650.), (650., 999.)],
        1.3,
    ),
    (
        [(300., 750.), (650., 999.), (250., 700.), (350., 800.)],
        1.1,
    ),
    (
        [(650., 999.), (350., 800.), (400., 850.), (150., 600.)],
        1.0,
    ),
    (
        [(400., 850.), (350., 800.), (500., 950.), (300., 750.)],
        1.1,
    ),
    (
        [(250., 700.), (450., 900.), (200., 650.), (650., 999.)],
        1.3,
    ),
    (
        [(250., 700.), (650., 999.), (250., 700.), (400., 850.)],
        1.1,
    ),
];

/// The four attributes (0..999).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Atts {
    pub fight: f32,
    pub speed: f32,
    pub armor: f32,
    pub magic: f32,
}

/// Combat values derived from the attributes (player_scale_att).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Derived {
    /// Melee damage per hit (5..20).
    pub damage: f32,
    /// Subtracted from each hit taken (0..5).
    pub armor: f32,
    /// Potion blast radius (8..32).
    pub magic_power: f32,
    /// Walk speed in units per second (5..12.5); running is 1.3x.
    pub speed: f32,
    /// Thrown weapon damage (5..20) and speed (20..60 u/s).
    pub missile_damage: f32,
    pub missile_speed: f32,
}

/// The select screen's class 16: playable Sumner (character 2 with
/// hidden_code player_sumner_desc).
pub const SUMNER: usize = 16;

/// PlayerUpdateAtts: each attribute is the class minimum plus 5 per level above
/// 1, up to the class maximum, plus any shop bonus, at most 999. Sumner skips
/// it (check_player_atts) and keeps player_get_from_save's 999s.
pub fn atts(class: usize, level: u32, bonus: Atts) -> Atts {
    if class == SUMNER {
        return Atts {
            fight: 999.0,
            speed: 999.0,
            armor: 999.0,
            magic: 999.0,
        };
    }
    let (r, _) = CLASS_ATTS[class.min(15)];
    let at = |(lo, hi): (f32, f32), b: f32| {
        ((lo + 5.0 * (level.max(1) - 1) as f32).min(hi) + b).min(999.0)
    };
    Atts {
        fight: at(r[0], bonus.fight),
        speed: at(r[1], bonus.speed),
        armor: at(r[2], bonus.armor),
        magic: at(r[3], bonus.magic),
    }
}

/// player_scale_att: `0.001 * att * (hi - lo) + lo`.
fn scale(att: f32, lo: f32, hi: f32) -> f32 {
    (0.001 * att * (hi - lo) + lo).clamp(lo, hi)
}

pub fn derived(class: usize, a: Atts) -> Derived {
    // Wizards and Sorceresses (and their secret counterparts) throw with magic.
    let missile_att = if matches!(class & 7, 2 | 6) {
        a.magic
    } else {
        a.fight
    };
    Derived {
        damage: scale(a.fight, 5.0, 20.0),
        armor: scale(a.armor, 0.0, 5.0),
        magic_power: scale(a.magic, 8.0, 32.0),
        speed: scale(a.speed, 5.0, 12.5),
        missile_damage: scale(missile_att, 5.0, 20.0),
        missile_speed: scale(missile_att, 20.0, 60.0),
    }
}

/// How long power-ups last for a class (PDAT powerup_time).
pub fn powerup_time(class: usize) -> f32 {
    CLASS_ATTS[class.min(15)].1
}

/// LevelToExp: experience needed for a level.
pub fn level_to_exp(level: u32) -> u32 {
    let lv = level.clamp(1, 99);
    if lv <= 60 {
        (lv - 1) * (30 * lv + 1000)
    } else {
        165_200 + (lv - 60) * 4600
    }
}

/// ExpToLevel: the highest level whose experience is reached.
pub fn exp_to_level(exp: u32) -> u32 {
    (1..=99)
        .rev()
        .find(|&lv| exp >= level_to_exp(lv))
        .unwrap_or(1)
}

/// AddExp's scaling (modes other than -2): the level's xpmul, falling off for
/// players above the level's intended `plevel`.
pub fn scale_exp(amount: f32, level: u32, xp_mul: f32, plevel: f32) -> u32 {
    let mut fac = xp_mul;
    if plevel > 0.0 && level as f32 > plevel {
        fac *= 1.0 / (0.1 * (level as f32 - plevel) + 1.0);
    }
    (amount * fac).max(0.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warrior_level_one_matches_the_spec() {
        let d = derived(0, atts(0, 1, Atts::default()));
        assert!((d.damage - 14.0).abs() < 0.01);
        assert!((d.armor - 1.5).abs() < 0.01);
        assert!((d.magic_power - 10.4).abs() < 0.01);
        assert!((d.speed - 7.625).abs() < 0.01);
        assert!((d.missile_speed - 44.0).abs() < 0.01);
    }

    #[test]
    fn experience_curve() {
        assert_eq!(level_to_exp(2), 1060);
        assert_eq!(level_to_exp(10), 11700);
        assert_eq!(level_to_exp(60), 165_200);
        assert_eq!(level_to_exp(99), 344_600);
        assert_eq!(exp_to_level(1059), 1);
        assert_eq!(exp_to_level(1060), 2);
    }
}
