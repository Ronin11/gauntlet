//! Skinned glTF characters for the engine's rigid-part hero skeletons.
//!
//! [`ModernModel`] reads a rigged `.glb` (Tripo's biped output, or any glTF skin);
//! [`mapping`] says which glTF joint stands in for each game skeleton node.

pub mod clip;
pub mod mapping;
pub mod prop;
pub mod retarget;

use anyhow::{bail, Context, Result};
use glam::{Mat4, Vec2, Vec3};
use std::path::Path;

/// One skin joint: its place in the hierarchy and bind pose.
#[derive(Clone, Debug)]
pub struct Joint {
    pub name: String,
    /// Index into [`ModernModel::joints`] of the nearest joint ancestor.
    pub parent: Option<usize>,
    /// Bind-pose transform relative to `parent` (or to the model for roots).
    pub rest_local: Mat4,
    /// Bind-pose transform in model space.
    pub rest_world: Mat4,
    pub inverse_bind: Mat4,
}

#[derive(Clone, Copy, Debug)]
pub struct SkinVertex {
    pub pos: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
    pub joints: [u16; 4],
    pub weights: [f32; 4],
}

/// Tightly packed RGBA8.
#[derive(Clone, Debug)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ModernModel {
    pub vertices: Vec<SkinVertex>,
    pub indices: Vec<u32>,
    pub joints: Vec<Joint>,
    /// The base-colour texture of the first primitive's material.
    pub texture: Option<Texture>,
}

impl ModernModel {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let (doc, buffers, images) =
            gltf::import(path).with_context(|| format!("reading {}", path.display()))?;
        let skin = doc.skins().next().context("glTF has no skin")?;

        // Model-space transform of every node, parents first.
        let mut world = vec![Mat4::IDENTITY; doc.nodes().len()];
        let mut parent_of = vec![None; doc.nodes().len()];
        fn walk(n: gltf::Node, up: Mat4, world: &mut [Mat4], parent_of: &mut [Option<usize>]) {
            let w = up * Mat4::from_cols_array_2d(&n.transform().matrix());
            world[n.index()] = w;
            for c in n.children() {
                parent_of[c.index()] = Some(n.index());
                walk(c, w, world, parent_of);
            }
        }
        for scene in doc.scenes() {
            for n in scene.nodes() {
                walk(n, Mat4::IDENTITY, &mut world, &mut parent_of);
            }
        }

        let joint_nodes: Vec<usize> = skin.joints().map(|j| j.index()).collect();
        let slot_of = |node: usize| joint_nodes.iter().position(|&n| n == node);
        let reader = skin.reader(|b| Some(&buffers[b.index()]));
        let inverse_binds: Vec<Mat4> = match reader.read_inverse_bind_matrices() {
            Some(it) => it.map(|m| Mat4::from_cols_array_2d(&m)).collect(),
            None => joint_nodes.iter().map(|&n| world[n].inverse()).collect(),
        };
        let joints = skin
            .joints()
            .enumerate()
            .map(|(i, j)| {
                let mut p = parent_of[j.index()];
                let parent = loop {
                    match p {
                        Some(n) => match slot_of(n) {
                            Some(s) => break Some(s),
                            None => p = parent_of[n],
                        },
                        None => break None,
                    }
                };
                let rest_world = world[j.index()];
                let rest_local = match parent {
                    Some(s) => world[joint_nodes[s]].inverse() * rest_world,
                    None => rest_world,
                };
                Joint {
                    name: j.name().unwrap_or_default().to_string(),
                    parent,
                    rest_local,
                    rest_world,
                    inverse_bind: inverse_binds[i],
                }
            })
            .collect::<Vec<_>>();

