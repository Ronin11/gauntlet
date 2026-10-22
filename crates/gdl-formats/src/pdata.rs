//! PDATA/<CHR>.WAD: a hero class's damage rows (plyrdata.h `plyr_damage`, DAMG,
//! 0x58 each) and the `plyr_data` record (PDAT, 0x180) naming which rows the
//! power moves, turbo moves and combos fire. Same container as the CRITTER
//! WADs: `u32 dir, u32 count` then 16-byte entries `tag (reversed), offset,
//! count, count`; little-endian.

use crate::bin::Le;
use anyhow::{bail, Result};

/// plyr_damage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DamageRow {
    /// 2 projectile, 3/4 effect at the player, 10 missile burst, 0/5 none.
    pub kind: i16,
    pub flags: i16,
    pub dmgtype: u32,
    pub hitrad: f32,
    pub radius: f32,
    pub minrad: f32,
    /// Re-hit delay (s).
    pub delay: f32,
    pub angle: f32,
    /// Minimum dot product to the facing.
    pub arc: f32,
    pub offset: [f32; 3],
    /// Damage; negative means -amount x the hero's melee damage.
    pub amount: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    /// SfxSetPhysics weight: a projectile row's fall per second, per second.
    pub weight: f32,
    pub next: i16,
    pub startframe: i16,
    pub endframe: i16,
    pub helpidx: i16,
}

/// The rows each move fires (indices into `damage`, -1 none).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MoveRows {
    pub turbo_a_close: i16,
    pub turbo_a_low: i16,
    pub turbo_a_step: i16,
    pub turbo_a_360: i16,
    pub turbo_a_throw: i16,
    pub turbo_b: i16,
    pub turbo_c1: i16,
    pub turbo_c2: i16,
    pub combo1: i16,
    pub combo2: i16,
    pub combohit: i16,
    pub victory: i16,
}

#[derive(Clone, Debug, Default)]
pub struct PlayerData {
    pub damage: Vec<DamageRow>,
    pub rows: MoveRows,
    /// Where a throw leaves the hand and where the power-A throw does, in the
    /// player's frame (x right, z ahead) from its effectpos.
    pub weapon_offset: [f32; 3],
    pub turboa_offset: [f32; 3],
}

impl PlayerData {
    pub fn parse(data: &[u8]) -> Result<PlayerData> {
        let r = Le::new(data);
        let dir = r.u32(0)? as usize;
        let n = r.u32(4)? as usize;
        if n > 16 {
            bail!("PDATA: {n} sections");
        }
        let mut secs = Vec::new();
        for i in 0..n {
            let e = dir + 16 * i;
            let tag: String = r.bytes(e, 4)?.iter().rev().map(|&c| c as char).collect();
            secs.push((tag, r.u32(e + 4)? as usize, r.u32(e + 8)? as usize));
        }
        let find = |t: &str| secs.iter().find(|s| s.0 == t).map(|s| (s.1, s.2));
        let mut pd = PlayerData::default();
        if let Some((off, count)) = find("DAMG") {
            for i in 0..count {
                let o = off + i * 0x58;
                pd.damage.push(DamageRow {
                    kind: r.i16(o)?,
                    flags: r.i16(o + 2)?,
                    dmgtype: r.u32(o + 4)?,
                    hitrad: r.f32(o + 8)?,
                    radius: r.f32(o + 0x0C)?,
                    minrad: r.f32(o + 0x10)?,
                    delay: r.f32(o + 0x14)?,
                    angle: r.f32(o + 0x20)?,
                    arc: r.f32(o + 0x24)?,
                    offset: [r.f32(o + 0x2C)?, r.f32(o + 0x30)?, r.f32(o + 0x34)?],
                    amount: r.f32(o + 0x38)?,
                    speed_min: r.f32(o + 0x3C)?,
                    speed_max: r.f32(o + 0x40)?,
                    weight: r.f32(o + 0x44)?,
                    next: r.i16(o + 0x4E)?,
                    startframe: r.i16(o + 0x50)?,
                    endframe: r.i16(o + 0x52)?,
                    helpidx: r.i16(o + 0x54)?,
                });
            }
        }
        if let Some((o, _)) = find("PDAT") {
            let s = |k: usize| r.i16(o + 0x0C + 2 * k);
            pd.rows = MoveRows {
                turbo_a_close: s(0)?,
                turbo_a_low: s(1)?,
                turbo_a_step: s(2)?,
                turbo_a_360: s(3)?,
                turbo_a_throw: s(4)?,
                turbo_b: s(5)?,
                turbo_c1: s(6)?,
                turbo_c2: s(7)?,
                combo1: s(8)?,
                combo2: s(9)?,
                combohit: s(10)?,
                victory: s(11)?,
            };
            let v = |a: usize| -> Result<[f32; 3]> {
                Ok([r.f32(o + a)?, r.f32(o + a + 4)?, r.f32(o + a + 8)?])
            };
            pd.weapon_offset = v(0x5C)?;
            pd.turboa_offset = v(0x158)?;
        }
        Ok(pd)
    }

    /// A row and the rows chained after it.
    pub fn chain(&self, first: i16) -> Vec<&DamageRow> {
        let mut out = Vec::new();
        let mut i = first;
        while i >= 0 && out.len() < 16 {
            let Some(row) = self.damage.get(i as usize) else {
                break;
            };
            out.push(row);
            i = row.next;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warrior_rows_match_the_spec() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/PDATA/WAR.WAD");
        let Ok(d) = std::fs::read(p) else { return };
        let pd = PlayerData::parse(&d).unwrap();
        let close = &pd.damage[pd.rows.turbo_a_close as usize];
        assert_eq!(
            (close.startframe, close.amount, close.radius),
            (6, -0.5, 5.0)
        );
        assert_eq!(close.dmgtype, 0x21);
        let b = &pd.damage[pd.rows.turbo_b as usize];
        assert_eq!((b.amount, b.radius), (50.0, 12.0));
        assert!(!pd.chain(pd.rows.turbo_c1).is_empty());
        assert_eq!(pd.weapon_offset, [-0.5, 0.5, 1.5]);
        assert_eq!(pd.turboa_offset, [0.0, 0.5, 1.5]);
    }
}
