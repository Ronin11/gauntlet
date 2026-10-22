//! `CRITTER/*.WAD`: the data-driven bosses, golems, generals and gargoyles
//! (critter.c `CritterLoadFile` / `CritterInitHeader`).
//!
//! The container is `u32 dir_offset, u32 sections, 0, 0`, then the payloads; the
//! directory holds `sections` 16-byte entries `{tag (byte-reversed), offset,
//! count, count}`. Records (sizes): SFXX 0x50, DAMG 0x50, DESC 0x30, ADDA 0x30,
//! NODE 0x50, MOVE 0x90, PTRN 0x50, TYPE 0x140. Field names follow the Xbox PDB
//! `crit_*` structs. Indices inside TYPE (moves, patterns, colnodes) point into
//! the file tables; move `nextidx` and pattern move lists are type-local.

use crate::bin::Le;
use anyhow::{bail, Context, Result};

/// `crit_vis`: when a move, pattern or the whole critter may target a player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vis {
    /// Target distance range (max <= 0: no cap).
    pub min: f32,
    pub max: f32,
    /// Yaw of the test cone off the critter's forward (radians).
    pub ang: f32,
    /// Minimum dot product with that direction (-1 = any).
    pub arc: f32,
    /// Difficulty gate (difficulty = 0.5 + 4.5 x lost health fraction).
    pub dif_min: f32,
    pub dif_max: f32,
    /// Max distance of the critter from its leash centre (<= 0: off).
    pub maxrad: f32,
    /// Max |dy| to the target (CalcTarget only).
    pub maxdy: f32,
}

/// TYPE (`crit_type`, 0x140).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CritType {
    /// Atree name suffix (desc.prefix + suffix); also the health meter's name.
    pub suffix: String,
    /// Child types: the parent atree node this part animates.
    pub rootnode: String,
    pub headnode: String,
    pub eyenode: String,
    pub noskinfxnode: String,
    pub descidx: i16,
    /// Table slot; -1 for child-only types.
    pub subtype: i16,
    pub flags: u32,
    /// maxheadyaw, maxeyeyaw, maxheadpitch, maxeyepitch, initheadpitch, initeyepitch.
    pub look: [f32; 6],
    /// Half height (vertical collide radius) and horizontal body radius.
    pub hht: f32,
    pub rad: f32,
    pub vis: Vis,
    /// Leash centre (y >= 999: the spawn point) and radius (<= 0: never moves).
    pub movecenter: [f32; 3],
    pub moverad: f32,
    pub y_offset: f32,
    pub y_attn: f32,
    pub fight: f32,
    pub armor: f32,
    /// Body collision centre (local).
    pub coll: [f32; 3],
    /// Max facing deviation from the spawn yaw (unless flags & 0x400).
    pub maxyaw: f32,
    /// Boss key offset from the spawn point.
    pub keydest: [f32; 3],
    pub coinarc: f32,
    /// ModifyDamage shield bits.
    pub resist: u32,
    pub health: f32,
    pub base_exp: f32,
    /// Boss wake distance (<= 0: any).
    pub activedist: f32,
    pub tgtdamage: f32,
    pub hit1fx: i16,
    pub hit2fx: i16,
    /// HealthMeterStart: pieces, x advance, inset left and right.
    pub meter: [i16; 4],
    /// In-world GMETER offset.
    pub meterdelta: [f32; 3],
    pub nmoves: i16,
    pub moveidx: i16,
    pub npatterns: i16,
    pub patternidx: i16,
    pub ncolnodes: i16,
    pub colnodeidx: i16,
    /// Next type in the child chain (-1: none) and the parent (-1: root).
    pub childtype: i16,
    pub parenttype: i16,
}

/// MOVE (`crit_move`, 0x90).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Move {
    pub kind: i32,
    pub flags: u32,
    pub priority: i32,
    pub name: String,
    pub anim: String,
    /// Node the move is attached to (damage origin); empty = root.
    pub colnode: String,
    pub actionframe: i32,
    pub actionframe2: i32,
    /// DAMG for event 1 and 2 (-1: none).
    pub damageidx: i16,
    pub damageidx2: i16,
    /// SPRAY repeat period, in frames.
    pub delay: f32,
    pub endframe: i16,
    pub endframe2: i16,
    /// Forced follow-up (type-local, -1: none).
    pub nextidx: i16,
    pub interrupt: i16,
    pub movefx: i16,
    pub fxframe: i16,
    pub movefx2: i16,
    pub fxframe2: i16,
    pub vis: Vis,
    /// Reuse delay (s), counted from the move's end.
    pub interval: f32,
    /// Translation speed (units/s x ene_speed).
    pub speed: f32,
    /// Max turn rate (rad/s).
    pub turn: f32,
    /// Seconds the move is held after its animation ends.
    pub holdtime: f32,
}

