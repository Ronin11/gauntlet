//! `ANIM.PS2`: animation trees (skeletons) and their keyframed sequences.
//!
//! Characters are made of rigid parts; each animation node ("anode") names the
//! model part it drives. Keyframe evaluation follows the game's
//! `GetAnimAngXYZVal` (anim_play.c in the GameCube decompilation).

use crate::bin::Le;
use anyhow::{Context, Result};

pub const ANIM_SKELETAL: u16 = 1;
pub const ANIM_OBJECT: u16 = 2;
pub const ANIM_TEXTURE: u16 = 3;
pub const ANIM_PARTICLE: u16 = 4;

/// Channel bits in a sequence's flag word, in pose order (pitch, yaw, roll, x, y, z, sx, sy, sz).
const CHANNEL_BITS: [u16; 9] = [1, 2, 4, 0x10, 0x20, 0x40, 0x100, 0x200, 0x400];
const FLAG_COMPRESSED: u16 = 0x2000;
const FLAG_INITIAL_ONLY: u16 = 0x4000;

/// Size of one `TEXMOD` record (PDB `struct texmod`, 0x58 bytes).
const TEXMOD_SIZE: usize = 0x58;

/// A `TEXMOD` record (PDB misc.h `struct texmod`; atree.c / auxanim.c in the
/// decompilation): animates one texture of the model's texture bank.
///
/// What it does depends on [`TexMod::source`] (PDB `enum TEXMOD_TYPE`):
/// a texture index (>= 0) cycles the texture through `frames` consecutive
/// textures starting there; [`TexMod::EXTERNAL`] first looks `source_name` up
/// in every loaded bank; the scroll modes move the texture's UVs; the fades
/// ramp a node's alpha.
#[derive(Debug, Clone, PartialEq)]
pub struct TexMod {
    /// Owning atree, or -1 for a model-wide texmod run every frame by `DoTexMods`.
    pub atree: i16,
    /// Sequence index within `atree` (-2 for a texmod node's record).
    pub seq: i16,
    /// The animated texture's name.
    pub name: String,
    /// The first frame's texture name.
    pub source_name: String,
    /// Index of the animated texture in the bank (`texidx`; < 0 means look `name` up).
    pub tex: i32,
    /// First frame's texture index (`sourceidx`), or a negative mode.
    pub source: i32,
    /// Frame count (`nframes`); for scrolls the period, negative to scroll backwards.
    pub frames: i16,
    /// First animation frame of a sequence texmod (`startframe`).
    pub start_frame: i16,
    /// Game frames per step (`rate`); <= 0 steps every frame.
    pub rate: i32,
    /// Initial frame counter (`frame`), staggering copies of one animation.
    pub frame: i32,
}

impl TexMod {
    /// `TEXMOD_EXTERNAL`: the source texture lives in another bank (found by name).
    pub const EXTERNAL: i32 = -1;
    pub const SCROLL_U: i32 = -2;
    pub const SCROLL_V: i32 = -3;
    pub const FADE_IN: i32 = -4;
    pub const FADE_OUT: i32 = -5;
    pub const MIPBLEND: i32 = -6;

    /// Model-wide texmod (`flag == -1`), animated by `DoTexMods` on the game clock.
    pub fn is_global(&self) -> bool {
        self.atree == -1
    }

