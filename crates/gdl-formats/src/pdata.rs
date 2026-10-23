//! PDATA/<CHR>.WAD: a hero class's effect/sound records (plyrdata.h
//! `plyr_sfx`, SFXX, 0x50 each), damage rows (`plyr_damage`, DAMG, 0x58 each)
//! and the `plyr_data` record (PDAT, 0x180) naming which rows the power moves,
//! turbo moves and combos fire. Same container as the CRITTER
//! WADs: `u32 dir, u32 count` then 16-byte entries `tag (reversed), offset,
//! count, count`; little-endian.

use crate::bin::Le;
use anyhow::{bail, Result};

/// plyr_sfx: an effect (by name, from the hero's SFX tree or a shared bank)
/// and a sound (by name), chained through `nextfxidx`. Particle records
/// (flags & 0x0F000000) name a texture and the node they hang from instead.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SfxRecord {
    pub flags: u32,
    pub nextfxidx: i32,
    pub fxdesc: String,
    pub snddesc: String,
    pub zmod: i16,
    pub alphamod: i16,
    pub offset: [f32; 3],
    /// Effect lifetime (s); 0 plays its animation once.
    pub maxlen: f32,
    pub radius: f32,
    pub scale: f32,
    pub color: u32,
}

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
    pub mintime: f32,
    pub maxtime: f32,
    pub angle: f32,
    /// Minimum dot product to the facing.
    pub arc: f32,
    pub pitch: f32,
    pub offset: [f32; 3],
    /// Damage; negative means -amount x the hero's melee damage.
    pub amount: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    /// SfxSetPhysics weight: a projectile row's fall per second, per second.
    pub weight: f32,
    /// `sfx` records: the effect started, the one spawned on a hit and the
    /// one morphed into after `maxtime` (-1 none).
    pub fxidx: i16,
    pub hitfxidx: i16,
    pub loopfxidx: i16,
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
    pub sfx: Vec<SfxRecord>,
    pub damage: Vec<DamageRow>,
    pub rows: MoveRows,
    /// Where a throw leaves the hand and where the power-A throw does, in the
    /// player's frame (x right, z ahead) from its effectpos.
    pub weapon_offset: [f32; 3],
    pub turboa_offset: [f32; 3],
    /// weapon_fx_offset / weapon_fx_scale: where an amulet's glow sits on the
    /// weapon hand, and its scale (x 0: unscaled), by level / 10.
    pub weapon_fx_offset: [[f32; 3]; 10],
    pub weapon_fx_scale: [[f32; 3]; 10],
    /// familiar_offset: where the level-30/80 familiar rides (PlayerProcessSkinFX).
    pub familiar_offset: [f32; 3],
    /// fam_proj_offset: where a familiar's or the Phoenix's shot starts, in the
    /// player's frame, scaled by the hero's node scale.
    pub fam_proj_offset: [f32; 3],
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
        if let Some((off, count)) = find("SFXX") {
            for i in 0..count {
                let o = off + i * 0x50;
                pd.sfx.push(SfxRecord {
                    flags: r.u32(o)?,
                    nextfxidx: r.i32(o + 4)?,
                    fxdesc: r.str(o + 0x10, 16)?,
                    snddesc: r.str(o + 0x20, 16)?,
                    zmod: r.i16(o + 0x30)?,
                    alphamod: r.i16(o + 0x32)?,
                    offset: r.vec3(o + 0x34)?,
                    maxlen: r.f32(o + 0x40)?,
                    radius: r.f32(o + 0x44)?,
                    scale: r.f32(o + 0x48)?,
                    color: r.u32(o + 0x4C)?,
                });
            }
        }
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
                    mintime: r.f32(o + 0x18)?,
                    maxtime: r.f32(o + 0x1C)?,
                    angle: r.f32(o + 0x20)?,
                    arc: r.f32(o + 0x24)?,
                    pitch: r.f32(o + 0x28)?,
                    offset: [r.f32(o + 0x2C)?, r.f32(o + 0x30)?, r.f32(o + 0x34)?],
                    amount: r.f32(o + 0x38)?,
                    speed_min: r.f32(o + 0x3C)?,
                    speed_max: r.f32(o + 0x40)?,
                    weight: r.f32(o + 0x44)?,
                    fxidx: r.i16(o + 0x48)?,
                    hitfxidx: r.i16(o + 0x4A)?,
                    loopfxidx: r.i16(o + 0x4C)?,
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
            for t in 0..10 {
                pd.weapon_fx_offset[t] = v(0x68 + 12 * t)?;
                pd.weapon_fx_scale[t] = v(0xE0 + 12 * t)?;
            }
            pd.familiar_offset = v(0x164)?;
            pd.fam_proj_offset = v(0x170)?;
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
        assert_eq!(pd.fam_proj_offset, [-1.0, 5.0, -1.0]);
    }

    #[test]
    fn warrior_sfx_records_match_the_spec() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/PDATA/WAR.WAD");
        let Ok(d) = std::fs::read(p) else { return };
        let pd = PlayerData::parse(&d).unwrap();
        assert_eq!(pd.sfx.len(), 15);
        // plyrdata.h: sfx[0] is the DRAGKEY_PART particle at R_WRIST.
        let s = &pd.sfx[0];
        assert_eq!(
            (s.fxdesc.as_str(), s.snddesc.as_str()),
            ("DRAGKEY_PART", "R_WRIST")
        );
        assert_eq!(
            (s.flags, s.nextfxidx, s.zmod, s.alphamod),
            (0x0200_0000, -1, 200, 150)
        );
        assert_eq!(
            (s.offset, s.maxlen, s.radius, s.scale),
            ([0.0, 0.0, 2.0], 0.5, 5.0, 2.0)
        );
        assert_eq!(s.color, 0x7FFF_FFFF);
        // Turbo A close: WAR_POWERA1 with S_WARTURBOA2; turbo C chains 7 -> 8.
        let close = &pd.damage[pd.rows.turbo_a_close as usize];
        let fx = &pd.sfx[close.fxidx as usize];
        assert_eq!(
            (fx.fxdesc.as_str(), fx.snddesc.as_str()),
            ("WAR_POWERA1", "S_WARTURBOA2")
        );
        assert_eq!(
            (close.hitfxidx, close.loopfxidx, close.maxtime),
            (-1, -1, 5.0)
        );
        assert_eq!(pd.sfx[7].nextfxidx, 8);
        // The turbo C wave: a type-2 row of WAR_PWRC3A morphing to WAR_PWRC3B.
        let wave = &pd.damage[7];
        assert_eq!((wave.kind, wave.fxidx, wave.loopfxidx), (2, 9, 10));
        assert_eq!(
            (wave.mintime, wave.maxtime, wave.speed_min),
            (6.0, 6.0, 30.0)
        );
        assert_eq!(pd.sfx[10].fxdesc, "WAR_PWRC3B");
    }

    #[test]
    fn wizard_sfx_chains() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/PDATA/WIZ.WAD");
        let Ok(d) = std::fs::read(p) else { return };
        let pd = PlayerData::parse(&d).unwrap();
        assert_eq!(pd.sfx.len(), 21);
        // plyrdata.h: WIZ sfx[0] chains nextfxidx 1 to sfx[1].
        assert_eq!(pd.sfx[0].nextfxidx, 1);
        assert_eq!(pd.sfx[1].snddesc, "L_WRIST");
        // The power B record shakes the camera (flag 2).
        let b = &pd.damage[pd.rows.turbo_b as usize];
        assert_eq!(pd.sfx[b.fxidx as usize].fxdesc, "WIZ_POWERB");
        assert_eq!(pd.sfx[b.fxidx as usize].flags, 6);
    }
}