/// PTRN (`crit_pattern`, 0x50).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pattern {
    pub name: String,
    pub flags: i16,
    pub priority: i16,
    pub interval: f32,
    /// Type-local moves, -1 padded.
    pub moves: [i16; 8],
    pub vis: Vis,
}

/// DAMG (`crit_damage`, 0x50).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Damage {
    /// CRIT_DMG kind: 0 NODE, 1 FIREBALL, 2 ARC, 3 STOMP, 4 CONE, 5 SPOUT,
    /// 6 SPOUT2, 7 GRAB, 8 FOUNTAIN, 9 SPEWCOINS.
    pub kind: i16,
    pub flags: i16,
    /// C_DMG bits handed to damage_player (low nibble: element).
    pub dmgtype: u32,
    /// Projectile radius; cone width.
    pub hitrad: f32,
    /// Node/grab radius; cone length; blast radius.
    pub radius: f32,
    pub minrad: f32,
    pub angle: f32,
    pub arc: f32,
    pub pitch: f32,
    pub offset: [f32; 3],
    pub amount: f32,
    pub speed_min: f32,
    pub speed_max: f32,
    pub weight: f32,
    pub duration: f32,
    pub fx: i16,
    pub hitfx: i16,
    pub loopfx: i16,
    pub endfx: i16,
    /// FIREBALL random yaw spread (full width).
    pub dyaw: f32,
}

/// NODE (`crit_colnode`, 0x50): a hit node.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ColNode {
    pub node: String,
    /// 1 NOFX, 2 DESTROY, 4 DESTROY_CHILDREN, 8 MOVECOL.
    pub flags: i16,
    pub destroyidx: i16,
    pub zmod: i16,
    pub maxdist: f32,
    pub priority: f32,
    pub offset: [f32; 3],
    pub rad: f32,
    pub flashnode: String,
    pub damagemul: f32,
    /// Fraction of the spawn health this node can absorb.
    pub maxdamage: f32,
}

/// SFXX (`crit_sfx`, 0x50): an effect and/or sound.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sfx {
    pub flags: u32,
    pub nextfx: i32,
    pub fxdesc: String,
    /// Sound name, a printf format taking the level letter.
    pub snddesc: String,
    pub offset: [f32; 3],
    pub maxlen: f32,
    pub rate: f32,
    pub zmod: i16,
    pub alphamod: i16,
    pub color: u32,
    pub scale: f32,
}

/// DESC (`crit_desc`, 0x30).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desc {
    /// Model directory under MONSTERS.
    pub name: String,
    /// Atree name prefix.
    pub prefix: String,
    /// CRIT_ETYPE: 3 golem, 4 boss, 7 gargoyle, 8 general.
    pub etype: i16,
}

/// ADDA (`crit_addanim`, 0x30): an extra animated tree on a type.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AddAnim {
    pub addto: i16,
    pub flags: i16,
    pub desc: String,
    pub node: String,
    pub offset: [f32; 3],
}

/// A parsed critter WAD.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CritterWad {
    pub types: Vec<CritType>,
    pub descs: Vec<Desc>,
    pub addanims: Vec<AddAnim>,
    pub moves: Vec<Move>,
    pub patterns: Vec<Pattern>,
    pub colnodes: Vec<ColNode>,
    pub damage: Vec<Damage>,
    pub sfx: Vec<Sfx>,
}

fn vis(r: &Le, o: usize) -> Result<Vis> {
    Ok(Vis {
        min: r.f32(o)?,
        max: r.f32(o + 4)?,
        ang: r.f32(o + 8)?,
        arc: r.f32(o + 0xC)?,
        dif_min: r.f32(o + 0x10)?,
        dif_max: r.f32(o + 0x14)?,
        maxrad: r.f32(o + 0x18)?,
        maxdy: r.f32(o + 0x1C)?,
    })
}

