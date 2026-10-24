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
