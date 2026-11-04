//! A rigid, unskinned glTF mesh (a weapon) that rides on one joint of a skinned
//! model, the way the game's own heroes carry theirs as separate parts.

use crate::Texture;
use anyhow::{Context, Result};
use glam::{EulerRot, Mat4, Quat, Vec2, Vec3};
use std::path::Path;

pub struct Prop {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub uvs: Vec<Vec2>,
    pub indices: Vec<u32>,
    pub texture: Option<Texture>,
}

/// Where a prop sits relative to the joint it is held by.
#[derive(Clone, Copy, Debug)]
pub struct Mount {
    /// Length of the prop's longest side, in the model's units.
    pub length: f32,
    /// Where along that side the hand holds it, from the start (0) to the end (1).
    pub grip: f32,
    /// Turn of the prop in the joint's own rest frame, degrees (x, y, z).
    pub rotate: Vec3,
    /// Shift in the joint's rest frame.
    pub offset: Vec3,
    /// Where the far end points, as a direction in the model's own space (the
    /// joint's bind pose is turned to suit); overrides `rotate` when set.
    pub aim: Option<Vec3>,
    /// Turn about that aim, degrees (which way a blade's flat faces).
    pub twist: f32,
}

impl Prop {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let (doc, buffers, images) =
            gltf::import(path).with_context(|| format!("reading {}", path.display()))?;
        let mut world = vec![Mat4::IDENTITY; doc.nodes().len()];
        fn walk(n: gltf::Node, up: Mat4, world: &mut [Mat4]) {
            let w = up * Mat4::from_cols_array_2d(&n.transform().matrix());
            world[n.index()] = w;
            for c in n.children() {
                walk(c, w, world);
            }
        }
        for scene in doc.scenes() {
            for n in scene.nodes() {
                walk(n, Mat4::IDENTITY, &mut world);
            }
        }
        let mut prop = Prop {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            texture: None,
        };
        for node in doc.nodes() {
            let Some(mesh) = node.mesh() else { continue };
            let m = world[node.index()];
            for prim in mesh.primitives() {
                let r = prim.reader(|b| Some(&buffers[b.index()]));
                let base = prop.positions.len() as u32;
                let pos: Vec<[f32; 3]> = r.read_positions().context("no positions")?.collect();
                let nrm: Vec<[f32; 3]> = match r.read_normals() {
                    Some(n) => n.collect(),
                    None => vec![[0.0, 1.0, 0.0]; pos.len()],
                };
                let uv: Vec<[f32; 2]> = match r.read_tex_coords(0) {
                    Some(t) => t.into_f32().collect(),
                    None => vec![[0.0; 2]; pos.len()],
                };
                for i in 0..pos.len() {
                    prop.positions.push(m.transform_point3(pos[i].into()));
                    prop.normals
                        .push(m.transform_vector3(nrm[i].into()).normalize_or_zero());
                    prop.uvs.push(uv[i].into());
                }
                match r.read_indices() {
                    Some(ix) => prop.indices.extend(ix.into_u32().map(|i| base + i)),
                    None => prop.indices.extend(base..prop.positions.len() as u32),
                }
                if prop.texture.is_none() {
                    if let Some(t) = prim
                        .material()
                        .pbr_metallic_roughness()
                        .base_color_texture()
                    {
                        prop.texture = crate::rgba8(&images[t.texture().source().index()]);
                    }
                }
            }
        }
        Ok(prop)
    }

    /// The prop's own transform for `mount`: the long side scaled to `length`,
    /// the grip point at the origin, then turned and shifted. The result is in
    /// the frame of the joint holding it.
    pub fn placement(&self, mount: &Mount, joint_rest: Quat) -> Mat4 {
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &self.positions {
            lo = lo.min(*p);
            hi = hi.max(*p);
        }
        let size = hi - lo;
        let axis = if size.x >= size.y && size.x >= size.z {
            0
        } else if size.y >= size.z {
            1
        } else {
            2
        };
        let scale = mount.length / size[axis].max(1e-6);
        let mut grip = (lo + hi) * 0.5;
        grip[axis] = lo[axis] + size[axis] * mount.grip;
        let turn = match mount.aim {
            Some(aim) => {
                // The far end's direction in the prop, brought to the joint's frame.
                let mut tip = Vec3::ZERO;
                tip[axis] = if mount.grip < 0.5 { 1.0 } else { -1.0 };
                let want = joint_rest.inverse() * aim.normalize();
                Quat::from_axis_angle(want, mount.twist.to_radians())
                    * Quat::from_rotation_arc(tip, want)
            }
            None => Quat::from_euler(
                EulerRot::XYZ,
                mount.rotate.x.to_radians(),
                mount.rotate.y.to_radians(),
                mount.rotate.z.to_radians(),
            ),
        };
        Mat4::from_translation(mount.offset)
            * Mat4::from_quat(turn)
            * Mat4::from_scale(Vec3::splat(scale))
            * Mat4::from_translation(-grip)
    }
}

