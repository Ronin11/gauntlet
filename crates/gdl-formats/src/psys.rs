//! Particle-system templates: the 0x138-byte `WORLDPSYS` record (atree.c
//! `AtreeWorldPsys`, byte-swapped by fn_80011DCC) that ANIM.PS2 atree lists
//! (v8+, `atreelist.psys`) and WORLDS.PS2 (header words 0x1C/0x1D) carry, and
//! the built-in preset table in main.dol (`psysPresetTable`, big-endian).
//! mb_particle.c's setWorldParms applies one to a live particle system.

use crate::bin::Le;
use anyhow::{bail, Result};

/// Size of one record.
pub const PSYS_RECORD_SIZE: usize = 0x138;

/// `WORLDPSYS.enables` (`used`): which fields setWorldParms applies.
pub mod enables {
    /// Inherit the preset named by `preset` first.
    pub const PRESET: u32 = 0x1;
    pub const MAX_PARTICLES: u32 = 0x2;
    pub const MAX_DIRECTIONS: u32 = 0x4;
    pub const MAX_POSITIONS: u32 = 0x8;
    pub const E_LIFEFADE: u32 = 0x10;
    pub const P_LIFEFADE: u32 = 0x20;
    pub const E_ANGLE: u32 = 0x40;
    pub const E_DIR: u32 = 0x80;
    pub const E_VOL: u32 = 0x100;
    pub const E_RATE: u32 = 0x200;
    pub const E_RATE_RAND: u32 = 0x400;
    pub const GRAVITY: u32 = 0x800;
    pub const DRAG: u32 = 0x1000;
    pub const SPEED: u32 = 0x2000;
    pub const TEXTURE: u32 = 0x4000;
    pub const TEXCNT: u32 = 0x8000;
    pub const RGB: u32 = 0x10000;
    pub const ALPHA: u32 = 0x20000;
    pub const WIDTH: u32 = 0x40000;
    pub const E_DELAY: u32 = 0x80000;
}

/// One particle-system template (`AtreeWorldPsys`, Xbox `WORLDPSYS`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PsysRecord {
    /// `pversion`: setWorldParms ignores records below 0x100.
    pub version: u32,
    /// Preset id inherited with [`enables::PRESET`] (-1 ends the preset table).
    pub preset: i16,
    /// The id character a world object's `PSYS<id>` name selects.
    pub id: u8,
    /// `flagval` / `flagmask`: render and emitter flags, applied where the mask is set.
    pub flags: u32,
    pub flag_mask: u32,
    /// [`enables`] bits.
    pub enables: u32,
    pub max_particles: i32,
    pub max_directions: i32,
    pub max_positions: i32,
    /// Emitter life and fade, seconds (negative: forever).
    pub e_lifefade: [f32; 2],
    /// Particle life and fade, seconds.
    pub p_lifefade: [f32; 2],
    /// Emission cone, degrees (< 1 a line, >= 359 or < 0 a sphere).
    pub e_angle: f32,
    pub tex_count: i32,
    pub tex_name: String,
    pub e_dir: [f32; 3],
    /// Emission box extents.
    pub e_vol: [f32; 3],
    /// Particles a second: life start/end, fade start/end.
    pub e_rate: [f32; 4],
    /// Rate jitter, percent.
    pub e_rate_rand: f32,
    pub gravity: f32,
    pub drag: f32,
    /// Units a second.
    pub speed: f32,
    /// 0xAARRGGBB at life start, life end, fade start, fade end.
    pub rgba: [u32; 4],
    pub width: [f32; 4],
    /// Seconds before emitting.
    pub e_delay: f32,
}

impl PsysRecord {
    /// A little-endian record (ANIM.PS2, WORLDS.PS2) at `o`.
    pub fn parse(r: Le, o: usize) -> Result<Self> {
        let b = r.bytes(o, PSYS_RECORD_SIZE)?;
        Ok(Self::decode(b, true))
    }

    /// A big-endian record (main.dol's preset table).
    pub fn parse_be(b: &[u8]) -> Result<Self> {
        if b.len() < PSYS_RECORD_SIZE {
            bail!("psys record: {} bytes", b.len());
        }
        Ok(Self::decode(b, false))
    }

    fn decode(b: &[u8], le: bool) -> Self {
        let u = |o: usize| {
            let w: [u8; 4] = b[o..o + 4].try_into().unwrap();
            if le {
                u32::from_le_bytes(w)
            } else {
                u32::from_be_bytes(w)
            }
        };
        let f = |o: usize| f32::from_bits(u(o));
        let v3 = |o: usize| [f(o), f(o + 4), f(o + 8)];
        let v4 = |o: usize| [f(o), f(o + 4), f(o + 8), f(o + 12)];
        let half = if le {
            u16::from_le_bytes([b[4], b[5]])
        } else {
            u16::from_be_bytes([b[4], b[5]])
        };
        let name = &b[0x40..0x60];
        let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
        PsysRecord {
            version: u(0),
            preset: half as i16,
            id: b[6],
            flags: u(0x08),
            flag_mask: u(0x0C),
            enables: u(0x10),
            max_particles: u(0x14) as i32,
            max_directions: u(0x18) as i32,
            max_positions: u(0x1C) as i32,
            e_lifefade: [f(0x20), f(0x24)],
            p_lifefade: [f(0x28), f(0x2C)],
            e_angle: f(0x38),
            tex_count: u(0x3C) as i32,
            tex_name: String::from_utf8_lossy(&name[..end]).into_owned(),
            e_dir: v3(0x60),
            e_vol: v3(0x6C),
            e_rate: v4(0x78),
            e_rate_rand: f(0x88),
            gravity: f(0x8C),
            drag: f(0x90),
            speed: f(0x94),
            rgba: [u(0x98), u(0x9C), u(0xA0), u(0xA4)],
            width: v4(0xA8),
            e_delay: f(0xB8),
        }
    }

    /// `count` records from `at`.
    pub fn parse_list(r: Le, at: usize, count: usize) -> Result<Vec<Self>> {
        (0..count)
            .map(|i| Self::parse(r, at + i * PSYS_RECORD_SIZE))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_byte_orders_agree() {
        let mut le = vec![0u8; PSYS_RECORD_SIZE];
        le[0..4].copy_from_slice(&0x101u32.to_le_bytes());
        le[4..6].copy_from_slice(&5u16.to_le_bytes());
        le[6] = b'A';
        le[0x10..0x14].copy_from_slice(&0x56be1u32.to_le_bytes());
        le[0x40..0x47].copy_from_slice(b"P_TORCH");
        le[0x94..0x98].copy_from_slice(&4.0f32.to_le_bytes());
        let a = PsysRecord::parse(Le::new(&le), 0).unwrap();
        let mut be = le.clone();
        for w in (0..PSYS_RECORD_SIZE).step_by(4) {
            if w != 4 && !(0x40..0x60).contains(&w) {
                be[w..w + 4].reverse();
            }
        }
        be[4..6].reverse();
        let b = PsysRecord::parse_be(&be).unwrap();
        assert_eq!(a, b);
        assert_eq!((a.version, a.preset, a.id), (0x101, 5, b'A'));
        assert_eq!(
            (a.enables, a.tex_name.as_str(), a.speed),
            (0x56be1, "P_TORCH", 4.0)
        );
    }
}
