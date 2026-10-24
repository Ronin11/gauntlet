//! The rules the simulation plays by, as a value instead of hard-coded
//! numbers: the classic preset is the GameCube original, and a remix can
//! change any of them without a rebuild. Every peer of a networked game
//! must run the same rules, so they count towards the game's fingerprint.
//!
//! Only tunables a remix actually needs move in here, one at a time, each
//! keeping its original value as the classic default; the rest of the
//! game's constants stay where they are until something wants to change
//! them.

use crate::game;

/// Which family of rules a game plays by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    /// The GameCube original.
    Classic,
    /// The modernised game: the same rules until a remix feature lands.
    Remix,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rules {
    pub preset: Preset,
    /// Turbo meter fill per second.
    pub turbo_fill: f32,
    /// How far a melee swing reaches from the target's surface.
    pub melee_reach: f32,
    /// The aim cone for auto-aim and melee: dot >= this (0.5 on Easy).
    pub aim_cone: f32,
    /// How far auto-aim looks for a target.
    pub target_range: f32,
    /// Seconds a stray thrown weapon flies before it is taken away.
    pub projectile_life: f32,
}

impl Rules {
    /// The original's values.
    pub fn classic() -> Rules {
        Rules {
            preset: Preset::Classic,
            turbo_fill: game::TURBO_FILL,
            melee_reach: game::MELEE_REACH,
            aim_cone: game::AIM_CONE,
            target_range: game::TARGET_RANGE,
            projectile_life: game::PROJECTILE_LIFE,
        }
    }

    /// The remix: the classic values until a remix feature changes one.
    pub fn remix() -> Rules {
        Rules {
            preset: Preset::Remix,
            ..Rules::classic()
        }
    }

    /// A preset by name (`classic`, `remix`), as on the command line.
    pub fn named(name: &str) -> Option<Rules> {
        match name.to_ascii_lowercase().as_str() {
            "classic" | "original" => Some(Rules::classic()),
            "remix" | "modern" => Some(Rules::remix()),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self.preset {
            Preset::Classic => "classic",
            Preset::Remix => "remix",
        }
    }

    /// FNV-1a over every value, the same on every build: peers compare it
    /// before playing together, and the game's fingerprint folds it in.
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut put = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        put(&[self.preset as u8]);
        for v in [
            self.turbo_fill,
            self.melee_reach,
            self.aim_cone,
            self.target_range,
            self.projectile_life,
        ] {
            put(&v.to_bits().to_le_bytes());
        }
        h
    }
}

impl Default for Rules {
    fn default() -> Rules {
        Rules::classic()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classic_is_the_default_and_remix_only_differs_by_name_for_now() {
        assert_eq!(Rules::default(), Rules::classic());
        let remix = Rules::remix();
        assert_eq!(remix.preset, Preset::Remix);
        assert_eq!(
            Rules {
                preset: Preset::Classic,
                ..remix.clone()
            },
            Rules::classic()
        );
        assert_ne!(
            remix.digest(),
            Rules::classic().digest(),
            "the preset is in the digest"
        );
        assert_eq!(Rules::named("REMIX"), Some(remix));
        assert_eq!(Rules::named("hard"), None);
        let mut tweaked = Rules::classic();
        tweaked.turbo_fill *= 2.0;
        assert_ne!(tweaked.digest(), Rules::classic().digest());
    }
}
