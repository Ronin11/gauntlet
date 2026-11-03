//! The player input the simulation reads each tick, and the control styles
//! that map a controller's buttons to it. The keyboard, gamepads and
//! key-event controllers that produce it live in the engine's input module.

use glam::Vec2;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInput {
    /// Left stick / WASD, x = right, y = up (screen relative), magnitude 0..1.
    pub stick: Vec2,
    /// A: attack (held and newly pressed).
    pub attack: bool,
    pub attack_pressed: bool,
    /// X: magic (use a potion).
    pub magic_pressed: bool,
    /// B held: turbo / block (they share a button on GameCube).
    pub turbo: bool,
    /// B newly pressed: back in menus, "Quit Game" when dead.
    pub b_pressed: bool,
    /// Y: slow attack (newly pressed, and held).
    pub slow_pressed: bool,
    pub slow: bool,
    /// X held.
    pub magic: bool,
    /// GameCube L: charge; R held: strafe; Z held: two-player combo.
    pub charge_pressed: bool,
    /// L held.
    pub charge: bool,
    pub strafe: bool,
    /// R newly pressed.
    pub strafe_pressed: bool,
    pub combo: bool,
    pub start_pressed: bool,
    /// The C-stick (right stick), x = right, y = up: Robotron style's
    /// firing direction (fireang / firemag).
    pub fire: Vec2,
    /// Start held (the reset combo).
    pub start: bool,
    /// The d-pad's share of `stick` (menus move with either; in play the d-pad
    /// runs the mini-inventory instead of walking).
    pub dpad: Vec2,
    /// D-pad presses: the mini-inventory (up opens / switches on, left and
    /// right choose, down closes).
    pub inv: crate::powerups::DPad,
}

/// controls.c rep_speed: ticks between auto-repeats while a direction is held.
const REP_SPEED: [u32; 13] = [30, 20, 10, 6, 3, 3, 3, 3, 2, 2, 2, 2, 1];

/// Directions with the game's auto-repeat edges (PlayerControls).
#[derive(Clone, Copy, Debug, Default)]
pub struct Repeat {
    held: (i32, i32),
    count: u32,
    index: usize,
}

impl Repeat {
    /// This tick's presses and repeats: (x, y) each -1, 0 or 1, y = 1 meaning up.
    pub fn edges(&mut self, input: &PlayerInput) -> (i32, i32) {
        let q = |v: f32| (v > 0.5) as i32 - (v < -0.5) as i32;
        let now = (q(input.stick.x), q(input.stick.y));
        if now == (0, 0) || now != self.held {
            let e = (
                if now.0 != self.held.0 { now.0 } else { 0 },
                if now.1 != self.held.1 { now.1 } else { 0 },
            );
            self.held = now;
            self.count = 0;
            self.index = 0;
            return e;
        }
        self.count += 1;
        if self.count >= REP_SPEED[self.index] {
            self.count -= REP_SPEED[self.index];
            self.index = (self.index + 1).min(REP_SPEED.len() - 1);
            return now;
        }
        (0, 0)
    }
}

/// One player's input for one tick, as it travels and is recorded: what the
/// simulation actually sees. Sticks have 255 positions, as a GameCube pad's do,
/// so two machines (or a replay) can't differ in a stick's last bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PackedInput {
    pub stick: [i8; 2],
    pub fire: [i8; 2],
    pub dpad: [i8; 2],
    pub buttons: u32,
}

fn pack_axis(v: f32) -> i8 {
    (v.clamp(-1.0, 1.0) * 127.0).round() as i8
}

fn pack_stick(v: Vec2) -> [i8; 2] {
    [pack_axis(v.x), pack_axis(v.y)]
}

fn unpack_stick(v: [i8; 2]) -> Vec2 {
    Vec2::new(v[0] as f32 / 127.0, v[1] as f32 / 127.0)
}

impl PlayerInput {
    pub fn pack(&self) -> PackedInput {
        let bits = [
            self.attack,
            self.attack_pressed,
            self.magic_pressed,
            self.turbo,
            self.b_pressed,
            self.slow_pressed,
            self.slow,
            self.magic,
            self.charge_pressed,
            self.charge,
            self.strafe,
            self.strafe_pressed,
            self.combo,
            self.start_pressed,
            self.start,
            self.inv.up,
            self.inv.down,
            self.inv.left,
            self.inv.right,
        ];
        let mut buttons = 0;
        for (i, on) in bits.into_iter().enumerate() {
            buttons |= (on as u32) << i;
        }
        PackedInput {
            stick: pack_stick(self.stick),
            fire: pack_stick(self.fire),
            dpad: pack_stick(self.dpad),
            buttons,
        }
    }
}

