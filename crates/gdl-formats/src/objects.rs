//! `objects.ngc` / `objects.ps2`: model geometry plus texture headers.
//!
//! Layout follows the v12/v13 object library format (version word 0xF00B000D).
//! All fields are little-endian, even on GameCube. Pointers are absolute file offsets.

use crate::bin::Le;
use anyhow::{bail, Context, Result};

pub const VERSION_V13: u32 = 0xF00B_000D;
pub const VERSION_V12: u32 = 0xF00B_000C;

pub mod object_flags {
    pub const ALPHA: u32 = 0x01;
    pub const VNORMS: u32 = 0x02;
    pub const VCOLORS: u32 = 0x04;
    pub const MESH: u32 = 0x08;
    pub const TEX2: u32 = 0x10;
    pub const LMAP: u32 = 0x20;
    pub const SHARP: u32 = 0x40;
    pub const BLUR: u32 = 0x80;
    pub const CHROME: u32 = 0x100;
    pub const SORT_A: u32 = 0x400;
    pub const SORT: u32 = 0x800;
}

pub mod bitmap_flags {
    pub const HALFRES: u16 = 0x0001;
    pub const SEE_ALPHA: u16 = 0x0002;
    pub const CLAMP_U: u16 = 0x0004;
    pub const CLAMP_V: u16 = 0x0008;
    pub const ANIMATION: u16 = 0x0010;
    pub const EXTERNAL: u16 = 0x0020;
    pub const HAS_ALPHA: u16 = 0x0080;
    pub const INVALID: u16 = 0x0100;
}

#[derive(Debug, Clone, Copy)]
pub struct SubObject {
    pub qword_count: u16,
    pub tex_index: u16,
    pub lm_index: u16,
    pub lod_k: i16,
}

#[derive(Debug, Clone)]
pub struct Object {
    pub inv_rad: f32,
    pub bnd_rad: f32,
    pub flags: u32,
    pub sub_objects: Vec<SubObject>,
    /// Raw geometry stream for each sub-object (header stripped).
    pub geometry: Vec<Vec<u8>>,
    pub vert_count: i32,
    pub tri_count: i32,
    pub id_num: i32,
}

#[derive(Debug, Clone)]
pub struct ObjectDef {
    pub name: String,
    pub bnd_rad: f32,
    pub obj_index: i16,
    pub frames: i16,
}

/// Texture header. Pixel data lives in the companion `textures.ngc` at `tex_pointer`.
#[derive(Debug, Clone)]
pub struct Bitmap {
    pub format: u8,
    pub lod_k: i8,
    pub mipmap_count: u8,
    pub flags: u16,
    pub tex_palette_index: u16,
    pub tex_pointer: u32,
    pub tex_palette_count: u16,
    pub tex_shift_index: u16,
    pub frame_count: u16,
    pub width: u16,
    pub height: u16,
    pub size: u16,
}

#[derive(Debug, Clone)]
pub struct BitmapDef {
    pub name: String,
    pub tex_index: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug, Clone)]
pub struct Objects {
    pub dir_name: String,
    pub model_name: String,
    pub version: u32,
    pub objects: Vec<Object>,
    pub bitmaps: Vec<Bitmap>,
    pub object_defs: Vec<ObjectDef>,
    pub bitmap_defs: Vec<BitmapDef>,
}

impl Objects {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let r = Le::new(data);
        let version = r.u32(64)?;
        if version != VERSION_V13 && version != VERSION_V12 {
            bail!("unsupported objects version {version:#010x}");
        }
        let objects_count = r.u32(68)? as usize;
        let bitmaps_count = r.u32(72)? as usize;
        let object_defs_count = r.u32(76)? as usize;
        let bitmap_defs_count = r.u32(80)? as usize;
        let objects_ptr = r.u32(84)? as usize;
        let bitmaps_ptr = r.u32(88)? as usize;
        let object_defs_ptr = r.u32(92)? as usize;
        let bitmap_defs_ptr = r.u32(96)? as usize;

