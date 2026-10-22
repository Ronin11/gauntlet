//! CPU-side mesh assembly: turns decoded objects into batched vertex/index buffers.

use bytemuck::{Pod, Zeroable};
use gdl_formats::anim::TexMod;
use gdl_formats::objects::{object_flags, Object, Objects};
use gdl_formats::{texture, Image, Mesh};
use glam::{Mat4, Vec3};
use std::collections::{HashMap, HashSet};

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
    /// Additive ("fb_add") blending: glows, fire, light shafts.
    pub additive: bool,
    /// Rigid part this batch belongs to (index into an instance's part transforms).
    pub part: u16,
}

#[derive(Default)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    pub batches: Vec<Batch>,
    pub textures: Vec<Image>,
    /// Animated textures (the bank's global texmods), in texmod order.
    pub tex_anims: Vec<TexAnim>,
    /// Flipbooks of the bank's sequence texmods and texmod nodes, shown per
    /// instance (`MBTreeSetAltTex`), in texmod order without duplicates.
    pub seq_anims: Vec<TexAnim>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

/// Another loaded texture bank (objects + textures file). `TEXMOD_EXTERNAL`
/// sources are looked up by name in every loaded bank (`MBOX_FindTexture`), e.g.
/// the levels' torches take their frames (`TORCH00`...) from WEAPONS.
#[derive(Clone, Copy)]
pub struct TexBank<'a> {
    pub objects: &'a Objects,
    pub textures: &'a [u8],
}

#[derive(Debug, Clone, PartialEq)]
pub enum TexAnimKind {
    /// Flipbook: the texture slot of each frame (None when a frame failed to
    /// decode; the texture's own image stands in).
    Flip(Vec<Option<usize>>),
    /// The texture's UV scroll record: U offset.
    ScrollU,
    /// V offset.
    ScrollV,
}

/// One texmod applied to a texture slot of a [`MeshData`]. A global one's state
/// at a game frame comes from [`crate::texanim::global_state`]; a sequence
/// one's from [`crate::texanim::seq_state`] with an instance's animation frame.
#[derive(Debug, Clone)]
pub struct TexAnim {
    /// The animated texture's slot in `MeshData::textures`.
    pub slot: usize,
    pub kind: TexAnimKind,
    pub texmod: TexMod,
}

/// A texmod resolved against the banks, waiting for its texture to be used.
struct PendingTexMod {
    /// Bitmap index of the animated texture in the builder's own bank.
    target: u16,
    /// Flipbook frames: (bank, first bitmap index); bank 0 is the builder's own.
    frames: Option<(usize, u16)>,
    texmod: TexMod,
}

/// Builds a [`MeshData`] from objects that share one objects/textures file pair.
pub struct MeshBuilder<'a> {
    objects: &'a Objects,
    textures_file: &'a [u8],
    /// Other banks texmods may draw frames from (bank 1, 2, ...).
    banks: Vec<TexBank<'a>>,
    /// (bank, bitmap index) -> texture slot
    tex_slots: HashMap<(usize, u16), Option<usize>>,
    texmods: Vec<PendingTexMod>,
    /// Bitmaps animated by a texmod (they always get a slot).
    anim_targets: HashSet<u16>,
    /// Bitmap flags replaced by `MBCopyTexture` (external texmods copy the
    /// whole descriptor, flags included).
    flag_override: HashMap<u16, u16>,
    /// (texture, lightmap, blend, prelit) -> (vertices, indices) accumulated per batch
    groups: Vec<(
        (Option<usize>, Option<usize>, bool, bool, u16, bool),
        Vec<u32>,
    )>,
    /// Objects added while this is set are drawn with additive blending.
    pub additive: bool,
    pub data: MeshData,
}

impl<'a> MeshBuilder<'a> {
    pub fn new(objects: &'a Objects, textures_file: &'a [u8]) -> Self {
        MeshBuilder {
            objects,
            textures_file,
            banks: Vec::new(),
            tex_slots: HashMap::new(),
            texmods: Vec::new(),
            anim_targets: HashSet::new(),
            flag_override: HashMap::new(),
            groups: Vec::new(),
            additive: false,
            data: MeshData {
                bounds_min: Vec3::splat(f32::MAX),
                bounds_max: Vec3::splat(f32::MIN),
                ..Default::default()
            },
        }
    }

    /// Animate this bank's textures with its texmod table (the bank's ANIM.PS2
    /// texmods), as `InitTexMods` sets them up: global texmods become
    /// [`MeshData::tex_anims`], the flipbooks of sequence texmods and texmod
    /// nodes [`MeshData::seq_anims`]. `banks` are the other loaded banks
    /// external sources are looked up in. Call before adding objects.
    pub fn with_texmods<'t>(
        mut self,
        texmods: impl IntoIterator<Item = &'t TexMod>,
        banks: &[TexBank<'a>],
    ) -> Self {
        self.banks = banks.to_vec();
        for tm in texmods {
            // InitTexMod: the stored index (in the model's own bank), else the name.
            let target = if tm.tex >= 0 {
                Some((tm.tex & 0xFFFF) as u16)
                    .filter(|&i| (i as usize) < self.objects.bitmaps.len())
            } else {
                find_bitmap(self.objects, &tm.name)
            };
            let Some(target) = target else {
                log::debug!(
                    "TEXMOD with 0 texidx, tex:{} src:{}",
                    tm.name,
                    tm.source_name
                );
                continue;
            };
            let frames = match tm.source {
                s if s >= 0 => {
                    // An index in another bank (high half set) is not used by the data.
                    (s as u32 & 0xFFFF_0000 == 0).then_some((0, s as u16))
                }
                TexMod::EXTERNAL => {
                    let found = self.find_texture(&tm.source_name);
                    if let Some((bank, i)) = found {
                        // MBCopyTexture copies the source's descriptor, flags too.
                        if let Some(b) = self.bank(bank).objects.bitmaps.get(i as usize) {
                            self.flag_override.insert(target, b.flags);
                        }
                    } else {
                        log::debug!("texmod {}: no texture {}", tm.name, tm.source_name);
                    }
                    found
                }
                // Only global scrolls get a scroll record (InitTexMod); sequence
                // scrolls and fades act on nodes, not textures.
                TexMod::SCROLL_U | TexMod::SCROLL_V if tm.is_global() => None,
                _ => continue,
            };
            if tm.source >= TexMod::EXTERNAL && frames.is_none() {
                continue;
            }
            self.anim_targets.insert(target);
            self.texmods.push(PendingTexMod {
                target,
                frames,
                texmod: tm.clone(),
            });
        }
        self
    }