    fn parse(r: Le, o: usize) -> Result<Self> {
        Ok(TexMod {
            atree: r.i16(o)?,
            seq: r.i16(o + 2)?,
            name: r.str(o + 4, 32)?,
            source_name: r.str(o + 0x24, 32)?,
            tex: r.i32(o + 0x44)?,
            source: r.i32(o + 0x48)?,
            frames: r.i16(o + 0x4C)?,
            start_frame: r.i16(o + 0x4E)?,
            rate: r.i32(o + 0x50)?,
            frame: r.i32(o + 0x54)?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ANode {
    pub name: String,
    pub init_pos: [f32; 3],
    pub anim_type: u16,
    pub flags: u16,
    pub mb_flags: u32,
    pub parent: i32,
    seq_info_offset: u32,
    /// For object-animation nodes: index of this node's first entry in `ATree::obj_anims`
    /// (one entry per sequence follows).
    pub obj_anim_first: Option<usize>,
    /// For texture-animation nodes: the node's texmod record.
    pub texmod: Option<TexMod>,
}

/// Object ("vertex") animation: a run of whole meshes swapped per frame.
#[derive(Debug, Clone)]
pub struct ObjAnim {
    /// Name of the first frame's object, e.g. `GRU1WALK20F02`.
    pub name: String,
    pub frame_count: i16,
    pub start_frame: i16,
}

impl ObjAnim {
    /// Object-name prefix shared by every frame of this animation (`GRU1WALK20`).
    pub fn prefix(&self) -> &str {
        match self.name.rfind('F') {
            Some(i) if i > 0 => &self.name[..i],
            _ => &self.name,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Sequence {
    pub name: String,
    pub frame_count: i16,
    pub frame_rate: i16,
    pub repeat: bool,
    pub flags: u16,
    /// Texmods run with this sequence's frame (`DoSeqTexMods`).
    pub texmods: Vec<TexMod>,
}

/// Pose of one node: pitch/yaw/roll (radians), translation, scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub pyr: [f32; 3],
    pub xyz: [f32; 3],
    pub scale: [f32; 3],
    /// Bit 0x8000 of the sequence flags selects pitch-yaw-roll matrix order.
    pub pyr_order: bool,
}

impl Default for Pose {
    fn default() -> Self {
        Pose {
            pyr: [0.0; 3],
            xyz: [0.0; 3],
            scale: [1.0; 3],
            pyr_order: false,
        }
    }
}

/// Decoded keyframes for one node in one sequence.
#[derive(Debug, Clone)]
pub struct Track {
    pub flags: u16,
    /// (frame, [p, y, r, x, y, z, sx, sy, sz])
    pub keys: Vec<(u16, [f32; 9])>,
}

#[derive(Debug, Clone)]
pub struct ATree {
    pub name: String,
    pub prefix: String,
    pub nodes: Vec<ANode>,
    pub sequences: Vec<Sequence>,
    /// tracks[node][sequence]; None when the node has no data for that sequence.
    pub tracks: Vec<Vec<Option<Track>>>,
    pub obj_anims: Vec<ObjAnim>,
}

#[derive(Debug, Clone)]
pub struct Anim {
    pub version: u16,
    pub atrees: Vec<ATree>,
    /// The atreelist's texmod table: the global ones plus those referenced by
    /// sequences and texmod nodes.
    pub texmods: Vec<TexMod>,
}

fn wrap_angle(v: f32) -> f32 {
    if v > std::f32::consts::PI {
        v - std::f32::consts::TAU
    } else if v <= -std::f32::consts::PI {
        v + std::f32::consts::TAU
    } else {
        v
    }
}

impl Anim {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let r = Le::new(data);
        let count = r.u16(0)? as usize;
        let version = r.u16(2)?;
        let infos = r.u32(4)? as usize;
        // atreelist (PDB misc.h, 0x18): ntexmods @0x08, texmods @0x0C (an offset
        // from the start of the file, rebased by fn_8001267C in atree.c).
        let ntexmods = r.i32(8)?.max(0) as usize;
        let texmods_ptr = r.u32(12)? as usize;
        let mut texmods = Vec::with_capacity(ntexmods);
        if texmods_ptr != 0 {
            for i in 0..ntexmods {
                texmods.push(TexMod::parse(r, texmods_ptr + i * TEXMOD_SIZE)?);
            }
        }
        let tm = Texmods {
            list: &texmods,
            ptr: texmods_ptr,
        };
        let mut atrees = Vec::with_capacity(count);
        for i in 0..count {
            let o = infos + i * 36;
            let name = r.str(o, 32)?;
            let off = r.i32(o + 32)? as usize;
            atrees.push(
                parse_atree(r, name.clone(), off, &tm).with_context(|| format!("atree {name}"))?,
            );
        }
        Ok(Anim {
            version,
            atrees,
            texmods,
        })
    }

    /// The global texmods (`flag == -1`), animated every frame by `DoTexMods`.
    pub fn global_texmods(&self) -> impl Iterator<Item = &TexMod> {
        self.texmods.iter().filter(|t| t.is_global())
    }
}

/// The atreelist's texmod table, for resolving sequence and node references.
struct Texmods<'a> {
    list: &'a [TexMod],
    ptr: usize,
}

fn parse_atree(r: Le, name: String, h: usize, tm: &Texmods) -> Result<ATree> {
    let seq_ptr = h + r.u32(h)? as usize;
    let anim_hdr_ptr = r.u32(h + 4)? as usize;
    let obj_hdr_ptr = r.u32(h + 8)? as usize;
    let anode_ptr = h + r.u32(h + 12)? as usize;
    let anode_count = r.i32(h + 16)?.max(0) as usize;
    let seq_count = r.i32(h + 20)?.max(0) as usize;
    let prefix = r.str(h + 24, 30)?;

    let mut sequences = Vec::with_capacity(seq_count);
    for i in 0..seq_count {
        let o = seq_ptr + i * 48;
        sequences.push(Sequence {
            name: r.str(o, 32)?,
            frame_count: r.i16(o + 32)?,
            frame_rate: r.i16(o + 34)?,
            repeat: r.u16(o + 36)? != 0,
            flags: r.u16(o + 42)?,
            // ntexmods @0x28 and a texmod *index* @0x2C (InitAtreeSeqs).
            texmods: {
                let n = r.i16(o + 40)?.max(0) as usize;
                let first = r.i32(o + 44)?.max(0) as usize;
                tm.list.get(first..first + n).unwrap_or_default().to_vec()
            },
        });
    }

    let mut nodes = Vec::with_capacity(anode_count);
    for i in 0..anode_count {
        let o = anode_ptr + i * 60;
        nodes.push(ANode {
            name: r.str(o, 32)?,
            init_pos: r.vec3(o + 32)?,
            anim_type: r.u16(o + 44)?,
            flags: r.u16(o + 46)?,
            mb_flags: r.u32(o + 48)?,
            seq_info_offset: r.u32(o + 52)?,
            parent: r.i32(o + 56)?,
            obj_anim_first: None,
            texmod: None,
        });
    }

    // Texture-animation nodes: the node's data offset, relative to the
    // sequence table (AtreeNodeInit's type-3 base), points at its record in the
    // atreelist's texmod table.
    for n in nodes.iter_mut().filter(|n| n.anim_type == ANIM_TEXTURE) {
        let at = seq_ptr as i64 + n.seq_info_offset as i32 as i64 - tm.ptr as i64;
        if tm.ptr != 0 && at >= 0 && at as usize % TEXMOD_SIZE == 0 {
            n.texmod = tm.list.get(at as usize / TEXMOD_SIZE).cloned();
        }
    }

    // Object animation header: { entries pointer, count }, pointers relative to itself.
    let mut obj_anims = Vec::new();
    if obj_hdr_ptr != 0 && h + obj_hdr_ptr + 8 <= r.data.len() {
        let oh = h + obj_hdr_ptr;
        let ptr = r.u32(oh)? as usize;
        let count = r.u32(oh + 4)? as usize;
        if count < 10_000 {
            for i in 0..count {
                let o = oh + ptr + i * 40;
                obj_anims.push(ObjAnim {
                    name: r.str(o, 32)?,
                    frame_count: r.i16(o + 36)?,
                    start_frame: r.i16(o + 38)?,
                });
            }
            for n in nodes.iter_mut() {
                if n.anim_type == ANIM_OBJECT && n.seq_info_offset as usize >= ptr {
                    n.obj_anim_first = Some((n.seq_info_offset as usize - ptr) / 40);
                }
            }
        }
    }

    // Skeletal animation header; its pointers (and the nodes' sequence-info
    // offsets) are relative to the animation header itself.
    let mut tracks = vec![vec![None; seq_count]; anode_count];
    if anim_hdr_ptr != 0 {
        let ah = h + anim_hdr_ptr;
        let table = |p: u32| -> Result<Option<Vec<f32>>> {
            if p == 0 {
                return Ok(None);
            }
            (0..256)
                .map(|i| r.f32(ah + p as usize + i * 4))
                .collect::<Result<Vec<_>>>()
                .map(Some)
        };
        let comp_ang = table(r.u32(ah)?)?;
        let comp_pos = table(r.u32(ah + 4)?)?;
        let comp_scale = table(r.u32(ah + 8)?)?;
        let blocks = ah + r.u32(ah + 12)? as usize;
        let tables = [
            comp_ang.as_deref(),
            comp_pos.as_deref(),
            comp_scale.as_deref(),
        ];

        for (ni, node) in nodes.iter().enumerate() {
            if node.anim_type != ANIM_SKELETAL {
                continue;
            }
            let infos = ah + node.seq_info_offset as usize;
            for (si, seq) in sequences.iter().enumerate() {
                let o = infos + si * 8;
                let flags = r.u16(o)?;
                let size = r.u16(o + 2)? as usize;
                let data_off = r.u32(o + 4)? as usize;
                if flags & 0xFFF == 0 {
                    continue;
                }
                let t = decode_track(
                    r,
                    blocks + data_off,
                    flags,
                    size,
                    seq.frame_count.max(1) as usize,
                    tables,
                )
                .with_context(|| format!("node {} seq {}", node.name, seq.name))?;
                tracks[ni][si] = Some(t);
            }
        }
    }

    Ok(ATree {
        name,
        prefix,
        nodes,
        sequences,
        tracks,
        obj_anims,
    })
}

fn decode_track(
    r: Le,
    at: usize,
    flags: u16,
    size: usize,
    numframes: usize,
    tables: [Option<&[f32]>; 3],
) -> Result<Track> {
    let read_abs = |o: usize, dflt_scale: bool| -> Result<[f32; 9]> {
        let mut v = [0.0f32; 9];
        let mut n = 0;
        for (c, bit) in CHANNEL_BITS.iter().enumerate() {
            if flags & bit != 0 {
                v[c] = r.f32(o + n * 4)?;
                n += 1;
            } else if c >= 6 && dflt_scale {
                v[c] = 1.0;
            }
        }
        Ok(v)
    };

    if flags & FLAG_INITIAL_ONLY != 0 {
        return Ok(Track {
            flags,
            keys: vec![(0, read_abs(at, true)?)],
        });
    }

    let bit_words = numframes.div_ceil(32);
    let bits: Vec<u32> = (0..bit_words)
        .map(|i| r.u32(at + i * 4))
        .collect::<Result<_>>()?;
    let has_key = |f: usize| bits.get(f / 32).is_some_and(|w| w & (1 << (f % 32)) != 0);
    let data = at + bit_words * 4;

    let mut keys = vec![(0u16, read_abs(data, true)?)];
    let compressed = flags & FLAG_COMPRESSED != 0;
    let mut keynum = 0usize;
    let mut cur = keys[0].1;
    for f in 1..numframes {
        if !has_key(f) {
            continue;
        }
        if compressed {
            // Records of `size` byte indices into the delta tables, after the initial floats.
            let rec = data + size * 4 + keynum * size;
            let mut n = 0;
            for (c, bit) in CHANNEL_BITS.iter().enumerate() {
                if flags & bit != 0 {
                    let idx = r.u8(rec + n)? as usize;
                    let table = tables[c / 3].context("compressed track without delta table")?;
                    cur[c] += table[idx];
                    n += 1;
                }
            }
            keynum += 1;
        } else {
            keynum += 1;
            cur = read_abs(data + keynum * size * 4, true)?;
        }
        keys.push((f as u16, cur));
    }
    Ok(Track { flags, keys })
}

impl Track {
    /// Evaluate at a (fractional) frame, interpolating like the game does.
    pub fn sample(&self, frame: f32) -> Pose {
        let pyr_order = self.flags & 0x8000 != 0;
        let mk = |v: &[f32; 9]| Pose {
            pyr: [wrap_angle(v[0]), wrap_angle(v[1]), wrap_angle(v[2])],
            xyz: [v[3], v[4], v[5]],
            scale: [v[6], v[7], v[8]],
            pyr_order,
        };
        let i = self.keys.partition_point(|(f, _)| (*f as f32) <= frame);
        if i == 0 {
            return mk(&self.keys[0].1);
        }
        let (f0, a) = &self.keys[i - 1];
        let Some((f1, b)) = self.keys.get(i) else {
            return mk(a);
        };
        let t = (frame - *f0 as f32) / (*f1 - *f0) as f32;
        let mut v = [0.0f32; 9];
        for c in 0..9 {
            let d = b[c] - a[c];
            v[c] = if c < 3 && d.abs() >= std::f32::consts::FRAC_PI_2 {
                a[c]
            } else {
                a[c] + d * t
            };
        }
        mk(&v)
    }
}

impl ATree {
    pub fn sequence_index(&self, name: &str) -> Option<usize> {
        self.sequences
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Pose of every node at `frame` of sequence `seq` (identity for nodes without data).
    pub fn sample(&self, seq: usize, frame: f32) -> Vec<Pose> {
        self.tracks
            .iter()
            .map(|t| {
                t.get(seq)
                    .and_then(|t| t.as_ref())
                    .map(|t| t.sample(frame))
                    .unwrap_or_default()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(rel: &str) -> Option<Vec<u8>> {
        let root = std::env::var_os("GDL_GAME_DATA").map_or_else(
            || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data"),
            std::path::PathBuf::from,
        );
        let p = root.join("files/Gauntlet").join(rel);
        match std::fs::read(&p) {
            Ok(d) => Some(d),
            Err(_) => {
                eprintln!("skipping: no {}", p.display());
                None
            }
        }
    }

    #[test]
    fn level_texmods() {
        let Some(d) = read("LEVELS/levelB6/ANIM.PS2") else {
            return;
        };
        let a = Anim::parse(&d).unwrap();
        assert!(a.atrees.is_empty());
        let rows: Vec<(&str, &str, i32, i32, i16, i32, i32)> = a
            .texmods
            .iter()
            .map(|t| {
                assert!(t.is_global());
                (
                    t.name.as_str(),
                    t.source_name.as_str(),
                    t.tex,
                    t.source,
                    t.frames,
                    t.rate,
                    t.frame,
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("TORCHA", "TORCH00", 42, TexMod::EXTERNAL, 15, 2, 0),
                ("TORCHB", "TORCH00", 43, TexMod::EXTERNAL, 15, 2, 5),
                ("TORCHC", "TORCH00", 44, TexMod::EXTERNAL, 15, 2, 10),
                ("LAVA", "LAVA00", 6, 7, 30, 4, 0),
                ("SKY", "", 41, TexMod::SCROLL_U, 1250, 0, 0),
            ]
        );
    }

    #[test]
    fn sequence_and_node_texmods() {
        let Some(d) = read("ITEMS/levelC5/ANIM.PS2") else {
            return;
        };
        let a = Anim::parse(&d).unwrap();
        assert_eq!(a.texmods.len(), 21);
        assert_eq!(a.global_texmods().count(), 6);
        // COIN_GOLD's ACTIVE sequence flips NEL_COIN through 29 frames.
        let coin = a.atrees.iter().find(|t| t.name == "COIN_GOLD").unwrap();
        let seq = &coin.sequences[0];
        assert_eq!(seq.texmods.len(), 1);
        assert_eq!(seq.texmods[0].name, "NEL_COIN");
        assert_eq!((seq.texmods[0].source, seq.texmods[0].frames), (166, 29));
        assert_eq!(seq.texmods[0].atree, 3);
        // Texmod nodes: TEXMOD<n>_<start>_<frames>_... names match their records.
        let prj = a.atrees.iter().find(|t| t.name == "LEGENDPRJ").unwrap();
        let nodes: Vec<&ANode> = prj
            .nodes
            .iter()
            .filter(|n| n.anim_type == ANIM_TEXTURE)
            .collect();
        assert_eq!(nodes.len(), 11);
        assert!(nodes.iter().all(|n| n.texmod.is_some()));
        let t = nodes[0].texmod.as_ref().unwrap();
        assert_eq!(nodes[0].name, "TEXMOD0_0_4_16_LEGGENIETEX");
        assert_eq!(
            (t.source_name.as_str(), t.frames, t.seq),
            ("LEGGENIETEX16", 4, -2)
        );
        let t = nodes[6].texmod.as_ref().unwrap();
        assert_eq!(nodes[6].name, "TEXMOD6_4_16_LEGGENIETEX");
        assert_eq!((t.start_frame, t.frames), (4, 16));
    }

    #[test]
    fn every_texmod_node_resolves() {
        for rel in [
            "WEAPONS/ANIM.PS2",
            "ITEMS/levelL/ANIM.PS2",
            "ITEMS/MONSTERS/PBOSS/ANIM.PS2",
        ] {
            let Some(d) = read(rel) else { return };
            let a = Anim::parse(&d).unwrap();
            let mut n = 0;
            for t in &a.atrees {
                for node in t.nodes.iter().filter(|n| n.anim_type == ANIM_TEXTURE) {
                    let tm = node.texmod.as_ref();
                    assert!(tm.is_some(), "{rel} {} node {}", t.name, node.name);
                    assert_eq!(tm.unwrap().seq, -2, "{rel} {}", node.name);
                    n += 1;
                }
            }
            assert!(n > 0, "{rel}");
        }
    }
}
