//! Power-ups (player.c PlayerAddPowerup / PlayerProcessPowerups): 11 slots of
//! {time left, type, amount, flags}. Timed ones count down in seconds; ones with
//! a negative time are counted in uses instead.

/// Power-up types (the item subtype).
pub const WEAPON: u8 = 5;
pub const ARMOR: u8 = 6;
pub const SPEED: u8 = 7;
pub const MAGIC: u8 = 8;
pub const SPECIAL: u8 = 9;

/// Weapon flags (DMG_TYPE): 1-4 are the element amulets.
pub const W_ELEMENT: u32 = 0xF;
pub const W_3WAY: u32 = 0x80000;
pub const W_SUPER: u32 = 0x100000;
pub const W_REFLECT: u32 = 0x200000;
pub const W_5WAY: u32 = 0x400000;
pub const W_HAMMER: u32 = 0x1000_0000;
pub const W_RAPID: u32 = 0x2000_0000;

/// Armour flags (ARMOR_TYPE).
pub const A_INVULN: u32 = 0x10000;
pub const A_GOLD: u32 = 0x100000;
pub const A_REFLECT: u32 = 0x20000;
pub const A_ANTIDEATH: u32 = 0x80000;
pub const A_FIRE: u32 = 0x200000;
pub const A_LIGHTNING: u32 = 0x400000;
pub const A_GAS: u32 = 0x2000;

/// Special flags (SPECIAL_TYPE).
pub const S_LEVITATE: u32 = 0x1;
pub const S_XRAY: u32 = 0x2;
pub const S_INVISIBLE: u32 = 0x4;
pub const S_STOPTIME: u32 = 0x8;
pub const S_BREATH: u32 = 0x70;
pub const S_PHOENIX: u32 = 0x80;
pub const S_GROWTH: u32 = 0x100;
pub const S_SHRINK: u32 = 0x200;
pub const S_TURBO: u32 = 0x80000;
pub const S_HANDOFDEATH: u32 = 0x200000;
pub const S_VAMPIRE: u32 = 0x400000;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    /// Seconds left; negative for a use-counted power-up; 0 when empty.
    pub time: f32,
    pub kind: u8,
    /// Uses left, or the speed / magic amount.
    pub amount: f32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Powerups {
    pub slots: [Slot; 11],
}

/// What the active power-ups add up to this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effects {
    /// ORed into every melee and shot (field_11C).
    pub weapon: u32,
    /// shield_flags.
    pub armor: u32,
    /// p->flags.
    pub special: u32,
    pub speed: f32,
    pub magic: f32,
}

impl Powerups {
    /// PlayerAddPowerup: `seconds` already scaled by the class's powerup_time
    /// (negative for use-counted ones, whose `amount` is the uses).
    pub fn add(&mut self, kind: u8, flags: u32, amount: f32, seconds: f32) {
        if let Some(s) = self
            .slots
            .iter_mut()
            .find(|s| s.time != 0.0 && s.kind == kind && s.flags == flags)
        {
            // The same power-up again: more uses, and half the new time.
            if amount > 0.0 {
                s.amount += amount;
            }
            if s.time >= 0.0 && seconds > 0.0 {
                s.time += 0.5 * seconds;
            } else if seconds < 0.0 {
                s.time = seconds;
            }
            return;
        }
        // Otherwise the free slot, or the one with the least time left.
        let pick = self
            .slots
            .iter()
            .enumerate()
            .min_by(|a, b| {
                let key = |s: &Slot| {
                    if s.time == 0.0 {
                        -1.0
                    } else if s.time < 0.0 {
                        f32::MAX
                    } else {
                        s.time
                    }
                };
                key(a.1).total_cmp(&key(b.1))
            })
            .map_or(0, |(i, _)| i);
        self.slots[pick] = Slot {
            time: seconds,
            kind,
            amount,
            flags,
        };
    }

    /// Count timed power-ups down (3x as fast while a boss is fighting).
    pub fn tick(&mut self, dt: f32) {
        for s in &mut self.slots {
            if s.time > 0.0 {
                s.time = (s.time - dt).max(0.0);
                if s.time == 0.0 {
                    *s = Slot::default();
                }
            }
        }
    }

    pub fn effects(&self) -> Effects {
        let mut e = Effects::default();
        for s in self.slots.iter().filter(|s| s.time != 0.0) {
            match s.kind {
                WEAPON => e.weapon |= s.flags,
                ARMOR => e.armor |= s.flags,
                SPEED => e.speed += s.amount,
                MAGIC => e.magic += s.amount,
                SPECIAL => e.special |= s.flags,
                _ => {}
            }
        }
        e
    }

    /// Spend one use of a use-counted power-up with `flags` of `kind`; false if
    /// there is none (player_get_powerup_state with dt 1).
    pub fn use_one(&mut self, kind: u8, flags: u32) -> bool {
        let Some(s) = self
            .slots
            .iter_mut()
            .find(|s| s.time < 0.0 && s.kind == kind && s.flags & flags != 0)
        else {
            return false;
        };
        s.amount -= 1.0;
        if s.amount <= 0.0 {
            *s = Slot::default();
        }
        true
    }

    /// Remove a power-up whose flags include `flags` (the turbo pick-up is
    /// consumed at once).
    pub fn remove(&mut self, kind: u8, flags: u32) {
        for s in &mut self.slots {
            if s.time != 0.0 && s.kind == kind && s.flags & flags != 0 {
                *s = Slot::default();
            }
        }
    }

    /// Dying clears every slot.
    pub fn clear(&mut self) {
        self.slots = Default::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stacking_adds_half_the_time() {
        let mut p = Powerups::default();
        p.add(WEAPON, W_3WAY, 0.0, 45.0);
        p.add(WEAPON, W_3WAY, 0.0, 45.0);
        assert_eq!(p.slots.iter().filter(|s| s.time != 0.0).count(), 1);
        assert!((p.slots[0].time - 67.5).abs() < 1e-3);
        p.tick(70.0);
        assert_eq!(p.effects().weapon, 0);
    }

    #[test]
    fn uses_run_out() {
        let mut p = Powerups::default();
        p.add(WEAPON, W_HAMMER, 3.0, -1.0);
        assert!(p.use_one(WEAPON, W_HAMMER));
        assert!(p.use_one(WEAPON, W_HAMMER));
        assert!(p.use_one(WEAPON, W_HAMMER));
        assert!(!p.use_one(WEAPON, W_HAMMER));
    }
}
