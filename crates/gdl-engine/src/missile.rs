//! Player thrown weapons: the launch maths of PlayerStartMissile / StartMissile
//! (combat.c) and ModifyPlayerDpos (pmotion.c). The flight itself (gravity,
//! tumble, walls) is the missile half of ProcessEffects, in game.rs.

use glam::Vec3;

/// MissileInfo (combat.c), the fields the player path reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MissileInfo {
    /// OR'd into the missile's damage type (SfxSetDamage).
    pub damage_type: u32,
    pub collision_radius: f32,
    pub hit_radius: f32,
    /// pyrvel (rad/s); all zero points the missile along its flight.
    pub angular_velocity: [f32; 3],
    /// SfxSetPhysics weight: the fall per second, per second.
    pub weight: f32,
    pub hit_effect: i32,
    pub wall_sound: i32,
}

const fn info(r: f32, spin: f32, weight: f32) -> MissileInfo {
    MissileInfo {
        damage_type: 0,
        collision_radius: r,
        hit_radius: 0.0,
        angular_velocity: [spin, 0.0, 0.0],
        weight,
        hit_effect: 0,
        wall_sound: 5,
    }
}

/// 6 pi: the axes and hammers tumble three turns a second.
const TUMBLE: f32 = 18.849_556;

/// PlayerMissileInfo (0x80118F68) by char_type, WAR .. JES.
pub const PLAYER_MISSILE_INFO: [MissileInfo; 8] = [
    info(1.0, TUMBLE, 12.0),
    info(1.0, TUMBLE, 8.0),
    info(1.2, 0.0, 8.0),
    info(0.7, 0.0, 8.0),
    info(1.0, TUMBLE, 20.0),
    info(1.0, TUMBLE, 8.0),
    info(1.2, 0.0, 8.0),
    MissileInfo {
        hit_radius: 2.0,
        hit_effect: 0x17,
        ..info(0.7, 0.0, 8.0)
    },
];

/// BallistaMissileInfo (0x8011A0E8): the super shot's arrow.
pub const BALLISTA_MISSILE_INFO: MissileInfo = MissileInfo {
    damage_type: 0x20,
    ..info(5.0, 0.0, 0.0)
};

/// BossElecMissileInfo / BossAcidMissileInfo (0x8011A118, 0x8011A148): the
/// boss gauntlets' bolts, lightning and acid, colrad 2, unweighted.
pub const BOSS_ELEC_MISSILE_INFO: MissileInfo = MissileInfo {
    damage_type: 2,
    ..info(2.0, 0.0, 0.0)
};
pub const BOSS_ACID_MISSILE_INFO: MissileInfo = MissileInfo {
    damage_type: 4,
    ..info(2.0, 0.0, 0.0)
};

/// lbl_8011A1A8: where Pojo's spit leaves, from its effectpos.
pub const POJO_AIM: [f32; 3] = [0.0, -0.5, -1.25];

/// DMG_TYPE bits PlayerStartMissile tests.
/// (effect.h) An explosion: items are reached 1.5 short of it.
pub const DMG_EXPLODE: u32 = 0x400;
pub const DMG_3WAY: u32 = 0x80000;
pub const DMG_SUPER: u32 = 0x100000;
pub const DMG_REFLECT: u32 = 0x200000;
pub const DMG_5WAY: u32 = 0x400000;
/// The power-A throw's big missile.
pub const DMG_TURBO: u32 = 0x2000000;

/// lbl_80111DE0 / lbl_80111DF4: cos and sin of each shot's turn (0, +-15 and
/// +-30 degrees).
pub const SPREAD_COS: [f32; 5] = [1.0, 0.966, 0.966, 0.866, 0.866];
pub const SPREAD_SIN: [f32; 5] = [0.0, 0.259, -0.259, 0.5, -0.5];