/// A prop whose pieces move: a glTF whose nodes each carry a piece and whose
/// one animation moves them (a barrel's break-apart sequence). Read once,
/// posed per game frame with [`AnimProp::pose`].
pub struct AnimProp {
    nodes: Vec<AnimNode>,
    pub texture: Option<Texture>,
}

struct AnimNode {
    parent: Option<usize>,
    rest: (Vec3, Quat, Vec3),
    translation: Track<Vec3>,
    rotation: Track<Quat>,
    scale: Track<Vec3>,
    /// The node's mesh in its own frame: positions, normals, uvs, indices.
    mesh: Option<(Vec<Vec3>, Vec<Vec3>, Vec<Vec2>, Vec<u32>)>,
}

#[derive(Clone, Default)]
struct Track<T> {
    times: Vec<f32>,
    values: Vec<T>,
}

impl<T: Copy> Track<T> {
    /// The value at `t`, `lerp`ed between the keyframes around it.
    fn at(&self, t: f32, lerp: impl Fn(T, T, f32) -> T) -> Option<T> {
        let (first, last) = (*self.times.first()?, *self.times.last()?);
        if t <= first || self.times.len() == 1 {
            return self.values.first().copied();
        }
        if t >= last {
            return self.values.last().copied();
        }
        let i = self.times.partition_point(|&k| k <= t) - 1;
        let (t0, t1) = (self.times[i], self.times[i + 1]);
        let w = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
        Some(lerp(self.values[i], self.values[i + 1], w))
    }
}