        let mut objects = Vec::with_capacity(objects_count);
        for i in 0..objects_count {
            objects.push(
                parse_object(r, objects_ptr + i * 64).with_context(|| format!("object {i}"))?,
            );
        }

        let mut bitmaps = Vec::with_capacity(bitmaps_count);
        for i in 0..bitmaps_count {
            let o = bitmaps_ptr + i * 64;
            bitmaps.push(Bitmap {
                format: r.u8(o)?,
                lod_k: r.i8(o + 1)?,
                mipmap_count: r.u8(o + 2)?,
                flags: r.u16(o + 8)?,
                tex_palette_index: r.u16(o + 10)?,
                tex_pointer: r.u32(o + 12)?,
                tex_palette_count: r.u16(o + 16)?,
                tex_shift_index: r.u16(o + 18)?,
                frame_count: r.u16(o + 20)?,
                width: r.u16(o + 22)?,
                height: r.u16(o + 24)?,
                size: r.u16(o + 26)?,
            });
        }

        let mut object_defs = Vec::with_capacity(object_defs_count);
        for i in 0..object_defs_count {
            let o = object_defs_ptr + i * 24;
            object_defs.push(ObjectDef {
                name: r.str(o, 16)?,
                bnd_rad: r.f32(o + 16)?,
                obj_index: r.i16(o + 20)?,
                frames: r.i16(o + 22)?,
            });
        }

        let mut bitmap_defs = Vec::with_capacity(bitmap_defs_count);
        for i in 0..bitmap_defs_count {
            let o = bitmap_defs_ptr + i * 36;
            bitmap_defs.push(BitmapDef {
                name: r.str(o, 30)?,
                tex_index: r.u16(o + 30)?,
                width: r.u16(o + 32)?,
                height: r.u16(o + 34)?,
            });
        }

        Ok(Objects {
            dir_name: r.str(0, 32)?,
            model_name: r.str(32, 32)?,
            version,
            objects,
            bitmaps,
            object_defs,
            bitmap_defs,
        })
    }

    /// Look up an object by its definition name (case-insensitive).
    pub fn object_by_name(&self, name: &str) -> Option<&Object> {
        let def = self
            .object_defs
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))?;
        self.objects.get(usize::try_from(def.obj_index).ok()?)
    }
}

fn parse_sub_object(r: Le, o: usize) -> Result<SubObject> {
    Ok(SubObject {
        qword_count: r.u16(o)?,
        tex_index: r.u16(o + 2)?,
        lm_index: r.u16(o + 4)?,
        lod_k: r.i16(o + 6)?,
    })
}

fn parse_object(r: Le, o: usize) -> Result<Object> {
    let sub_count = r.i32(o + 12)?.max(0) as usize;
    let sub_ptr = r.u32(o + 24)? as usize;
    let models_ptr = r.u32(o + 28)? as usize;

    let mut sub_objects = Vec::with_capacity(sub_count);
    if sub_count > 0 {
        sub_objects.push(parse_sub_object(r, o + 16)?);
        for i in 1..sub_count {
            sub_objects.push(parse_sub_object(r, sub_ptr + (i - 1) * 8)?);
        }
    }

    // Each sub-object model is an 8-byte header (u16 qword count + 6 bytes) followed
    // by `(qwc + 1) * 16 - 8` bytes of stream data, so each block is `(qwc + 1) * 16`.
    let mut geometry = Vec::with_capacity(sub_count);
    if models_ptr != 0 {
        let mut at = models_ptr;
        for _ in 0..sub_count {
            let qwc = r.u16(at)? as usize;
            let len = (qwc + 1) * 16 - 8;
            geometry.push(r.bytes(at + 8, len)?.to_vec());
            at += (qwc + 1) * 16;
        }
    }

    Ok(Object {
        inv_rad: r.f32(o)?,
        bnd_rad: r.f32(o + 4)?,
        flags: r.u32(o + 8)?,
        sub_objects,
        geometry,
        vert_count: r.i32(o + 32)?,
        tri_count: r.i32(o + 36)?,
        id_num: r.i32(o + 40)?,
    })
}
