//! `WORLDS.PS2`: level layout. World geometry objects (named after entries in the
//! level's `objects.ngc`), collision triangles, a collision grid, item placements
//! and locators (player starts, cameras, ...). Little-endian; Y is up.

use crate::bin::Le;
use anyhow::{Context, Result};

const COLL_SCALE: f32 = 1.0 / 64.0;

pub mod world_flags {
    pub const PARTICLE_SYSTEM: u32 = 1 << 11;
    pub const ANIMATED: u32 = 1 << 12;
}

#[derive(Debug, Clone)]
pub struct WorldObject {
    pub name: String,
    pub flags: u32,
    pub mb_flags: u32,
    /// Position relative to the parent object.
    pub pos: [f32; 3],
    pub next_index: i16,
    pub child_index: i16,
    pub radius: f32,
    pub no_collision: i8,
    pub coll_tri_count: i16,
    pub coll_tri_index: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct CollTri {
    pub min_y: f32,
    pub max_y: f32,
    pub norm: [f32; 3],
    /// Fully decoded world-space vertices.
    pub verts: [[f32; 3]; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemType {
    None,
    Powerup,
    Container,
    Generator,
    Enemy,
    Trigger,
    Trap,
    Door,
    DamageTile,
    Exit,
    Obstacle,
    Transporter,
    Rotator,
    Sound,
    Random,
    Unknown(i32),
}

impl ItemType {
    fn from_i32(v: i32) -> Self {
        use ItemType::*;
        match v {
            0 => None,
            1 => Powerup,
            2 => Container,
            3 => Generator,
            4 => Enemy,
            5 => Trigger,
            6 => Trap,
            7 => Door,
            8 => DamageTile,
            9 => Exit,
            10 => Obstacle,
            11 => Transporter,
            12 => Rotator,
            13 => Sound,
            -1 => Random,
            v => Unknown(v),
        }
    }
}

/// Item subtype names (index = subtype id). Ids 17..=19 and 32..=39 are unused.
pub const ITEM_SUBTYPES: &[&str] = &[
    "none",
    "gold",
    "key",
    "food",
    "potion",
    "weapon",
    "armor",
    "speed",
    "magic",
    "special",
    "runestone",
    "bosskey",
    "obelisk",
    "legend_item",
    "scroll",
    "crystal",
    "gargoyle_item",
    "unused17",
    "unused18",
    "unused19",
    "bridge_pad",
    "door_pad",
    "bridge_switch",
    "door_switch",
    "activator_switch",
    "elevator_pad",
    "elevator_switch",
    "lift_pad",
    "lift_start",
    "lift_end",
    "no_weapon_collider",
    "shoot_trigger",
    "unused32",
    "unused33",
    "unused34",
    "unused35",
    "unused36",
    "unused37",
    "unused38",
    "unused39",
    "rock_fall",
    "safe_rock",
    "wall",
    "barrel",
    "barrel_explode",
    "barrel_poison",
    "chest",
];

#[derive(Debug, Clone)]
pub struct ItemInfo {
    pub item_type: ItemType,
    pub subtype: i32,
    pub coll_type: i16,
    pub coll_flags: u16,
    pub radius: f32,
    pub height: f32,
    pub x_dim: f32,
    pub z_dim: f32,
    pub coll_offset: [f32; 3],
    /// Model / animation-tree name for this item.
    pub name: String,
    pub properties: u32,
    pub value: i16,
    pub armor: i16,
    pub hit_points: i16,
    pub active_type: i16,
    pub active_off: i16,
    /// For power-ups, seconds they last (scaled by the class's powerup_time);
    /// negative for ones counted in uses.
    pub active_on: i16,
    /// For `Random` items: candidate item_info indices.
    pub random_indices: Vec<i16>,
}

impl ItemInfo {
    pub fn subtype_name(&self) -> &'static str {
        ITEM_SUBTYPES
            .get(self.subtype as usize)
            .copied()
            .unwrap_or("?")
    }
}

#[derive(Debug, Clone)]
pub struct ItemInstance {
    pub item_index: i16,
    pub min_players: i8,
    pub flags: u8,
    pub coll_tri_index: i16,
    pub coll_tri_count: i16,
    pub name: String,
    pub pos: [f32; 3],
    /// Pitch, yaw, roll in radians.
    pub rot: [f32; 3],
    /// Type-specific parameters (interpretation depends on the item type).
    pub params: [u8; 12],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocatorType {
    None,
    CameraStart,
    CameraGame,
    CameraAttractStart,
    CameraAttract,
    Milestone,
    Boss,
    Start,
    Sentry,
    TriggerCamera,
    Event,
    Unknown(i8),
}

/// A world object's keyframe animation (gauntworld.c `struct worldanim`,
/// 0x10 bytes, from the worlds file's version >= 1 header words 0x19..0x1B).
#[derive(Debug, Clone)]
pub struct WorldAnim {
    /// The animated world object (`objidx`).
    pub obj: usize,
    /// Frame count (`nframes`).
    pub nframes: i16,
    /// Initial `curframe`.
    pub curframe: f32,
    /// The decoded keyframe stream (`data`: mode word, record size, offset
    /// into the animheader's block); None when its mode has no channels.
    pub track: Option<crate::anim::Track>,
}

#[derive(Debug, Clone)]
pub struct Locator {
    pub kind: LocatorType,
    pub delay: u8,
    pub next: u16,
    pub pos: [f32; 3],
    pub rot: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct Worlds {
    pub world_objects: Vec<WorldObject>,
    pub coll_tris: Vec<CollTri>,
    pub min_bounds: [f32; 3],
    pub max_bounds: [f32; 3],
    pub grid_size: f32,
    pub grid_number_x: u32,
    pub grid_number_z: u32,
    pub item_infos: Vec<ItemInfo>,
    pub item_instances: Vec<ItemInstance>,
    pub locators: Vec<Locator>,
    /// World-object keyframe animations (`gWorldInfo.worldanims`).
    pub world_anims: Vec<WorldAnim>,
    /// The particle-system templates `PSYS<id>` objects select
    /// (`gWorldInfo.worldpsys`, version >= 2 header words 0x1C/0x1D).
    pub world_psys: Vec<crate::psys::PsysRecord>,
}

fn rotate_by_quat(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let s = q[3];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let a = 2.0 * dot(u, v);
    let b = s * s - dot(u, u);
    let c = 2.0 * s;
    [
        u[0] * a + v[0] * b + cross[0] * c,
        u[1] * a + v[1] * b + cross[1] * c,
        u[2] * a + v[2] * b + cross[2] * c,
    ]
}

/// Collision triangles store v1/v2 as 2D offsets in the triangle's plane; rotate
/// them back into world space using the plane normal.
fn plane_to_world(norm: [f32; 3], scale: f32, v0: [f32; 3], x: f32, z: f32) -> [f32; 3] {
    let roll = norm[1].clamp(-1.0, 1.0).acos();
    let yaw = if scale.is_finite() {
        (-norm[0] * scale).atan2(-norm[2] * scale)
    } else {
        0.0
    };
    let (c0, c1) = ((yaw / 2.0).cos(), (roll / 2.0).cos());
    let (s0, s1) = ((yaw / 2.0).sin(), (roll / 2.0).sin());
    let q = [-c0 * s1, s0 * c1, s0 * s1, c0 * c1];
    let d = rotate_by_quat([x, 0.0, z], q);
    [v0[0] + d[0], v0[1] + d[1], v0[2] + d[2]]
}

/// InitWorldInfo's world-animation block: header magic 0xF00BABvv at word
/// 0x18; for vv >= 1, the animheader (SetupAnimHeader: delta tables and the
/// keyframe block, offsets from itself) at word 0x19 and `nworldanims`
/// records at word 0x1B.
fn parse_world_anims(r: Le, nobjs: usize) -> Result<Vec<WorldAnim>> {
    let magic = r.u32(0x18 * 4).unwrap_or(0);
    let version = if magic & 0xF00B_AB00 == 0xF00B_AB00 {
        magic & 0xFF
    } else {
        0
    };
    let hdr = r.u32(0x19 * 4).unwrap_or(0) as usize;
    if version < 1 || hdr == 0 {
        return Ok(Vec::new());
    }
    let count = r.u32(0x1A * 4)? as usize;
    let recs = r.u32(0x1B * 4)? as usize;
    let table = |w: usize| -> Result<Option<Vec<f32>>> {
        let p = r.u32(hdr + w * 4)? as usize;
        if p == 0 {
            return Ok(None);
        }
        (0..256)
            .map(|i| r.f32(hdr + p + i * 4))
            .collect::<Result<Vec<_>>>()
            .map(Some)
    };
    let tables = [table(0)?, table(1)?, table(2)?];
    let block = hdr + r.u32(hdr + 12)? as usize;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let o = recs + i * 16;
        let obj = r.i16(o)?;
        let nframes = r.i16(o + 2)?;
        let curframe = r.f32(o + 8)?;
        let data = r.u32(o + 12)? as usize;
        if obj < 0 || obj as usize >= nobjs {
            continue;
        }
        // DoWorldAnimation skips a track without data.
        if data == 0 {
            continue;
        }
        // The stream header: mode, record size, keyframe offset.
        let track = {
            let mode = r.u16(data)?;
            let size = r.u16(data + 2)? as usize;
            let at = block + r.u32(data + 4)? as usize;
            if mode & 0xFFF == 0 {
                None
            } else {
                let t = [
                    tables[0].as_deref(),
                    tables[1].as_deref(),
                    tables[2].as_deref(),
                ];
                Some(
                    crate::anim::decode_track(r, at, mode, size, nframes.max(1) as usize, t)
                        .with_context(|| format!("world anim {i}"))?,
                )
            }
        };
        out.push(WorldAnim {
            obj: obj as usize,
            nframes,
            curframe,
            track,
        });
    }
    Ok(out)
}

/// InitWorldInfo's particle templates: for header version >= 2, `blob[0x1C]`
/// records at offset `blob[0x1D]`.
fn parse_world_psys(r: Le) -> Result<Vec<crate::psys::PsysRecord>> {
    let magic = r.u32(0x18 * 4).unwrap_or(0);
    let version = if magic & 0xF00B_AB00 == 0xF00B_AB00 {
        magic & 0xFF
    } else {
        0
    };
    let at = r.u32(0x1D * 4).unwrap_or(0) as usize;
    if version < 2 || at == 0 {
        return Ok(Vec::new());
    }
    let count = r.u32(0x1C * 4)? as usize;
    crate::psys::PsysRecord::parse_list(r, at, count).context("world psys")
}

impl Worlds {
    /// WorldPsysActivate's template for a world object: the record whose id
    /// is the character after "PSYS" in its name (GetWorldPsysIdx).
    pub fn psys_for(&self, obj: &WorldObject) -> Option<&crate::psys::PsysRecord> {
        let at = obj.name.find("PSYS")?;
        let id = *obj.name.as_bytes().get(at + 4)?;
        self.world_psys.iter().find(|p| p.id == id)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let r = Le::new(data);
        let wo_count = r.u32(0)? as usize;
        let wo_ptr = r.u32(4)? as usize;
        let ct_count = r.u32(8)? as usize;
        let ct_ptr = r.u32(12)? as usize;
        let ii_count = r.u32(72)? as usize;
        let ii_ptr = r.u32(76)? as usize;
        let inst_count = r.u32(80)? as usize;
        let inst_ptr = r.u32(84)? as usize;
        let loc_count = r.u32(88)? as usize;
        let loc_ptr = r.u32(92)? as usize;

        let mut world_objects = Vec::with_capacity(wo_count);
        for i in 0..wo_count {
            let o = wo_ptr + i * 60;
            world_objects.push(WorldObject {
                name: r.str(o, 16)?,
                flags: r.u32(o + 16)?,
                mb_flags: r.u32(o + 24)?,
                pos: r.vec3(o + 28)?,
                next_index: r.i16(o + 44)?,
                child_index: r.i16(o + 46)?,
                radius: r.f32(o + 48)?,
                no_collision: r.i8(o + 53)?,
                coll_tri_count: r.i16(o + 54)?,
                coll_tri_index: r.i32(o + 56)?,
            });
        }

        let mut coll_tris = Vec::with_capacity(ct_count);
        for i in 0..ct_count {
            let o = ct_ptr + i * 40;
            let scale = r.f32(o + 4)?;
            let norm = r.vec3(o + 8)?;
            let v0 = r.vec3(o + 20)?;
            let xz = |k| r.i16(o + 32 + k).map(|v| v as f32 * COLL_SCALE);
            let v1 = plane_to_world(norm, scale, v0, xz(0)?, xz(2)?);
            let v2 = plane_to_world(norm, scale, v0, xz(4)?, xz(6)?);
            coll_tris.push(CollTri {
                min_y: r.i16(o)? as f32 * COLL_SCALE,
                max_y: r.i16(o + 2)? as f32 * COLL_SCALE,
                norm,
                verts: [v0, v1, v2],
            });
        }

        let mut item_infos = Vec::with_capacity(ii_count);
        for i in 0..ii_count {
            let o = ii_ptr + i * 80;
            let item_type = ItemType::from_i32(r.i32(o)?);
            let d = o + 4;
            if item_type == ItemType::Random {
                let n = r.i32(d)?.clamp(0, 16) as usize;
                let random_indices = (0..n)
                    .map(|k| r.i16(d + 4 + k * 2))
                    .collect::<Result<_>>()?;
                item_infos.push(ItemInfo {
                    item_type,
                    subtype: 0,
                    coll_type: -1,
                    coll_flags: 0,
                    radius: 0.0,
                    height: 0.0,
                    x_dim: 0.0,
                    z_dim: 0.0,
                    coll_offset: [0.0; 3],
                    name: String::new(),
                    properties: 0,
                    value: 0,
                    armor: 0,
                    hit_points: 0,
                    active_type: 0,
                    active_off: 0,
                    active_on: 0,
                    random_indices,
                });
                continue;
            }
            item_infos.push(ItemInfo {
                item_type,
                subtype: r.i32(d)?,
                coll_type: r.i16(d + 4)?,
                coll_flags: r.u16(d + 6)?,
                radius: r.f32(d + 8)?,
                height: r.f32(d + 12)?,
                x_dim: r.f32(d + 16)?,
                z_dim: r.f32(d + 20)?,
                coll_offset: r.vec3(d + 24)?,
                name: r.str(d + 36, 16)?,
                properties: r.u32(d + 56)?,
                value: r.i16(d + 60)?,
                armor: r.i16(d + 62)?,
                hit_points: r.i16(d + 64)?,
                active_type: r.i16(d + 66)?,
                active_off: r.i16(d + 68)?,
                active_on: r.i16(d + 70)?,
                random_indices: Vec::new(),
            });
        }

        let mut item_instances = Vec::with_capacity(inst_count);
        for i in 0..inst_count {
            let o = inst_ptr + i * 60;
            item_instances.push(ItemInstance {
                item_index: r.i16(o)?,
                min_players: r.i8(o + 2)?,
                flags: r.u8(o + 3)?,
                coll_tri_index: r.i16(o + 4)?,
                coll_tri_count: r.i16(o + 6)?,
                name: r.str(o + 8, 16)?,
                pos: r.vec3(o + 24)?,
                rot: r.vec3(o + 36)?,
                params: r.bytes(o + 48, 12)?.try_into().unwrap(),
            });
        }

        let mut locators = Vec::with_capacity(loc_count);
        for i in 0..loc_count {
            let o = loc_ptr + i * 28;
            use LocatorType::*;
            let kind = match r.i8(o)? {
                0 => None,
                1 => CameraStart,
                2 => CameraGame,
                3 => CameraAttractStart,
                4 => CameraAttract,
                5 => Milestone,
                6 => Boss,
                7 => Start,
                8 => Sentry,
                9 => TriggerCamera,
                10 => Event,
                v => Unknown(v),
            };
            locators.push(Locator {
                kind,
                delay: r.u8(o + 1)?,
                next: r.u16(o + 2)?,
                pos: r.vec3(o + 4)?,
                rot: r.vec3(o + 16)?,
            });
        }

        let world_anims = parse_world_anims(r, world_objects.len())?;
        let world_psys = parse_world_psys(r)?;

        Ok(Worlds {
            world_objects,
            coll_tris,
            min_bounds: r.vec3(36).context("bounds")?,
            max_bounds: r.vec3(48)?,
            grid_size: r.f32(60)?,
            grid_number_x: r.u32(64)?,
            grid_number_z: r.u32(68)?,
            item_infos,
            item_instances,
            locators,
            world_anims,
            world_psys,
        })
    }

    /// Absolute position of every world object, resolving the child/next hierarchy.
    /// WorldObjGetAllFlags: each object's flags OR'd with its ancestors'.
    pub fn all_flags(&self) -> Vec<u32> {
        let n = self.world_objects.len();
        let mut out: Vec<Option<u32>> = vec![None; n];
        let mut is_child = vec![false; n];
        for o in &self.world_objects {
            let mut c = o.child_index;
            while c >= 0 && (c as usize) < n && !is_child[c as usize] {
                is_child[c as usize] = true;
                c = self.world_objects[c as usize].next_index;
            }
        }
        fn walk(w: &Worlds, first: i16, base: u32, out: &mut [Option<u32>]) {
            let mut i = first;
            while i >= 0 && (i as usize) < out.len() && out[i as usize].is_none() {
                let o = &w.world_objects[i as usize];
                let f = base | o.flags;
                out[i as usize] = Some(f);
                walk(w, o.child_index, f, out);
                i = o.next_index;
            }
        }
        for i in 0..n {
            if !is_child[i] && out[i].is_none() {
                let f = self.world_objects[i].flags;
                out[i] = Some(f);
                walk(self, self.world_objects[i].child_index, f, &mut out);
            }
        }
        out.into_iter().map(|f| f.unwrap_or(0)).collect()
    }

    pub fn world_positions(&self) -> Vec<[f32; 3]> {
        let n = self.world_objects.len();
        let mut out: Vec<Option<[f32; 3]>> = vec![None; n];
        let mut is_child = vec![false; n];
        for o in &self.world_objects {
            let mut c = o.child_index;
            while c >= 0 && (c as usize) < n && !is_child[c as usize] {
                is_child[c as usize] = true;
                c = self.world_objects[c as usize].next_index;
            }
        }
        fn walk(w: &Worlds, first: i16, base: [f32; 3], out: &mut [Option<[f32; 3]>]) {
            let mut i = first;
            while i >= 0 && (i as usize) < out.len() && out[i as usize].is_none() {
                let o = &w.world_objects[i as usize];
                let p = [base[0] + o.pos[0], base[1] + o.pos[1], base[2] + o.pos[2]];
                out[i as usize] = Some(p);
                walk(w, o.child_index, p, out);
                i = o.next_index;
            }
        }
        for i in 0..n {
            if !is_child[i] && out[i].is_none() {
                let o = &self.world_objects[i];
                out[i] = Some(o.pos);
                walk(self, o.child_index, o.pos, &mut out);
            }
        }
        out.into_iter().map(|p| p.unwrap_or([0.0; 3])).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_inherit_hazard_flags() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/LEVELS/levelH1/WORLDS.PS2");
        let Ok(data) = std::fs::read(p) else { return };
        let w = Worlds::parse(&data).unwrap();
        let f = w.all_flags();
        let burning = w
            .world_objects
            .iter()
            .zip(&f)
            .filter(|(o, f)| o.name.starts_with("H1FLOORB") && *f & 0xF0000 == 0x10000)
            .count();
        assert_eq!(burning, 13);
    }

    #[test]
    fn world_psys_templates() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/LEVELS/levelH1/WORLDS.PS2");
        let Ok(data) = std::fs::read(p) else { return };
        let w = Worlds::parse(&data).unwrap();
        let ids: String = w.world_psys.iter().map(|p| p.id as char).collect();
        assert_eq!(ids, "ABDEFGHIJK");
        let a = &w.world_psys[0];
        assert_eq!((a.version, a.preset, a.enables), (0x101, 5, 0x56be1));
        assert_eq!((a.flags, a.flag_mask), (0x288, 0x288));
        assert_eq!(a.tex_name, "P_TORCH");
        assert_eq!(a.p_lifefade, [0.2, 0.22]);
        assert_eq!((a.e_angle, a.speed, a.gravity), (80.0, 4.0, -0.38));
        assert_eq!(a.rgba, [0, 0xFF_FFFF, 0xFF_FFFF, 0]);
        assert_eq!(a.width, [2.8, 2.0, 2.0, 0.1]);
        // Objects name their template: H1PSYSJ1 uses J (POOLFIRE).
        let o = w
            .world_objects
            .iter()
            .find(|o| o.name == "H1PSYSJ1")
            .unwrap();
        assert_eq!(
            o.flags & world_flags::PARTICLE_SYSTEM,
            world_flags::PARTICLE_SYSTEM
        );
        assert_eq!(w.psys_for(o).unwrap().tex_name, "POOLFIRE");
        // Its collision triangle faces down; WorldPsysActivate negates it.
        assert_eq!(
            w.coll_tris[o.coll_tri_index as usize].norm,
            [0.0, -1.0, 0.0]
        );
    }

    #[test]
    fn world_anims_parse() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../game-data/files/Gauntlet/LEVELS/levelA2/WORLDS.PS2");
        let Ok(data) = std::fs::read(p) else { return };
        let w = Worlds::parse(&data).unwrap();
        assert_eq!(w.world_anims.len(), 11);
        let hog = w
            .world_anims
            .iter()
            .find(|a| w.world_objects[a.obj].name == "A2HOG04")
            .unwrap();
        assert_eq!(hog.nframes, 260);
        let t = hog.track.as_ref().unwrap();
        // A yaw track keyed over the whole run.
        assert_eq!(t.flags & 0xFFF, 2);
        assert!(t.keys.len() > 100 && t.keys.last().unwrap().0 < 260);
    }
}