impl PackedInput {
    pub fn unpack(&self) -> PlayerInput {
        let on = |i: u32| self.buttons & (1 << i) != 0;
        PlayerInput {
            stick: unpack_stick(self.stick),
            attack: on(0),
            attack_pressed: on(1),
            magic_pressed: on(2),
            turbo: on(3),
            b_pressed: on(4),
            slow_pressed: on(5),
            slow: on(6),
            magic: on(7),
            charge_pressed: on(8),
            charge: on(9),
            strafe: on(10),
            strafe_pressed: on(11),
            combo: on(12),
            start_pressed: on(13),
            fire: unpack_stick(self.fire),
            start: on(14),
            dpad: unpack_stick(self.dpad),
            inv: crate::powerups::DPad {
                up: on(15),
                down: on(16),
                left: on(17),
                right: on(18),
            },
        }
    }
}

/// The pad's buttons as ReadControls' level bits (the PS2 report scePadRead
/// builds from the GameCube pad: A cross, B triangle, X circle, Y square, L
/// L1, R R1, Z R2).
pub mod pad {
    pub const L1: u32 = 0x0010_0000;
    pub const R1: u32 = 0x0040_0000;
    pub const R2: u32 = 0x0080_0000;
    pub const CIRCLE: u32 = 0x0100_0000;
    pub const CROSS: u32 = 0x0200_0000;
    pub const SQUARE: u32 = 0x0400_0000;
    pub const TRIANGLE: u32 = 0x0800_0000;
}

/// SpecialData (controls.c lbl_8011ACD0): for each logical button (bit
/// 8 + row: magic, quick, slow, turbo, defend, charge, strafe, magic shield,
/// throw magic, combo) the pad buttons that press it, two masks per control
/// scheme (0 Default, 1 Arcade, 2 Robotron, 3 unused by the menu).
pub const SPECIAL_DATA: [[u32; 8]; 10] = [
    [
        0x0100_0000,
        0,
        0x0100_0000,
        0,
        0x0800_0000,
        0,
        0x0800_0000,
        0,
    ],
    [
        0x0200_0000,
        0,
        0x0200_0000,
        0,
        0x0200_0000,
        0,
        0x0200_0000,
        0,
    ],
    [
        0x0400_0000,
        0,
        0x0800_0000,
        0,
        0x0010_0000,
        0,
        0x0010_0000,
        0,
    ],
    [
        0x0800_0000,
        0,
        0x0400_0000,
        0,
        0x0040_0000,
        0,
        0x0040_0000,
        0,
    ],
    [
        0x0800_0000,
        0,
        0x0400_0000,
        0,
        0x0040_0000,
        0,
        0x0040_0000,
        0,
    ],
    [
        0x0010_0000,
        0,
        0x0010_0000,
        0,
        0x0400_0000,
        0x0100_0000,
        0,
        0,
    ],
    [0x0040_0000, 0, 0x0040_0000, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
    [
        0x0080_0000,
        0,
        0x0080_0000,
        0,
        0x0080_0000,
        0,
        0x0080_0000,
        0,
    ],
];

/// Control scheme 2 (Robotron): the C-stick aims (PlayerControls' fireang).
pub const TWIN_STICK: usize = 2;

impl PlayerInput {
    /// The pad buttons held and newly pressed, as [`pad`] bits.
    pub fn pad_bits(&self) -> (u32, u32) {
        let mut held = 0;
        let mut edges = 0;
        for (h, e, b) in [
            (self.attack, self.attack_pressed, pad::CROSS),
            (self.turbo, self.b_pressed, pad::TRIANGLE),
            (self.magic, self.magic_pressed, pad::CIRCLE),
            (self.slow, self.slow_pressed, pad::SQUARE),
            (self.charge, self.charge_pressed, pad::L1),
            (self.strafe, self.strafe_pressed, pad::R1),
            (self.combo, false, pad::R2),
        ] {
            if h || e {
                held |= b;
            }
            if e {
                edges |= b;
            }
        }
        (held, edges)
    }

    /// ControlsUpdate: the logical buttons (bits 8..17) held and newly
    /// pressed under control scheme `scheme`.
    pub fn logical(&self, scheme: usize) -> (u32, u32) {
        let (held, edges) = self.pad_bits();
        let s = scheme.min(3) * 2;
        let mut levels = 0;
        let mut new = 0;
        for (t, row) in SPECIAL_DATA.iter().enumerate() {
            let mask = row[s] | row[s + 1];
            if held & mask != 0 {
                levels |= 1 << (t + 8);
            }
            if edges & mask != 0 {
                new |= 1 << (t + 8);
            }
        }
        (levels, new)
    }

    /// PlayerControls' fire stick for `scheme`: the C-stick past half way
    /// (firemag > 0.5) under the Robotron scheme, else none.
    pub fn fire_stick(&self, scheme: usize) -> Vec2 {
        if scheme == TWIN_STICK && self.fire.length() > 0.5 {
            self.fire.clamp_length_max(1.0)
        } else {
            Vec2::ZERO
        }
    }
}

/// Control-style names at 0x8011F048. `% 3` in DoOptions keeps "One Handed" out of reach.
pub const CONTROL_STYLES: [&str; 4] = ["Default", "Arcade", "Robotron", "One Handed"];

/// One player's control settings (PlayerControl[p]; controls.c defaults).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerPrefs {
    /// 0 Default, 1 Arcade, 2 Robotron (see [`CONTROL_STYLES`]).
    pub control_style: usize,
    pub rumble: bool,
    pub auto_aim: bool,
    pub auto_attack: bool,
}