    fn bank(&self, bank: usize) -> TexBank<'a> {
        match bank {
            0 => TexBank {
                objects: self.objects,
                textures: self.textures_file,
            },
            b => self.banks[b - 1],
        }
    }

    /// `MBOX_FindTexture`: a bitmap by name in the loaded banks.
    fn find_texture(&self, name: &str) -> Option<(usize, u16)> {
        (0..=self.banks.len()).find_map(|b| find_bitmap(self.bank(b).objects, name).map(|i| (b, i)))
    }

    fn texture_slot(&mut self, index: u16) -> Option<usize> {
        self.bank_slot(0, index)
    }

    fn bank_slot(&mut self, bank: usize, index: u16) -> Option<usize> {
        if let Some(s) = self.tex_slots.get(&(bank, index)) {
            return *s;
        }
        let TexBank { objects, textures } = self.bank(bank);
        let img = objects.bitmaps.get(index as usize).and_then(|b| {
            match texture::decode_ngc(b, textures) {
                Ok(img) => Some(img),
                Err(e) => {
                    log::debug!("texture {bank}/{index}: {e:#}");
                    None
                }
            }
        });
        // Animated textures always get a slot: their frames replace the image.
        let img = img.or_else(|| {
            (bank == 0 && self.anim_targets.contains(&index)).then(|| Image::glow(16, 16))
        });
        let slot = img.map(|img| {
            self.data.textures.push(img);
            self.data.textures.len() - 1
        });
        self.tex_slots.insert((bank, index), slot);
        slot
    }

    fn bitmap_has_alpha(&self, index: u16) -> bool {
        let flags = self
            .flag_override
            .get(&index)
            .copied()
            .or_else(|| self.objects.bitmaps.get(index as usize).map(|b| b.flags));
        flags.is_some_and(|f| f & gdl_formats::objects::bitmap_flags::HAS_ALPHA != 0)
    }

    /// Append an object, transformed by `xf`.
    pub fn add_object(&mut self, obj: &Object, xf: Mat4) {
        self.add_part(obj, xf, 0);
    }

    /// Append an object as rigid part `part`, transformed by `xf`.
    pub fn add_part(&mut self, obj: &Object, xf: Mat4, part: u16) {
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
            let key = (
                tex,
                lm,
                blend && !self.additive,
                prelit && lm.is_none(),
                part,
                self.additive,
            );
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
        // Texmods of textures the geometry uses: decode their frames.
        for p in std::mem::take(&mut self.texmods) {
            let Some(Some(slot)) = self.tex_slots.get(&(0, p.target)).copied() else {
                continue;
            };
            let kind = match (p.texmod.source, p.frames) {
                (TexMod::SCROLL_U, _) => TexAnimKind::ScrollU,
                (TexMod::SCROLL_V, _) => TexAnimKind::ScrollV,
                (_, Some((bank, first))) => {
                    let n = (p.texmod.frames as i32).unsigned_abs().max(1);
                    TexAnimKind::Flip(
                        (0..n)
                            .map(|k| {
                                u16::try_from(first as u32 + k)
                                    .ok()
                                    .and_then(|i| self.bank_slot(bank, i))
                            })
                            .collect(),
                    )
                }
                _ => continue,
            };
            let anim = TexAnim {
                slot,
                kind,
                texmod: p.texmod,
            };
            if anim.texmod.is_global() {
                self.data.tex_anims.push(anim);
            } else if !self.data.seq_anims.iter().any(|a| a.texmod == anim.texmod) {
                self.data.seq_anims.push(anim);
            }
        }
        for ((texture, lightmap, blend, prelit, part, additive), idx) in
            std::mem::take(&mut self.groups)
        {
            let start = self.data.indices.len() as u32;
            self.data.indices.extend(idx);
            self.data.batches.push(Batch {
                start,
                count: self.data.indices.len() as u32 - start,
                texture,
                lightmap,
                blend,
                prelit,
                additive,
                part,
            });
        }
        if self.data.vertices.is_empty() {
            self.data.bounds_min = Vec3::ZERO;
            self.data.bounds_max = Vec3::ZERO;
        }
        self.data
    }
}

/// A bitmap's index by name (the bank's texture definitions, `texcmp` ignores case).
fn find_bitmap(objects: &Objects, name: &str) -> Option<u16> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    objects
        .bitmap_defs
        .iter()
        .find(|d| d.name.eq_ignore_ascii_case(name))
        .map(|d| d.tex_index)
}
