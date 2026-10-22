//! CPU-side mesh assembly: turns decoded objects into batched vertex/index buffers.

use bytemuck::{Pod, Zeroable};
use gdl_formats::objects::{object_flags, Object, Objects};
use gdl_formats::{texture, Image, Mesh};
use glam::{Mat4, Vec3};
use std::collections::HashMap;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
    pub lm_uv: [f32; 2],
}

#[derive(Debug, Clone)]
pub struct Batch {
    pub start: u32,
    pub count: u32,
    /// Index into `MeshData::textures`.
    pub texture: Option<usize>,
    pub lightmap: Option<usize>,
    pub blend: bool,
    pub prelit: bool,
}

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    pub textures: Vec<Image>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

/// Builds a [`MeshData`] from objects that share one objects/textures file pair.
pub struct MeshBuilder<'a> {
    objects: &'a Objects,
    textures_file: &'a [u8],
    /// bitmap index -> texture slot
    tex_slots: HashMap<u16, Option<usize>>,
    /// (texture, lightmap, blend, prelit) -> (vertices, indices) accumulated per batch
    groups: Vec<((Option<usize>, Option<usize>, bool, bool), Vec<u32>)>,
    pub data: MeshData,
}

impl<'a> MeshBuilder<'a> {
    pub fn new(objects: &'a Objects, textures_file: &'a [u8]) -> Self {
        MeshBuilder {
            objects,
            textures_file,
            tex_slots: HashMap::new(),
            groups: Vec::new(),
            data: MeshData {
                bounds_min: Vec3::splat(f32::MAX),
                bounds_max: Vec3::splat(f32::MIN),
                ..Default::default()
            },
        }
    }

    fn texture_slot(&mut self, index: u16) -> Option<usize> {
        if let Some(s) = self.tex_slots.get(&index) {
            return *s;
        }
        let slot =
            self.objects.bitmaps.get(index as usize).and_then(|b| {
                match texture::decode_ngc(b, self.textures_file) {
                    Ok(img) => {
                        self.data.textures.push(img);
                        Some(self.data.textures.len() - 1)
                    }
                    Err(e) => {
                        log::debug!("texture {index}: {e:#}");
                        None
                    }
                }
            });
        self.tex_slots.insert(index, slot);
        slot
    }

    fn bitmap_has_alpha(&self, index: u16) -> bool {
        self.objects
            .bitmaps
            .get(index as usize)
            .is_some_and(|b| b.flags & gdl_formats::objects::bitmap_flags::HAS_ALPHA != 0)
    }

    /// Append an object, transformed by `xf`.
    pub fn add_object(&mut self, obj: &Object, xf: Mat4) {
        let has_lmap = obj.flags & object_flags::LMAP != 0;
        let prelit = obj.flags & 0x30000 == 0x10000 || obj.flags & object_flags::VCOLORS != 0;
        let obj_alpha = obj.flags & (object_flags::ALPHA | object_flags::SORT_A) != 0;
        let normal_xf = xf.inverse().transpose();
        for (sub, stream) in obj.sub_objects.iter().zip(&obj.geometry) {
            let mesh = match Mesh::decode(stream) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("mesh decode: {e:#}");
                    continue;
                }
            };
            if mesh.indices.is_empty() {
                continue;
            }
            let tex = self.texture_slot(sub.tex_index);
            let lm = if has_lmap {
                self.texture_slot(sub.lm_index)
            } else {
                None
            };
            let blend = obj_alpha
                && self.bitmap_has_alpha(sub.tex_index)
                && obj.flags & object_flags::ALPHA != 0;
            let key = (tex, lm, blend, prelit && lm.is_none());
            let base = self.data.vertices.len() as u32;
            for i in 0..mesh.positions.len() {
                let p = xf.transform_point3(Vec3::from(mesh.positions[i]));
                let n = normal_xf
                    .transform_vector3(Vec3::from(mesh.normals[i]))
                    .normalize_or_zero();
                self.data.bounds_min = self.data.bounds_min.min(p);
                self.data.bounds_max = self.data.bounds_max.max(p);
                self.data.vertices.push(Vertex {
                    pos: p.to_array(),
                    normal: n.to_array(),
                    color: mesh.colors[i],
                    uv: mesh.uvs[i],
                    lm_uv: mesh.lm_uvs.get(i).copied().unwrap_or([0.0; 2]),
                });
            }
            let idx: Vec<u32> = mesh.indices.iter().map(|i| i + base).collect();
            match self.groups.iter_mut().find(|(k, _)| *k == key) {
                Some((_, v)) => v.extend(idx),
                None => self.groups.push((key, idx)),
            }
        }
    }

    pub fn finish(mut self) -> MeshData {
        for ((texture, lightmap, blend, prelit), idx) in std::mem::take(&mut self.groups) {
            let start = self.data.indices.len() as u32;
            self.data.indices.extend(idx);
            self.data.batches.push(Batch {
                start,
                count: self.data.indices.len() as u32 - start,
                texture,
                lightmap,
                blend,
                prelit,
            });
        }
        if self.data.vertices.is_empty() {
            self.data.bounds_min = Vec3::ZERO;
            self.data.bounds_max = Vec3::ZERO;
        }
        self.data
    }
}
