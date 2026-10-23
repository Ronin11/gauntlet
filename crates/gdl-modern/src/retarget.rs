//! Drives a modern skeleton from the game's rigid-part animation.
//!
//! The game's heroes are posed by a handful of nodes; a modern rig has dozens of
//! joints and different proportions, so the animations only translate loosely.
//! Each mapped joint gets the *change of rotation* its game node made since a
//! reference pose, applied on top of the joint's own bind rotation. Bone lengths
//! stay the modern rig's own; only the pelvis follows the game's movement
//! (scaled by the height ratio). Joints with no mapping follow their parents.

use crate::ModernModel;
use anyhow::{bail, Result};
use glam::{Mat4, Quat, Vec3};

pub struct Retarget {
    /// (game node index, joint index).
    pairs: Vec<(usize, usize)>,
    /// The game skeleton is a mirror image of the modern rig (left on -x while
    /// facing +z), so rotation changes are reflected across x.
    reflect_x: bool,
    /// The game node that moves the whole body (`ROOT_PELVIS`).
    root: usize,
    /// Joints in an order where parents come first.
    order: Vec<usize>,
}

/// The rotation of a model-space matrix, scale stripped.
fn rotation(m: &Mat4) -> Quat {
    let x = m.x_axis.truncate().normalize_or_zero();
    let y = m.y_axis.truncate().normalize_or_zero();
    let z = m.z_axis.truncate().normalize_or_zero();
    Quat::from_mat3(&glam::Mat3::from_cols(x, y, z)).normalize()
}

/// `M q M` for the reflection M = diag(-1, 1, 1).
fn reflect(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

impl Retarget {
    /// `game_nodes`: the skeleton's node names; `game_ref`: its model-space
    /// matrices in the rest pose (used to tell left from right); `map`: see
    /// [`crate::mapping`].
    pub fn new(
        model: &ModernModel,
        game_nodes: &[&str],
        game_ref: &[Mat4],
        map: &[(&str, &str)],
    ) -> Result<Self> {
        let mut pairs = Vec::new();
        let mut missing = Vec::new();
        for &(node, joint) in map {
            let n = game_nodes.iter().position(|g| g.eq_ignore_ascii_case(node));
            let j = model.joint(joint);
            match (n, j) {
                (Some(n), Some(j)) => pairs.push((n, j)),
                _ => missing.push(format!("{node} -> {joint}")),
            }
        }
        if !missing.is_empty() {
            bail!("unmapped: {}", missing.join(", "));
        }
        let find = |name: &str| pairs.iter().find(|&&(n, _)| game_nodes[n] == name).copied();
        let root = find("ROOT_PELVIS").map(|(n, _)| n).unwrap_or(pairs[0].0);
        // Left is on -x in the game; check the modern rig puts it on +x.
        let reflect_x = match (find("L_HIP"), find("R_HIP")) {
            (Some((gl, jl)), Some((gr, jr))) => {
                let game = game_ref[gl].w_axis.x - game_ref[gr].w_axis.x;
                let modern =
                    model.joints[jl].rest_world.w_axis.x - model.joints[jr].rest_world.w_axis.x;
                game.signum() != modern.signum()
            }
            _ => false,
        };
        let mut order: Vec<usize> = (0..model.joints.len()).collect();
        let depth = |mut j: usize| {
            let mut d = 0;
            while let Some(p) = model.joints[j].parent {
                j = p;
                d += 1;
            }
            d
        };
        order.sort_by_key(|&j| depth(j));
        Ok(Retarget {
            pairs,
            reflect_x,
            root,
            order,
        })
    }

    /// Game-space scale of the modern model: the game pelvis' height over the
    /// modern hip's.
    pub fn scale(&self, model: &ModernModel, game_ref: &[Mat4]) -> f32 {
        let (_, j) = self
            .pairs
            .iter()
            .find(|&&(n, _)| n == self.root)
            .copied()
            .unwrap();
        let hip = model.joints[j].rest_world.w_axis.y;
        if hip.abs() < 1e-6 {
            1.0
        } else {
            game_ref[self.root].w_axis.y / hip
        }
    }

    /// Skin matrices (joint world pose x inverse bind) for the game pose
    /// `game_now`, measured against `game_ref`.
    pub fn skin_matrices(
        &self,
        model: &ModernModel,
        game_ref: &[Mat4],
        game_now: &[Mat4],
    ) -> Vec<Mat4> {
        let scale = self.scale(model, game_ref);
        let mut target: Vec<Option<(usize, Quat)>> = vec![None; model.joints.len()];
        for &(n, j) in &self.pairs {
            let mut d = rotation(&game_now[n]) * rotation(&game_ref[n]).inverse();
            if self.reflect_x {
                d = reflect(d);
            }
            target[j] = Some((n, d));
        }
        let mut world = vec![Mat4::IDENTITY; model.joints.len()];
        for &j in &self.order {
            let joint = &model.joints[j];
            let parent = joint.parent.map_or(Mat4::IDENTITY, |p| world[p]);
            world[j] = match target[j] {
                None => parent * joint.rest_local,
                Some((n, d)) => {
                    let mut t = parent.transform_point3(joint.rest_local.w_axis.truncate());
                    if n == self.root {
                        let mut dp = (game_now[n].w_axis - game_ref[n].w_axis).truncate() / scale;
                        if self.reflect_x {
                            dp.x = -dp.x;
                        }
                        t += dp;
                    }
                    Mat4::from_rotation_translation(d * rotation(&joint.rest_world), t)
                }
            };
        }
        world
            .iter()
            .zip(&model.joints)
            .map(|(w, j)| *w * j.inverse_bind)
            .collect()
    }
}

/// Linear-blend skinning: each vertex's posed position and normal.
pub fn skin(model: &ModernModel, mats: &[Mat4]) -> Vec<(Vec3, Vec3)> {
    model
        .vertices
        .iter()
        .map(|v| {
            let mut m = Mat4::ZERO;
            for k in 0..4 {
                m += mats[v.joints[k] as usize] * v.weights[k];
            }
            (
                m.transform_point3(v.pos),
                m.transform_vector3(v.normal).normalize_or_zero(),
            )
        })
        .collect()
}
