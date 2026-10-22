//! Power-ups (player.c PlayerAddPowerup / PlayerProcessPowerups): 11 slots of
//! {time left, type, amount, flags}. Timed ones count down in seconds; ones with
//! a negative time are counted in uses instead. A power-up picked up is kept
//! fresh until its player switches it on from the mini-inventory; only then
//! does it work and count down (powerup_state 1 fresh, 2 on, 3 off).

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
    /// powerup_state: FRESH, ON or OFF.
    pub state: u8,
}

pub const FRESH: u8 = 1;
pub const ON: u8 = 2;
pub const OFF: u8 = 3;

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
            state: FRESH,
        };
    }

    /// Count switched-on timed power-ups down (3x as fast while a boss is
    /// fighting).
    pub fn tick(&mut self, dt: f32) {
        for s in &mut self.slots {
            if s.time > 0.0 && s.state == ON {
                s.time = (s.time - dt).max(0.0);
                if s.time == 0.0 {
                    *s = Slot::default();
                }
            }
        }
    }

    pub fn effects(&self) -> Effects {
        let mut e = Effects::default();
        for s in self.slots.iter().filter(|s| s.time != 0.0 && s.state == ON) {
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

    /// The mini-inventory's previous / next slot (of the first ten, wrapping)
    /// holding a power-up, from `from` (-1 = none yet).
    pub fn prev_live(&self, from: i32) -> Option<usize> {
        let mut sel = if from < 0 { 9 } else { from - 1 };
        for _ in 0..10 {
            if sel < 0 {
                sel = 9;
            }
            if self.slots[sel as usize].time != 0.0 {
                return Some(sel as usize);
            }
            sel -= 1;
        }
        None
    }

    pub fn next_live(&self, from: i32) -> Option<usize> {
        let mut sel = from + 1;
        for _ in 0..10 {
            if sel > 9 {
                sel = 0;
            }
            if self.slots[sel as usize].time != 0.0 {
                return Some(sel as usize);
            }
            sel += 1;
        }
        None
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
        // Fresh: kept, not working, not counting down.
        p.tick(70.0);
        assert_eq!(p.effects().weapon, 0);
        assert!(p.slots[0].time > 0.0);
        p.slots[0].state = ON;
        assert_eq!(p.effects().weapon, W_3WAY);
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

/// mini_inv_items (player.c:409): the mini-inventory's name for a power-up,
/// by type and flag; the first entry whose flag is all set wins.
const LABELS: [(u8, u32, &str); 75] = [
    (9, 0x00000001, "Levitation"),
    (9, 0x00000002, "XRay"),
    (9, 0x00000004, "Invisible"),
    (9, 0x00000008, "StopTime"),
    (9, 0x00000010, "FireBreath"),
    (9, 0x00000020, "AcidBreath"),
    (9, 0x00000040, "LightningBreath"),
    (9, 0x00000080, "Phoenix"),
    (9, 0x00000100, "Growth"),
    (9, 0x00000200, "EnemyShrink"),
    (9, 0x00000400, "Pojo"),
    (9, 0x00001000, "BossHorns"),
    (9, 0x00002000, "BossMask"),
    (9, 0x00004000, "BossGauntR"),
    (9, 0x00008000, "BossGauntL"),
    (9, 0x00010000, "Speed"),
    (9, 0x00020000, "Heath"),
    (9, 0x00040000, "Dummy"),
    (9, 0x00080000, "Turbo"),
    (9, 0x00100000, "Mikey"),
    (9, 0x00200000, "HandOfDeath"),
    (9, 0x00400000, "HealthVamp"),
    (6, 0x00000001, "Fire Shield"),
    (6, 0x00000002, "Elec Shield"),
    (6, 0x00000004, "Resist Light"),
    (6, 0x00000008, "Gas Mask"),
    (6, 0x00000010, "Resist Magic"),
    (6, 0x00000100, "Immune Fire"),
    (6, 0x00000200, "Immune Elec"),
    (6, 0x00000400, "Immune Light"),
    (6, 0x00000800, "Immune Acid"),
    (6, 0x00001000, "Immune Magic"),
    (6, 0x00002000, "Immune Gas"),
    (6, 0x00010000, "Invulnerable"),
    (6, 0x00020000, "Reflective Armor"),
    (6, 0x00040000, "Knockback Armor"),
    (6, 0x00080000, "AntiDeath"),
    (6, 0x00100000, "Gold Invuln"),
    (6, 0x00200000, "Fire Armor"),
    (6, 0x00400000, "Elec Armor"),
    (6, 0x00800000, "Armor Protect"),
    (6, 0x01000000, "Armor Reflect"),
    (5, 0x00000010, "KnockBack"),
    (5, 0x00000020, "KnockDown"),
    (5, 0x00000040, "BlownAway"),
    (5, 0x00000080, "Stun"),
    (5, 0x00000100, "KnockOver"),
    (5, 0x00000200, "Magic"),
    (5, 0x00000400, "Explode"),
    (5, 0x00000800, "PoisonGas"),
    (5, 0x00001000, "DeathStun"),
    (5, 0x00002000, "Spike"),
    (5, 0x00004000, "Grabbed"),
    (5, 0x00008000, "Thrown"),
    (5, 0x00010000, "Whirlwind"),
    (5, 0x00020000, "Arrow"),
    (5, 0x00040000, "FireBall"),
    (5, 0x00080000, "3Way Shot"),
    (5, 0x00100000, "Super Shot"),
    (5, 0x00200000, "Weapon Reflect"),
    (5, 0x00400000, "5Way Shot"),
    (5, 0x00800000, "Weapon Heal"),
    (5, 0x01000000, "No Hit"),
    (5, 0x02000000, "Weapon Turbo"),
    (5, 0x04000000, "Weapon Sticky"),
    (5, 0x08000000, "Weapon Sticky"),
    (5, 0x10000000, "Hammer"),
    (5, 0x20000000, "RapidFire"),
    (5, 0x40000000, "Weapon Low"),
    (5, 0x00000000, "Weapon"),
    (7, 0x00000000, "Speed"),
    (8, 0x00000000, "Magic"),
    (9, 0x00000000, "Special"),
    (6, 0x00000000, "Armor"),
    (0, 0x00000000, "Cheat"),
];

/// The mini-inventory label for a slot (the last entry if none matches).
pub fn label(s: &Slot) -> &'static str {
    LABELS
        .iter()
        .find(|&&(k, f, _)| k == s.kind && s.flags & f == f)
        .map_or(LABELS[74].2, |l| l.2)
}

/// One player's mini-inventory box (tb_info): 0 closed, 2 sliding open, 1 open,
/// 3 sliding shut; the selected slot (-1 none) and the slide (0..128).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiniInv {
    pub state: u8,
    pub sel: i32,
    pub slide: i32,
}

impl Default for MiniInv {
    fn default() -> Self {
        MiniInv {
            state: 0,
            sel: -1,
            slide: 0,
        }
    }
}

/// D-pad edges for the mini-inventory.
#[derive(Clone, Copy, Debug, Default)]
pub struct DPad {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
}

impl MiniInv {
    /// mini_inventory_update for one tick; the sounds it plays.
    pub fn update(&mut self, pu: &mut Powerups, pad: DPad) -> Vec<&'static str> {
        let mut sfx = Vec::new();
        if self.state == 1 {
            let cur_gone = self.sel < 0 || pu.slots[self.sel as usize].time == 0.0;
            if cur_gone || pad.left {
                sfx.push("S_OPTMENUMOVHRZ");
                match pu.prev_live(self.sel) {
                    Some(s) => self.sel = s as i32,
                    None => {
                        self.state = 3;
                        self.slide = 0;
                    }
                }
            }
            if pad.right {
                sfx.push("S_OPTMENUMOVHRZ");
                match pu.next_live(self.sel) {
                    Some(s) => self.sel = s as i32,
                    None => {
                        self.sel = -1;
                        self.state = 3;
                        self.slide = 0;
                    }
                }
            }
            // Up switches the power-up on, and off again.
            if pad.up && self.sel >= 0 {
                sfx.push("S_OPTMENUSEL");
                let s = &mut pu.slots[self.sel as usize].state;
                *s = if *s != ON { ON } else { OFF };
            }
            if pad.down && self.state == 1 {
                sfx.push("S_OPTMENUMOVVRT");
                self.state = 3;
                self.slide = 0;
            }
        } else if pad.up {
            sfx.push("S_OPTMENUMOVVRT");
            if self.state == 0 {
                if self.sel == -1 {
                    if let Some(s) = pu.prev_live(-1) {
                        self.sel = s as i32;
                    }
                }
                if self.sel >= 0 {
                    self.state = 2;
                    self.slide = 0;
                }
            }
        }
        match self.state {
            2 | 3 => {
                if self.slide < 0x80 {
                    self.slide = (self.slide + 4).min(0x80);
                } else {
                    self.state = if self.state == 2 { 1 } else { 0 };
                }
            }
            _ => {}
        }
        sfx
    }
}

#[cfg(test)]
mod mini_inv_tests {
    use super::*;

    #[test]
    fn open_pick_and_switch_on() {
        let mut pu = Powerups::default();
        pu.add(WEAPON, W_3WAY, 0.0, 45.0);
        pu.add(SPECIAL, S_LEVITATE, 0.0, 30.0);
        let mut inv = MiniInv::default();
        let up = DPad {
            up: true,
            ..Default::default()
        };
        inv.update(&mut pu, up);
        assert_eq!((inv.state, inv.sel), (2, 1), "opens on the newest slot");
        for _ in 0..40 {
            inv.update(&mut pu, DPad::default());
        }
        assert_eq!(inv.state, 1);
        assert_eq!(label(&pu.slots[1]), "Levitation");
        inv.update(&mut pu, up);
        assert_eq!(pu.effects().special, S_LEVITATE);
        assert_eq!(pu.effects().weapon, 0);
        inv.update(
            &mut pu,
            DPad {
                left: true,
                ..Default::default()
            },
        );
        assert_eq!(inv.sel, 0);
        assert_eq!(label(&pu.slots[0]), "3Way Shot");
    }
}