/// lbl_803463A0 / lbl_80346388: the launch sweep runs from 3 behind the hand to
/// 2 past it, where the missile starts.
pub const SWEEP_BACK: f32 = -3.0;
pub const SWEEP_AHEAD: f32 = 2.0;
/// lbl_803463C8 / lbl_803463D0: a big (DMG_TURBO) missile's radius and model.
pub const BIG_RADIUS: f32 = 1.8;
pub const BIG_SCALE: f32 = 1.8;
/// lbl_803463B8: missiles are x1.2 at level 99.
pub const LEVEL99_SCALE: f32 = 1.2;

impl MissileInfo {
    /// PlayerStartMissile's desc: the super shot's arrow, else the class's.
    pub fn for_shot(char_type: usize, damage_type: u32) -> MissileInfo {
        if damage_type & DMG_SUPER != 0 && damage_type & DMG_TURBO == 0 {
            BALLISTA_MISSILE_INFO
        } else {
            PLAYER_MISSILE_INFO[char_type % 8]
        }
    }

    /// PlayerStartMissile's desc with the hero's power-up flags: the left
    /// boss gauntlet's (p->flags 0x8000) and right's (0x4000) bolts first.
    pub fn for_player(char_type: usize, special: u32, damage_type: u32) -> MissileInfo {
        if special & 0x8000 != 0 {
            BOSS_ELEC_MISSILE_INFO
        } else if special & 0x4000 != 0 {
            BOSS_ACID_MISSILE_INFO
        } else {
            Self::for_shot(char_type, damage_type)
        }
    }

    /// StartMissile's colrad: x1.8 for a big missile.
    pub fn radius(&self, damage_type: u32) -> f32 {
        if damage_type & DMG_TURBO != 0 {
            self.collision_radius * BIG_RADIUS
        } else {
            self.collision_radius
        }
    }
}

/// How many missiles a throw fans into: 5 with DMG_5WAY, 3 with DMG_3WAY.
pub fn shot_count(damage_type: u32) -> usize {
    if damage_type & DMG_5WAY != 0 {
        5
    } else if damage_type & DMG_3WAY != 0 {
        3
    } else {
        1
    }
}

/// StartMissile's lifetime: 2 s for a fanned throw, else 3 s.
pub fn lifetime(damage_type: u32) -> f32 {
    if damage_type & (DMG_3WAY | DMG_5WAY) != 0 {
        2.0
    } else {
        3.0
    }
}

/// MulVecMat3 by the player matrix: x along its right, z along its facing.
pub fn rotate_offset(offset: [f32; 3], yaw: f32) -> Vec3 {
    let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let side = Vec3::new(fwd.z, 0.0, -fwd.x);
    side * offset[0] + Vec3::Y * offset[1] + fwd * offset[2]
}

/// pmotion.c's missile block: the charge handed to ModifyPlayerDpos for a
/// plain throw, from the time since the throw began (combo_fade).
pub fn throw_charge(since_swing: f32) -> f32 {
    let c = since_swing - 0.27;
    if c < 0.0 {
        0.0
    } else if c > 0.1 {
        0.1
    } else {
        c
    }
}

/// ModifyPlayerDpos: aim `dpos` (the unit direction to the target, or the
/// facing) against the floor slope along `from` (the facing), and return the
/// throw's reach (speedArg, 200 x charge + 15).
pub fn modify_dpos(
    from: Vec3,
    dpos: &mut Vec3,
    slope: f32,
    char_type: usize,
    damage_type: u32,
    has_target: bool,
    charge: f32,
) -> f32 {
    let dot = from.x * dpos.x + from.z * dpos.z;
    let mag = dpos.x.hypot(dpos.z);
    let mut slope = slope.min(0.707);
    if char_type == 4 {
        slope += 0.2;
    }
    if dot < 0.866 * mag || damage_type & DMG_SUPER != 0 {
        *dpos = Vec3::new(from.x, slope, from.z);
    } else if has_target {
        if dpos.y > 0.0 {
            dpos.y *= 1.2;
        }
        if slope.abs() > dpos.y.abs() {
            dpos.y = 0.5 * (dpos.y + slope);
        }
    } else {
        let s = (1.0 - slope * slope).sqrt();
        dpos.x *= s;
        dpos.z *= s;
        dpos.y = slope;
    }
    if dpos.y < 0.0 {
        dpos.y *= 0.5;
    }
    *dpos = dpos.normalize_or(from);
    if dpos.y > 0.866 {
        *dpos = from;
    }
    200.0 * charge + 15.0
}