impl CritterWad {
    pub fn parse(data: &[u8]) -> Result<CritterWad> {
        let r = Le::new(data);
        let dir = r.u32(0)? as usize;
        let n = r.u32(4)? as usize;
        if n > 64 {
            bail!("critter WAD: {n} sections");
        }
        let mut secs: Vec<(String, usize, usize)> = Vec::new();
        for i in 0..n {
            let e = dir + 16 * i;
            let tag: String = r.bytes(e, 4)?.iter().rev().map(|&c| c as char).collect();
            secs.push((tag, r.u32(e + 4)? as usize, r.u32(e + 8)? as usize));
        }
        let sec = |tag: &str, size: usize| -> Result<Vec<usize>> {
            let Some(&(_, off, count)) = secs.iter().find(|(t, _, _)| t == tag) else {
                return Ok(Vec::new());
            };
            if off + count * size > data.len() {
                bail!("critter WAD: {tag} past end");
            }
            Ok((0..count).map(|i| off + i * size).collect())
        };
        let mut w = CritterWad::default();
        for o in sec("TYPE", 0x140)? {
            w.types.push(CritType {
                suffix: r.str(o, 16)?,
                rootnode: r.str(o + 0x10, 16)?,
                headnode: r.str(o + 0x20, 16)?,
                eyenode: r.str(o + 0x30, 16)?,
                noskinfxnode: r.str(o + 0x40, 16)?,
                descidx: r.i16(o + 0x50)?,
                subtype: r.i16(o + 0x52)?,
                flags: r.u32(o + 0x5C)?,
                look: std::array::from_fn(|k| r.f32(o + 0x60 + 4 * k).unwrap_or(0.0)),
                hht: r.f32(o + 0x78)?,
                rad: r.f32(o + 0x7C)?,
                vis: vis(&r, o + 0x80)?,
                movecenter: r.vec3(o + 0xA0)?,
                moverad: r.f32(o + 0xAC)?,
                y_offset: r.f32(o + 0xB0)?,
                y_attn: r.f32(o + 0xB4)?,
                fight: r.f32(o + 0xB8)?,
                armor: r.f32(o + 0xBC)?,
                coll: r.vec3(o + 0xC0)?,
                maxyaw: r.f32(o + 0xCC)?,
                keydest: r.vec3(o + 0xD0)?,
                coinarc: r.f32(o + 0xDC)?,
                resist: r.u32(o + 0xE0)?,
                health: r.f32(o + 0xE4)?,
                base_exp: r.f32(o + 0xE8)?,
                activedist: r.f32(o + 0xEC)?,
                tgtdamage: r.f32(o + 0xF0)?,
                hit1fx: r.i16(o + 0xF4)?,
                hit2fx: r.i16(o + 0xF6)?,
                meter: std::array::from_fn(|k| r.i16(o + 0xF8 + 2 * k).unwrap_or(0)),
                meterdelta: r.vec3(o + 0x100)?,
                nmoves: r.i16(o + 0x110)?,
                moveidx: r.i16(o + 0x112)?,
                npatterns: r.i16(o + 0x114)?,
                patternidx: r.i16(o + 0x116)?,
                ncolnodes: r.i16(o + 0x118)?,
                colnodeidx: r.i16(o + 0x11A)?,
                childtype: r.i16(o + 0x11C)?,
                parenttype: r.i16(o + 0x11E)?,
            });
        }
        if w.types.is_empty() {
            bail!("Critter Header has no types");
        }
        for o in sec("DESC", 0x30)? {
            w.descs.push(Desc {
                name: r.str(o, 16)?,
                prefix: r.str(o + 0x10, 16)?,
                etype: r.i16(o + 0x20)?,
            });
        }
        for o in sec("ADDA", 0x30)? {
            w.addanims.push(AddAnim {
                addto: r.i16(o)?,
                flags: r.i16(o + 2)?,
                desc: r.str(o + 0x10, 8)?,
                node: r.str(o + 0x18, 8)?,
                offset: r.vec3(o + 0x20)?,
            });
        }
        for o in sec("MOVE", 0x90)? {
            w.moves.push(Move {
                kind: r.i32(o)?,
                flags: r.u32(o + 4)?,
                priority: r.i32(o + 8)?,
                name: r.str(o + 0x10, 16)?,
                anim: r.str(o + 0x20, 16)?,
                colnode: r.str(o + 0x30, 16)?,
                actionframe: r.i32(o + 0x40)?,
                actionframe2: r.i32(o + 0x44)?,
                damageidx: r.i16(o + 0x48)?,
                damageidx2: r.i16(o + 0x4A)?,
                delay: r.f32(o + 0x4C)?,
                endframe: r.i16(o + 0x50)?,
                endframe2: r.i16(o + 0x52)?,
                nextidx: r.i16(o + 0x54)?,
                interrupt: r.i16(o + 0x56)?,
                movefx: r.i16(o + 0x58)?,
                fxframe: r.i16(o + 0x5A)?,
                movefx2: r.i16(o + 0x5C)?,
                fxframe2: r.i16(o + 0x5E)?,
                vis: vis(&r, o + 0x60)?,
                interval: r.f32(o + 0x80)?,
                speed: r.f32(o + 0x84)?,
                turn: r.f32(o + 0x88)?,
                holdtime: r.f32(o + 0x8C)?,
            });
        }
        for o in sec("PTRN", 0x50)? {
            w.patterns.push(Pattern {
                name: r.str(o, 16)?,
                flags: r.i16(o + 0x10)?,
                priority: r.i16(o + 0x12)?,
                interval: r.f32(o + 0x14)?,
                moves: std::array::from_fn(|k| r.i16(o + 0x20 + 2 * k).unwrap_or(-1)),
                vis: vis(&r, o + 0x30)?,
            });
        }
        for o in sec("NODE", 0x50)? {
            w.colnodes.push(ColNode {
                node: r.str(o, 16)?,
                flags: r.i16(o + 0x10)?,
                destroyidx: r.i16(o + 0x12)?,
                zmod: r.i16(o + 0x16)?,
                maxdist: r.f32(o + 0x18)?,
                priority: r.f32(o + 0x1C)?,
                offset: r.vec3(o + 0x20)?,
                rad: r.f32(o + 0x2C)?,
                flashnode: r.str(o + 0x30, 16)?,
                damagemul: r.f32(o + 0x40)?,
                maxdamage: r.f32(o + 0x44)?,
            });
        }
        for o in sec("DAMG", 0x50)? {
            w.damage.push(Damage {
                kind: r.i16(o)?,
                flags: r.i16(o + 2)?,
                dmgtype: r.u32(o + 4)?,
                hitrad: r.f32(o + 8)?,
                radius: r.f32(o + 0xC)?,
                minrad: r.f32(o + 0x10)?,
                angle: r.f32(o + 0x14)?,
                arc: r.f32(o + 0x18)?,
                pitch: r.f32(o + 0x1C)?,
                offset: r.vec3(o + 0x20)?,
                amount: r.f32(o + 0x2C)?,
                speed_min: r.f32(o + 0x30)?,
                speed_max: r.f32(o + 0x34)?,
                weight: r.f32(o + 0x38)?,
                duration: r.f32(o + 0x3C)?,
                fx: r.i16(o + 0x40)?,
                hitfx: r.i16(o + 0x42)?,
                loopfx: r.i16(o + 0x44)?,
                endfx: r.i16(o + 0x46)?,
                dyaw: r.f32(o + 0x48)?,
            });
        }
        for o in sec("SFXX", 0x50)? {
            w.sfx.push(Sfx {
                flags: r.u32(o)?,
                nextfx: r.i32(o + 4)?,
                fxdesc: r.str(o + 0x10, 16)?,
                snddesc: r.str(o + 0x20, 16)?,
                offset: r.vec3(o + 0x30)?,
                maxlen: r.f32(o + 0x3C)?,
                rate: r.f32(o + 0x40)?,
                zmod: r.i16(o + 0x44)?,
                alphamod: r.i16(o + 0x46)?,
                color: r.u32(o + 0x48)?,
                scale: r.f32(o + 0x4C)?,
            });
        }
        w.validate().context("critter WAD indices")?;
        Ok(w)
    }