impl Default for PlayerPrefs {
    fn default() -> Self {
        PlayerPrefs {
            control_style: 0,
            rumble: true,
            auto_aim: true,
            auto_attack: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paction::bits;

    /// SpecialData's three menu schemes, button by button.
    #[test]
    fn control_styles_map_the_buttons() {
        let b = PlayerInput {
            turbo: true,
            b_pressed: true,
            ..Default::default()
        };
        let y = PlayerInput {
            slow: true,
            slow_pressed: true,
            ..Default::default()
        };
        let l = PlayerInput {
            charge: true,
            ..Default::default()
        };
        let r = PlayerInput {
            strafe: true,
            ..Default::default()
        };
        let x = PlayerInput {
            magic: true,
            ..Default::default()
        };
        let held = |i: &PlayerInput, s| i.logical(s).0;
        // Default: B turbo / block, Y slow, L charge, R strafe, X magic.
        assert_eq!(held(&b, 0), bits::TURBO | bits::DEFEND);
        assert_eq!(b.logical(0).1, bits::TURBO | bits::DEFEND);
        assert_eq!(held(&y, 0), bits::ATK_SLOW);
        assert_eq!(held(&l, 0), bits::CHARGE);
        assert_eq!(held(&r, 0), bits::STRAFE);
        assert_eq!(held(&x, 0), bits::MAGIC);
        // Arcade swaps B and Y.
        assert_eq!(held(&b, 1), bits::ATK_SLOW);
        assert_eq!(held(&y, 1), bits::TURBO | bits::DEFEND);
        // Robotron: B magic, L slow, R turbo / block, X and Y charge, no strafe.
        assert_eq!(held(&b, 2), bits::MAGIC);
        assert_eq!(held(&l, 2), bits::ATK_SLOW);
        assert_eq!(held(&r, 2), bits::TURBO | bits::DEFEND);
        assert_eq!(held(&x, 2), bits::CHARGE);
        assert_eq!(held(&y, 2), bits::CHARGE);
        // A and Z are the same everywhere.
        let a = PlayerInput {
            attack: true,
            combo: true,
            ..Default::default()
        };
        for s in 0..3 {
            assert_eq!(held(&a, s), bits::ATK_QUICK | bits::COMBO);
        }
        // Only Robotron reads the C-stick, past half way.
        let c = PlayerInput {
            fire: Vec2::new(0.8, 0.0),
            ..Default::default()
        };
        assert_eq!(c.fire_stick(0), Vec2::ZERO);
        assert_eq!(c.fire_stick(TWIN_STICK), Vec2::new(0.8, 0.0));
        let weak = PlayerInput {
            fire: Vec2::new(0.4, 0.0),
            ..Default::default()
        };
        assert_eq!(weak.fire_stick(TWIN_STICK), Vec2::ZERO);
    }
}
