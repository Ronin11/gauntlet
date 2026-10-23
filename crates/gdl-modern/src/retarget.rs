//! Drives a modern skeleton from the game's rigid-part animation.
//!
//! The game's heroes are posed by a handful of nodes; a modern rig has dozens of
//! joints, different proportions and a different rest pose (the game's legs are
//! bent at rest, a modern rig's are straight), so the animations only translate
//! loosely. Two rules turn a game pose into a modern one:
//!
//! * A bone with an *aim* node (a thigh aims at the knee) points the way the game's
//!   bone points: the shortest turn from its bind direction to the game's current
//!   one. This is absolute, so differing rest poses don't matter; twist stays as
//!   in the bind pose.
//! * Any other mapped joint (pelvis, hands, feet, head) gets the rotation its game
//!   node made since a reference pose, beyond what its parent made, on top of
//!   what its modern parent did. The pelvis also follows the game's movement,
//!   scaled by the height ratio.
//!
//! Joints with no mapping follow their parents.

use crate::ModernModel;
use anyhow::{bail, Result};
use glam::{Mat3, Mat4, Quat, Vec3};

struct Pair {
    node: usize,
    joint: usize,
    /// Game node and joint this bone points at.
    aim: Option<(usize, usize)>,
    /// Index into `pairs` of the nearest mapped ancestor joint.
    parent: Option<usize>,
}

pub struct Retarget {
    pairs: Vec<Pair>,
    /// Modern space to game space: carries the modern rig's forward, up and left
    /// onto the game's (+z, +y and the side its left hip is on). Rigs can face
    /// any way, and the game's space is a mirror image of a standard one, so this
    /// can be a rotation, a reflection, or both.
    basis: Mat3,
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

/// The unit vector along `v` with its height removed.
fn level(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z).normalize_or_zero()
}