/// PlayerStartMissile's speed clamp and aimed arc: the centre missile's
/// velocity. With a reach (`speed_arg` > 0, not a super shot) it is lobbed to
/// land `speed_arg` x the aim's flat length away and 0.5 below the launch,
/// falling at `weight`; else it flies straight along `dir`.
pub fn launch_velocity(
    dir: Vec3,
    missile_speed: f32,
    speed_arg: f32,
    weight: f32,
    damage_type: u32,
) -> Vec3 {
    let speed = missile_speed.clamp(1.0, 100.0);
    let inv = 1.0 / speed;
    let mut d = dir;
    if damage_type & DMG_SUPER == 0 && speed_arg > 0.0 {
        d *= speed_arg;
        let horiz = d.x.hypot(d.z);
        let hn = if horiz > 0.001 { 1.0 / horiz } else { 1.0 };
        d.x *= hn;
        d.z *= hn;
        d.y = inv * (0.5 * weight * (horiz * inv) + (d.y - 0.5) * (speed * hn));
    }
    d * speed
}

/// Shot `i` of a fan: `vel` turned by the spread table.
pub fn spread(vel: Vec3, i: usize) -> Vec3 {
    let (c, s) = (SPREAD_COS[i], SPREAD_SIN[i]);
    Vec3::new(vel.x * c + vel.z * s, vel.y, -vel.x * s + vel.z * c)
}