        let node = doc
            .nodes()
            .find(|n| n.skin().is_some() && n.mesh().is_some())
            .context("no skinned mesh node")?;
        let mesh_world = world[node.index()];
        let mesh = node.mesh().unwrap();
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        let mut texture = None;
        for prim in mesh.primitives() {
            let r = prim.reader(|b| Some(&buffers[b.index()]));
            let base = vertices.len() as u32;
            let pos: Vec<[f32; 3]> = r.read_positions().context("no positions")?.collect();
            let nrm: Vec<[f32; 3]> = match r.read_normals() {
                Some(n) => n.collect(),
                None => vec![[0.0, 1.0, 0.0]; pos.len()],
            };
            let uv: Vec<[f32; 2]> = match r.read_tex_coords(0) {
                Some(t) => t.into_f32().collect(),
                None => vec![[0.0; 2]; pos.len()],
            };
            let jn: Vec<[u16; 4]> = r
                .read_joints(0)
                .context("no JOINTS_0")?
                .into_u16()
                .collect();
            let wt: Vec<[f32; 4]> = r
                .read_weights(0)
                .context("no WEIGHTS_0")?
                .into_f32()
                .collect();
            if [nrm.len(), uv.len(), jn.len(), wt.len()]
                .iter()
                .any(|&n| n != pos.len())
            {
                bail!("mismatched vertex attribute counts");
            }
            for i in 0..pos.len() {
                vertices.push(SkinVertex {
                    pos: mesh_world.transform_point3(pos[i].into()),
                    normal: mesh_world
                        .transform_vector3(nrm[i].into())
                        .normalize_or_zero(),
                    uv: uv[i].into(),
                    joints: jn[i],
                    weights: wt[i],
                });
            }
            match r.read_indices() {
                Some(ix) => indices.extend(ix.into_u32().map(|i| base + i)),
                None => indices.extend(base..vertices.len() as u32),
            }
            if texture.is_none() {
                if let Some(t) = prim
                    .material()
                    .pbr_metallic_roughness()
                    .base_color_texture()
                {
                    texture = rgba8(&images[t.texture().source().index()]);
                }
            }
        }
        Ok(ModernModel {
            vertices,
            indices,
            joints,
            texture,
        })
    }

    /// Stop a vertex being pulled by two limbs at once. Automatic skinning bleeds
    /// weight between limbs that touch in the bind pose (a hand hanging beside a
    /// thigh drags the thigh's skin along). Each joint belongs to the limb whose
    /// root joint (`limb_roots`) it descends from; a vertex keeps only the limb
    /// with the most weight on it (plus any joints outside every limb) and is
    /// renormalised, and triangles spanning two limbs are dropped. Returns how
    /// many vertices and triangles changed.
    pub fn separate_limbs(&mut self, limb_roots: &[&str]) -> usize {
        let roots: Vec<usize> = limb_roots.iter().filter_map(|n| self.joint(n)).collect();
        let limb_of: Vec<Option<usize>> = (0..self.joints.len())
            .map(|mut j| loop {
                if let Some(l) = roots.iter().position(|&r| r == j) {
                    break Some(l);
                }
                match self.joints[j].parent {
                    Some(p) => j = p,
                    None => break None,
                }
            })
            .collect();
        let mut changed = 0;
        for v in &mut self.vertices {
            let mut per_limb = vec![0.0f32; roots.len()];
            for k in 0..4 {
                if let Some(l) = limb_of[v.joints[k] as usize] {
                    per_limb[l] += v.weights[k];
                }
            }
            if per_limb.iter().filter(|&&w| w > 0.0).count() < 2 {
                continue;
            }
            let keep = (0..per_limb.len())
                .max_by(|&a, &b| per_limb[a].total_cmp(&per_limb[b]))
                .unwrap();
            for k in 0..4 {
                if limb_of[v.joints[k] as usize].is_some_and(|l| l != keep) {
                    v.weights[k] = 0.0;
                }
            }
            let sum: f32 = v.weights.iter().sum();
            if sum > 0.0 {
                v.weights.iter_mut().for_each(|w| *w /= sum);
            }
            changed += 1;
        }
        // The mesh can be welded across limbs (a hand resting on a thigh shares
        // triangles with it); posed apart, those triangles stretch into a sheet.
        let limb_of_vertex: Vec<Option<usize>> = self
            .vertices
            .iter()
            .map(|v| {
                (0..4)
                    .filter(|&k| v.weights[k] > 0.0)
                    .find_map(|k| limb_of[v.joints[k] as usize])
            })
            .collect();
        let limb_anchor: Vec<Option<usize>> =
            roots.iter().map(|&r| self.joints[r].parent).collect();
        let before = self.indices.len();
        let mut kept = Vec::with_capacity(before);
        for t in self.indices.chunks_exact(3) {
            // Limbs hanging off the same joint (both thighs, both clavicles) meet
            // at the body's centre line on purpose; only others get cut apart.
            let limbs: Vec<usize> = t
                .iter()
                .filter_map(|&i| limb_of_vertex[i as usize])
                .collect();
            let apart = limbs
                .iter()
                .any(|&a| limbs.iter().any(|&b| limb_anchor[a] != limb_anchor[b]));
            if !apart {
                kept.extend_from_slice(t);
            }
        }
        self.indices = kept;
        changed + (before - self.indices.len()) / 3
    }

    pub fn joint(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }
}

pub(crate) fn rgba8(img: &gltf::image::Data) -> Option<Texture> {
    use gltf::image::Format;
    let n = (img.width * img.height) as usize;
    let rgba = match img.format {
        Format::R8G8B8A8 => img.pixels.clone(),
        Format::R8G8B8 => img
            .pixels
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        Format::R8 => img.pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    (rgba.len() == n * 4).then_some(Texture {
        width: img.width,
        height: img.height,
        rgba,
    })
}