impl Retarget {
    /// `game_nodes` / `game_parents`: the skeleton's node names and parents;
    /// `game_ref`: its model-space matrices in the reference pose (used to tell
    /// left from right); `map` and `aim`: see [`crate::mapping`]; `forward`: two
    /// modern joints (heel, toe) that point the way the rig faces.
    pub fn new(
        model: &ModernModel,
        game_nodes: &[&str],
        game_ref: &[Mat4],
        map: &[(&str, &str)],
        aim: &[(&str, &str)],
        forward: (&str, &str),
    ) -> Result<Self> {
        let node_of = |name: &str| game_nodes.iter().position(|g| g.eq_ignore_ascii_case(name));
        let mut pairs = Vec::new();
        let mut missing = Vec::new();
        for &(node, joint) in map {
            match (node_of(node), model.joint(joint)) {
                (Some(node), Some(joint)) => pairs.push(Pair {
                    node,
                    joint,
                    aim: None,
                    parent: None,
                }),
                _ => missing.push(format!("{node} -> {joint}")),
            }
        }
        if !missing.is_empty() {
            bail!("unmapped: {}", missing.join(", "));
        }
        let joint_of_node = |n: usize| pairs.iter().find(|p| p.node == n).map(|p| p.joint);
        let aims: Vec<Option<(usize, usize)>> = pairs
            .iter()
            .map(|p| {
                let (_, target) = aim.iter().find(|(from, _)| node_of(from) == Some(p.node))?;
                let tn = node_of(target)?;
                Some((tn, joint_of_node(tn)?))
            })
            .collect();
        let parents: Vec<Option<usize>> = pairs
            .iter()
            .map(|p| {
                let mut j = model.joints[p.joint].parent;
                while let Some(a) = j {
                    if let Some(i) = pairs.iter().position(|q| q.joint == a) {
                        return Some(i);
                    }
                    j = model.joints[a].parent;
                }
                None
            })
            .collect();
        for (i, p) in pairs.iter_mut().enumerate() {
            p.aim = aims[i];
            p.parent = parents[i];
        }
        let find = |name: &str| pairs.iter().find(|p| game_nodes[p.node] == name);
        let root = find("ROOT_PELVIS").map(|p| p.node).unwrap_or(pairs[0].node);
        let pos = |name: &str| {
            model
                .joint(name)
                .map(|j| model.joints[j].rest_world.w_axis.truncate())
        };
        let basis = match (find("L_HIP"), find("R_HIP"), pos(forward.0), pos(forward.1)) {
            (Some(l), Some(r), Some(heel), Some(toe)) => {
                let left_m = level(
                    model.joints[l.joint].rest_world.w_axis.truncate()
                        - model.joints[r.joint].rest_world.w_axis.truncate(),
                );
                // Forward is square to the hips (the toes can point elsewhere, and
                // a rig made from a three-quarter view stands turned); the toes
                // only say front from back.
                let mut fwd_m = left_m.cross(Vec3::Y);
                if fwd_m.dot(level(toe - heel)) < 0.0 {
                    fwd_m = -fwd_m;
                }
                // The game faces +z with up +y; only which side is left varies (its
                // reference stance can have the hips twisted, so don't copy that).
                let side = (game_ref[l.node].w_axis.x - game_ref[r.node].w_axis.x).signum();
                let left_g = Vec3::X * side;
                if left_m == Vec3::ZERO || fwd_m == Vec3::ZERO || left_g == Vec3::ZERO {
                    bail!("can't tell which way the rig faces");
                }
                let modern = Mat3::from_cols(fwd_m, Vec3::Y, left_m);
                let game = Mat3::from_cols(Vec3::Z, Vec3::Y, left_g);
                game * modern.transpose()
            }
            _ => bail!("need L_HIP, R_HIP and the forward joints to orient the rig"),
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
            basis,
            root,
            order,
        })
    }

    /// A point or direction of the modern model, in game space (unscaled).
    pub fn to_game(&self, v: Vec3) -> Vec3 {
        self.basis * v
    }

    /// Whether going to game space turns the model inside out (flips winding).
    pub fn mirrors(&self) -> bool {
        self.basis.determinant() < 0.0
    }

    /// Game-space scale of the modern model: the game pelvis' height over the
    /// modern hip's.
    pub fn scale(&self, model: &ModernModel, game_ref: &[Mat4]) -> f32 {
        let p = self.pairs.iter().find(|p| p.node == self.root).unwrap();
        let hip = model.joints[p.joint].rest_world.w_axis.y;
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
        let to_modern = self.basis.transpose();
        let refl = |v: Vec3| to_modern * v;
        // The rotation each game node made since the reference pose, in modern space.
        let delta = |n: usize| {
            let d = rotation(&game_now[n]) * rotation(&game_ref[n]).inverse();
            Quat::from_mat3(&(to_modern * Mat3::from_quat(d) * self.basis)).normalize()
        };
        let by_joint: Vec<Option<usize>> = {
            let mut v = vec![None; model.joints.len()];
            for (i, p) in self.pairs.iter().enumerate() {
                v[p.joint] = Some(i);
            }
            v
        };
        let mut world = vec![Mat4::IDENTITY; model.joints.len()];
        for &j in &self.order {
            let joint = &model.joints[j];
            let parent = joint.parent.map_or(Mat4::IDENTITY, |p| world[p]);
            let Some(pi) = by_joint[j] else {
                world[j] = parent * joint.rest_local;
                continue;
            };
            let pair = &self.pairs[pi];
            let rest_rot = rotation(&joint.rest_world);
            let mut t = parent.transform_point3(joint.rest_local.w_axis.truncate());
            let rot = if let Some((tn, tj)) = pair.aim {
                let want = refl((game_now[tn].w_axis - game_now[pair.node].w_axis).truncate());
                let bind = model.joints[tj].rest_world.w_axis.truncate()
                    - joint.rest_world.w_axis.truncate();
                if want.length() > 1e-6 && bind.length() > 1e-6 {
                    Quat::from_rotation_arc(bind.normalize(), want.normalize()) * rest_rot
                } else {
                    rest_rot
                }
            } else if let Some(pp) = pair.parent {
                let parent_pair = &self.pairs[pp];
                let extra = delta(pair.node) * delta(parent_pair.node).inverse();
                let parent_rest = rotation(&model.joints[parent_pair.joint].rest_world);
                let parent_did = rotation(&world[parent_pair.joint]) * parent_rest.inverse();
                extra * parent_did * rest_rot
            } else {
                delta(pair.node) * rest_rot
            };
            if pair.node == self.root {
                t += refl((game_now[pair.node].w_axis - game_ref[pair.node].w_axis).truncate())
                    / scale;
            }
            world[j] = Mat4::from_rotation_translation(rot, t);
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