/// The model scale: StartMissile's x1.8 for a big missile, replaced by
/// PlayerStartMissile's ScaleFX of the throw's scale (x1.2 at level 99) when
/// that is not 1.
pub fn model_scale(scale_arg: f32, level: u32, damage_type: u32) -> f32 {
    let mut s = 1.0;
    if scale_arg > 1.0 {
        s *= scale_arg;
    }
    if level >= 99 {
        s *= LEVEL99_SCALE;
    }
    if s != 1.0 {
        s
    } else if damage_type & DMG_TURBO != 0 {
        BIG_SCALE
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dol() -> Option<gdl_formats::dol::Dol> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/main.dol");
        Some(gdl_formats::dol::Dol::new(std::fs::read(p).ok()?))
    }

    #[test]
    fn tables_match_the_dol() {
        let Some(d) = dol() else { return };
        let f = |a: u32| d.f32_at(a).unwrap();
        for (ct, m) in PLAYER_MISSILE_INFO.iter().enumerate() {
            let a = 0x8011_8F68 + 0x30 * ct as u32;
            assert_eq!(d.u32_at(a), Some(m.damage_type), "char {ct}");
            assert_eq!(f(a + 0x0C), m.collision_radius, "char {ct}");
            assert_eq!(f(a + 0x10), m.hit_radius, "char {ct}");
            assert_eq!(
                [f(a + 0x14), f(a + 0x18), f(a + 0x1C)],
                m.angular_velocity,
                "char {ct}"
            );
            assert_eq!(f(a + 0x20), m.weight, "char {ct}");
            assert_eq!(d.u32_at(a + 0x24), Some(m.hit_effect as u32));
            assert_eq!(d.u32_at(a + 0x2C), Some(m.wall_sound as u32));
        }
        let b = 0x8011_A0E8;
        assert_eq!(d.u32_at(b), Some(BALLISTA_MISSILE_INFO.damage_type));
        assert_eq!(f(b + 0x0C), BALLISTA_MISSILE_INFO.collision_radius);
        assert_eq!(f(b + 0x20), BALLISTA_MISSILE_INFO.weight);
        for i in 0..5 {
            assert_eq!(f(0x8011_1DE0 + 4 * i as u32), SPREAD_COS[i]);
            assert_eq!(f(0x8011_1DF4 + 4 * i as u32), SPREAD_SIN[i]);
        }
        assert_eq!(f(0x8034_63D0), BIG_SCALE);
    }

    #[test]
    fn a_throw_lands_its_reach_away() {
        // A warrior's flat throw (no target): lobbed to come down 0.5 below
        // the hand at speedArg = 200 x 0.1 + 15 = 35.
        let from = Vec3::Z;
        let mut d = from;
        let reach = modify_dpos(from, &mut d, 0.0, 0, 0, false, throw_charge(1.0));
        assert_eq!(reach, 35.0);
        assert_eq!(d, Vec3::Z);
        let w = PLAYER_MISSILE_INFO[0].weight;
        let v = launch_velocity(d, 40.0, reach, w, 0);
        assert!((v.z - 40.0).abs() < 1e-4 && v.x.abs() < 1e-6);
        let t = reach / v.z;
        let y = v.y * t - 0.5 * w * t * t;
        assert!((y + 0.5).abs() < 1e-3, "lands at {y}");
        // Thrown straight off the swing: the shortest reach.
        assert_eq!(throw_charge(0.2), 0.0);
        assert_eq!(200.0 * throw_charge(0.2) + 15.0, 15.0);
    }

    #[test]
    fn dwarf_throws_up_the_slope_bias() {
        let from = Vec3::Z;
        let mut d = from;
        modify_dpos(from, &mut d, 0.0, 4, 0, false, 0.0);
        assert!((d.y - 0.2).abs() < 1e-5 && (d.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn super_shot_flies_straight() {
        let v = launch_velocity(Vec3::Z, 250.0, 35.0, 0.0, DMG_SUPER);
        assert_eq!(v, Vec3::Z * 100.0);
        assert_eq!(MissileInfo::for_shot(0, DMG_SUPER), BALLISTA_MISSILE_INFO);
        assert_eq!(
            MissileInfo::for_player(0, 0x8000, DMG_SUPER),
            BOSS_ELEC_MISSILE_INFO
        );
        assert_eq!(
            MissileInfo::for_player(0, 0x4000, 0),
            BOSS_ACID_MISSILE_INFO
        );
        assert_eq!(
            MissileInfo::for_shot(0, DMG_SUPER | DMG_TURBO),
            PLAYER_MISSILE_INFO[0]
        );
    }

    #[test]
    fn fans_turn_by_the_table() {
        assert_eq!(shot_count(DMG_3WAY), 3);
        assert_eq!(shot_count(DMG_5WAY), 5);
        assert_eq!(shot_count(DMG_3WAY | DMG_5WAY), 5);
        assert_eq!(shot_count(0), 1);
        let v = spread(Vec3::new(0.0, 1.0, 10.0), 1);
        // +15 degrees about y keeps the fall.
        assert!((v.x.atan2(v.z).to_degrees() - 15.0).abs() < 0.1);
        assert_eq!(v.y, 1.0);
        assert_eq!(lifetime(DMG_3WAY), 2.0);
        assert_eq!(lifetime(0), 3.0);
    }

    #[test]
    fn power_a_missile_is_big() {
        let m = PLAYER_MISSILE_INFO[0];
        assert!((m.radius(DMG_TURBO) - 1.8).abs() < 1e-6);
        assert_eq!(model_scale(2.0, 10, DMG_TURBO), 2.0);
        assert_eq!(model_scale(1.0, 10, DMG_TURBO), 1.8);
        assert_eq!(model_scale(1.0, 10, 0), 1.0);
        assert!((model_scale(1.0, 99, 0) - 1.2).abs() < 1e-6);
    }
}
