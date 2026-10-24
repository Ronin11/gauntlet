//! Keyframe animation read from a glTF (Tripo's preset clips), played on the
//! model's own skeleton.

use crate::ModernModel;
use anyhow::{Context, Result};
use glam::{Mat4, Quat, Vec3};
use std::path::Path;

#[derive(Default, Clone)]
struct Track<T> {
    times: Vec<f32>,
    values: Vec<T>,
}

impl<T: Copy> Track<T> {
    /// The keys around `t` and how far between them: (before, after, 0..1).
    fn at(&self, t: f32) -> Option<(T, T, f32)> {
        let n = self.times.len();
        if n == 0 {
            return None;
        }
        let i = self.times.partition_point(|&k| k <= t);
        if i == 0 {
            return Some((self.values[0], self.values[0], 0.0));
        }
        if i >= n {
            return Some((self.values[n - 1], self.values[n - 1], 0.0));
        }
        let (t0, t1) = (self.times[i - 1], self.times[i]);
        Some((
            self.values[i - 1],
            self.values[i],
            (t - t0) / (t1 - t0).max(1e-9),
        ))
    }
}

#[derive(Default, Clone)]
struct JointTracks {
    translation: Track<Vec3>,
    rotation: Track<Quat>,
    scale: Track<Vec3>,
}

pub struct Clip {
    pub duration: f32,
    tracks: Vec<JointTracks>,
    /// Each joint's own transform when the clip doesn't animate it.
    rest: Vec<(Vec3, Quat, Vec3)>,
    /// For joints with no joint parent: what sits above them (an armature node).
    above: Vec<Mat4>,
}

impl Clip {
    /// The first animation of `path`, with channels matched to `model`'s joints
    /// by node name.
    pub fn load(path: impl AsRef<Path>, model: &ModernModel) -> Result<Self> {
        Self::load_named(path, model, None)
    }

    /// The animation called `name` (the first, if `None`).
    pub fn load_named(
        path: impl AsRef<Path>,
        model: &ModernModel,
        name: Option<&str>,
    ) -> Result<Self> {
        let path = path.as_ref();
        let (doc, buffers, _) =
            gltf::import(path).with_context(|| format!("reading {}", path.display()))?;
        let anim = doc
            .animations()
            .find(|a| name.map_or(true, |n| a.name() == Some(n)))
            .with_context(|| format!("no animation {name:?} in {}", path.display()))?;
        let by_node: std::collections::HashMap<usize, usize> = doc
            .nodes()
            .filter_map(|n| Some((n.index(), model.joint(n.name()?)?)))
            .collect();
        let mut rest = vec![(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE); model.joints.len()];
        let mut above = vec![Mat4::IDENTITY; model.joints.len()];
        for n in doc.nodes() {
            if let Some(&j) = by_node.get(&n.index()) {
                let (t, r, s) = n.transform().decomposed();
                rest[j] = (t.into(), Quat::from_array(r), s.into());
                if model.joints[j].parent.is_none() {
                    let local =
                        Mat4::from_scale_rotation_translation(rest[j].2, rest[j].1, rest[j].0);
                    above[j] = model.joints[j].rest_world * local.inverse();
                }
            }
        }
        let mut tracks = vec![JointTracks::default(); model.joints.len()];
        let mut duration = 0.0f32;
        for ch in anim.channels() {
            let Some(&j) = by_node.get(&ch.target().node().index()) else {
                continue;
            };
            let r = ch.reader(|b| Some(&buffers[b.index()]));
            let times: Vec<f32> = r.read_inputs().context("no keyframe times")?.collect();
            duration = duration.max(times.last().copied().unwrap_or(0.0));
            match r.read_outputs().context("no keyframe values")? {
                gltf::animation::util::ReadOutputs::Translations(it) => {
                    tracks[j].translation = Track {
                        times,
                        values: it.map(Vec3::from).collect(),
                    }
                }
                gltf::animation::util::ReadOutputs::Rotations(it) => {
                    tracks[j].rotation = Track {
                        times,
                        values: it.into_f32().map(Quat::from_array).collect(),
                    }
                }
                gltf::animation::util::ReadOutputs::Scales(it) => {
                    tracks[j].scale = Track {
                        times,
                        values: it.map(Vec3::from).collect(),
                    }
                }
                _ => {}
            }
        }
        Ok(Clip {
            duration,
            tracks,
            rest,
            above,
        })
    }

    /// Skin matrices (joint pose x inverse bind) at `time` seconds, looping.
    pub fn skin_matrices(&self, model: &ModernModel, time: f32) -> Vec<Mat4> {
        let t = if self.duration > 0.0 {
            time.rem_euclid(self.duration)
        } else {
            0.0
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
        let mut world = vec![Mat4::IDENTITY; model.joints.len()];
        for &j in &order {
            let tr = &self.tracks[j];
            let (rt, rr, rs) = self.rest[j];
            let pos = tr.translation.at(t).map_or(rt, |(a, b, f)| a.lerp(b, f));
            let rot = tr.rotation.at(t).map_or(rr, |(a, b, f)| a.slerp(b, f));
            let scl = tr.scale.at(t).map_or(rs, |(a, b, f)| a.lerp(b, f));
            let local = Mat4::from_scale_rotation_translation(scl, rot, pos);
            world[j] = match model.joints[j].parent {
                Some(p) => world[p] * local,
                None => self.above[j] * local,
            };
        }
        world
            .iter()
            .zip(&model.joints)
            .map(|(w, j)| *w * j.inverse_bind)
            .collect()
    }
}