impl AnimProp {
    /// Keyframe spacing the pieces are sampled at: frame `k` is `k / 30` s.
    pub const FPS: f32 = 30.0;

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let (doc, buffers, images) =
            gltf::import(path).with_context(|| format!("reading {}", path.display()))?;
        let mut parent = vec![None; doc.nodes().len()];
        for n in doc.nodes() {
            for c in n.children() {
                parent[c.index()] = Some(n.index());
            }
        }
        let mut texture = None;
        let mut nodes = Vec::new();
        for n in doc.nodes() {
            let (t, r, s) = n.transform().decomposed();
            let mut mesh = None;
            if let Some(m) = n.mesh() {
                let (mut pos, mut nrm, mut uv, mut idx) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
                for prim in m.primitives() {
                    let rd = prim.reader(|b| Some(&buffers[b.index()]));
                    let base = pos.len() as u32;
                    let p: Vec<[f32; 3]> = rd.read_positions().context("no positions")?.collect();
                    let count = p.len();
                    pos.extend(p.into_iter().map(Vec3::from));
                    match rd.read_normals() {
                        Some(ns) => nrm.extend(ns.map(Vec3::from)),
                        None => nrm.extend(std::iter::repeat_n(Vec3::Y, count)),
                    }
                    match rd.read_tex_coords(0) {
                        Some(ts) => uv.extend(ts.into_f32().map(Vec2::from)),
                        None => uv.extend(std::iter::repeat_n(Vec2::ZERO, count)),
                    }
                    match rd.read_indices() {
                        Some(ix) => idx.extend(ix.into_u32().map(|i| base + i)),
                        None => idx.extend(base..base + count as u32),
                    }
                    if texture.is_none() {
                        if let Some(t) = prim
                            .material()
                            .pbr_metallic_roughness()
                            .base_color_texture()
                        {
                            texture = crate::rgba8(&images[t.texture().source().index()]);
                        }
                    }
                }
                mesh = Some((pos, nrm, uv, idx));
            }
            nodes.push(AnimNode {
                parent: parent[n.index()],
                rest: (t.into(), Quat::from_array(r), s.into()),
                translation: Track::default(),
                rotation: Track::default(),
                scale: Track::default(),
                mesh,
            });
        }
        // Every animation's channels count (an exporter may write one per
        // piece); a node's later channels replace its earlier ones.
        for anim in doc.animations() {
            for ch in anim.channels() {
                let node = &mut nodes[ch.target().node().index()];
                let rd = ch.reader(|b| Some(&buffers[b.index()]));
                let times: Vec<f32> = rd.read_inputs().context("no keyframe times")?.collect();
                match rd.read_outputs().context("no keyframe values")? {
                    gltf::animation::util::ReadOutputs::Translations(it) => {
                        node.translation = Track {
                            times,
                            values: it.map(Vec3::from).collect(),
                        }
                    }
                    gltf::animation::util::ReadOutputs::Rotations(it) => {
                        node.rotation = Track {
                            times,
                            values: it.into_f32().map(Quat::from_array).collect(),
                        }
                    }
                    gltf::animation::util::ReadOutputs::Scales(it) => {
                        node.scale = Track {
                            times,
                            values: it.map(Vec3::from).collect(),
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(AnimProp { nodes, texture })
    }

    /// The pieces as one rigid mesh, posed at animation frame `frame`
    /// (`frame / FPS` seconds in).
    pub fn pose(&self, frame: usize) -> Prop {
        let t = frame as f32 / Self::FPS;
        let local: Vec<Mat4> = self
            .nodes
            .iter()
            .map(|n| {
                let tr = n.translation.at(t, |a, b, w| a.lerp(b, w)).unwrap_or(n.rest.0);
                let ro = n.rotation.at(t, |a, b, w| a.slerp(b, w)).unwrap_or(n.rest.1);
                let sc = n.scale.at(t, |a, b, w| a.lerp(b, w)).unwrap_or(n.rest.2);
                Mat4::from_scale_rotation_translation(sc, ro, tr)
            })
            .collect();
        let mut world = vec![None; self.nodes.len()];
        fn resolve(i: usize, nodes: &[AnimNode], local: &[Mat4], world: &mut [Option<Mat4>]) -> Mat4 {
            if let Some(w) = world[i] {
                return w;
            }
            let w = match nodes[i].parent {
                Some(p) => resolve(p, nodes, local, world) * local[i],
                None => local[i],
            };
            world[i] = Some(w);
            w
        }
        let mut prop = Prop {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            texture: self.texture.clone(),
        };
        for i in 0..self.nodes.len() {
            let Some((pos, nrm, uv, idx)) = &self.nodes[i].mesh else {
                continue;
            };
            let m = resolve(i, &self.nodes, &local, &mut world);
            let nm = m.inverse().transpose();
            let base = prop.positions.len() as u32;
            prop.positions
                .extend(pos.iter().map(|&p| m.transform_point3(p)));
            prop.normals
                .extend(nrm.iter().map(|&n| nm.transform_vector3(n).normalize_or_zero()));
            prop.uvs.extend_from_slice(uv);
            prop.indices.extend(idx.iter().map(|&i| base + i));
        }
        prop
    }
}