    /// Every index into the tables is in range.
    fn validate(&self) -> Result<()> {
        for (i, t) in self.types.iter().enumerate() {
            let range = |base: i16, n: i16, len: usize, what: &str| -> Result<()> {
                if n > 0 && (base < 0 || base as usize + n as usize > len) {
                    bail!("type {i}: {what} {base}+{n} of {len}");
                }
                Ok(())
            };
            range(t.moveidx, t.nmoves, self.moves.len(), "moves")?;
            range(t.patternidx, t.npatterns, self.patterns.len(), "patterns")?;
            range(t.colnodeidx, t.ncolnodes, self.colnodes.len(), "colnodes")?;
            if t.descidx < 0 || t.descidx as usize >= self.descs.len() {
                bail!("type {i}: desc {}", t.descidx);
            }
            for idx in [t.childtype, t.parenttype] {
                if idx >= 0 && idx as usize >= self.types.len() {
                    bail!("type {i}: linked type {idx}");
                }
            }
        }
        Ok(())
    }

    /// The moves, patterns and colnodes of type `t`.
    pub fn type_moves(&self, t: usize) -> &[Move] {
        let ty = &self.types[t];
        let b = ty.moveidx.max(0) as usize;
        &self.moves[b..b + ty.nmoves.max(0) as usize]
    }
    pub fn type_patterns(&self, t: usize) -> &[Pattern] {
        let ty = &self.types[t];
        let b = ty.patternidx.max(0) as usize;
        &self.patterns[b..b + ty.npatterns.max(0) as usize]
    }
    pub fn type_colnodes(&self, t: usize) -> &[ColNode] {
        let ty = &self.types[t];
        let b = ty.colnodeidx.max(0) as usize;
        &self.colnodes[b..b + ty.ncolnodes.max(0) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> Option<std::path::PathBuf> {
        let root = std::env::var_os("GDL_GAME_DATA").map_or_else(
            || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data"),
            std::path::PathBuf::from,
        );
        let d = root.join("files/Gauntlet/CRITTER");
        if !d.is_dir() {
            eprintln!("skipping: no {}", d.display());
            return None;
        }
        Some(d)
    }

    #[test]
    fn every_critter_wad_parses() {
        let Some(d) = dir() else { return };
        let mut n = 0;
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            let w = CritterWad::parse(&std::fs::read(&p).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e:#}", p.display()));
            assert!(!w.moves.is_empty(), "{}", p.display());
            for t in 0..w.types.len() {
                let moves = w.type_moves(t);
                for m in moves {
                    assert!(m.nextidx < moves.len() as i16, "{} {}", p.display(), m.name);
                    for d in [m.damageidx, m.damageidx2] {
                        assert!(d < w.damage.len() as i16, "{} {}", p.display(), m.name);
                    }
                }
                for pt in w.type_patterns(t) {
                    assert!(pt.moves.iter().all(|&m| m < moves.len() as i16));
                }
            }
            n += 1;
        }
        assert_eq!(n, 18);
    }

    #[test]
    fn dragon_and_chimera_match_the_spec() {
        let Some(d) = dir() else { return };
        let w = CritterWad::parse(&std::fs::read(d.join("DRAGON.WAD")).unwrap()).unwrap();
        let t = &w.types[0];
        assert_eq!(t.health, 2000.0);
        assert_eq!(t.armor, 1.0);
        assert_eq!(t.flags, 0x1E);
        assert_eq!(t.moverad, 3.0);
        assert_eq!(w.descs[0].etype, 4);
        let m = w.type_moves(0);
        let claw = m.iter().find(|m| m.name == "CLAWL").unwrap();
        assert_eq!(claw.kind, 0x80);
        assert_eq!(claw.actionframe, 30);
        assert_eq!(claw.interval, 6.0);
        let dmg = &w.damage[claw.damageidx as usize];
        assert_eq!(dmg.kind, 0);
        assert_eq!(dmg.amount, 15.0);
        assert_eq!(dmg.dmgtype & 0x20, 0x20);
        let c = CritterWad::parse(&std::fs::read(d.join("CHIMERA.WAD")).unwrap()).unwrap();
        assert_eq!(c.types.len(), 4);
        assert_eq!(c.types[0].childtype, 1);
        let names: Vec<&str> = c.types.iter().map(|t| t.suffix.as_str()).collect();
        assert_eq!(names, ["", "EAGLE", "LION", "SNAKE"]);
        assert_eq!(c.types[2].health, 1500.0);
        let g = CritterWad::parse(&std::fs::read(d.join("GARM.WAD")).unwrap()).unwrap();
        assert_eq!(g.type_colnodes(0).len(), 12);
        assert!(g.type_colnodes(0).iter().any(|n| n.flags & 2 != 0));
    }
}
