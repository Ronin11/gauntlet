//! Render interpolation between game ticks.
//!
//! The simulation runs at a fixed 60 Hz (every original timing is in ticks);
//! a faster or uneven display draws each frame between the last two ticks'
//! poses, `alpha = acc / TICK` of the way from the previous to the current
//! one. [`crate::scene::Scene`] keeps a [`Pose`] per instance for both ticks
//! and the camera, and [`Interp`] decides what blends and what snaps:
//!
//! - anything new this tick (spawned, or the scene was rebuilt) snaps;
//! - a body that moved further than [`SNAP_DIST`] or turned more than
//!   [`SNAP_ANGLE`] in one tick (teleport, transporter, respawn) snaps;
//! - a skeleton whose model or sequence changed, or whose sample time went
//!   backwards (a wrap or restart), snaps its parts but still blends the body;
//! - a camera cut (more than [`CAM_SNAP_DIST`], or a new near plane) snaps;
//! - while the game is not ticking (menus, a scroll, the screen saver) the
//!   current tick is drawn as it is.
//!
//! Headless snapshots never blend: they draw the exact tick state.

use glam::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Further than this in one tick (game units; a fast missile moves ~2) is a teleport.
pub const SNAP_DIST: f32 = 8.0;
/// More than this turn in one tick (radians) is a cut, not a turn.
pub const SNAP_ANGLE: f32 = 100.0 * std::f32::consts::PI / 180.0;
/// A camera that jumps this far in one tick has cut.
pub const CAM_SNAP_DIST: f32 = 10.0;
/// With no new tick for this long the game is stopped: draw the last tick.
const STALL: Duration = Duration::from_millis(50);

/// Camera eye, target and near plane (as `Game::camera_view`).
pub type Cam = (Vec3, Vec3, f32);

/// One instance's drawn state at a tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub transform: Mat4,
    pub parts: Vec<Mat4>,
    /// What the parts are a sample of: model and sequence (0: unknown).
    pub tag: u64,
    /// The sequence's sample time.
    pub frame: f32,
}

/// The tag for model `model` playing sequence `seq`.
pub fn tag(model: usize, seq: usize) -> u64 {
    1 << 63 | (model as u64) << 32 | seq as u64
}

fn finite(v: Vec3) -> bool {
    v.is_finite()
}

/// `a` to `b` at `t`, or None to snap to `b`: translation and scale lerp,
/// rotation slerps the shortest way. A matrix that isn't scale-rotation-
/// translation (shear from uneven scales) lerps element-wise instead.
pub fn blend_mat(a: &Mat4, b: &Mat4, t: f32) -> Option<Mat4> {
    if a == b {
        return Some(*b);
    }
    let (sa, ra, ta) = a.to_scale_rotation_translation();
    let (sb, rb, tb) = b.to_scale_rotation_translation();
    if !(finite(ta) && finite(tb)) || ta.distance_squared(tb) > SNAP_DIST * SNAP_DIST {
        return None;
    }
    if !(finite(sa) && finite(sb) && ra.is_finite() && rb.is_finite())
        || sa.x * sb.x < 0.0
        || sa.y * sb.y < 0.0
        || sa.z * sb.z < 0.0
    {
        // A degenerate (zero-scale) or flipped matrix: no rotation to blend.
        return Some(lerp_mat(a, b, t));
    }
    let ra = if ra.dot(rb) < 0.0 { -ra } else { ra };
    if ra.angle_between(rb) > SNAP_ANGLE {
        return None;
    }
    // Only trust the decomposition if it rebuilds the matrices.
    let tol = 1e-3
        * a.abs()
            .to_cols_array()
            .iter()
            .fold(1.0f32, |m, v| m.max(*v));
    let same =
        |m: &Mat4, s, r, tr| Mat4::from_scale_rotation_translation(s, r, tr).abs_diff_eq(*m, tol);
    if !same(a, sa, ra, ta) || !same(b, sb, rb, tb) {
        return Some(lerp_mat(a, b, t));
    }
    Some(Mat4::from_scale_rotation_translation(
        sa.lerp(sb, t),
        slerp(ra, rb, t),
        ta.lerp(tb, t),
    ))
}

/// Element-wise matrix lerp.
pub fn lerp_mat(a: &Mat4, b: &Mat4, t: f32) -> Mat4 {
    *a * (1.0 - t) + *b * t
}

/// Shortest-way quaternion slerp.
pub fn slerp(a: Quat, b: Quat, t: f32) -> Quat {
    let a = if a.dot(b) < 0.0 { -a } else { a };
    a.slerp(b, t).normalize()
}

/// Yaw `a` to `b` (radians) the shortest way round.
pub fn lerp_yaw(a: f32, b: f32, t: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let d = (b - a).rem_euclid(tau);
    let d = if d > std::f32::consts::PI { d - tau } else { d };
    a + d * t
}

/// Whether a skeleton's parts can blend from `a` to `b`: the same model and
/// sequence, sampled forwards, the same part count.
pub fn parts_blend(a: &Pose, b: &Pose) -> bool {
    a.parts.len() == b.parts.len() && a.tag == b.tag && b.frame >= a.frame
}

/// `prev` to `cur` at `t` into `transform` and `parts` (which hold `cur`).
/// Returns whether anything blended.
pub fn blend_pose(
    prev: &Pose,
    cur: &Pose,
    t: f32,
    transform: &mut Mat4,
    parts: &mut [Mat4],
) -> bool {
    let Some(m) = blend_mat(&prev.transform, &cur.transform, t) else {
        return false;
    };
    *transform = m;
    if parts_blend(prev, cur) && parts.len() == cur.parts.len() {
        for ((p, a), b) in parts.iter_mut().zip(&prev.parts).zip(&cur.parts) {
            if let Some(m) = blend_mat(a, b, t) {
                *p = m;
            }
        }
    }
    true
}

/// The camera from `a` to `b` at `t`, or `b` across a cut.
pub fn blend_cam(a: Cam, b: Cam, t: f32) -> Cam {
    let cut = a.0.distance(b.0) > CAM_SNAP_DIST
        || a.1.distance(b.1) > CAM_SNAP_DIST
        || a.2 != b.2
        || !(a.0.is_finite() && a.1.is_finite());
    if cut {
        b
    } else {
        (a.0.lerp(b.0, t), a.1.lerp(b.1, t), b.2)
    }
}

/// The poses of the last two ticks.
#[derive(Default)]
pub struct Interp {
    /// The game tick `cur` is of.
    tick: Option<u32>,
    pub prev: HashMap<u64, Pose>,
    pub cur: HashMap<u64, Pose>,
    prev_cam: Option<Cam>,
    cur_cam: Option<Cam>,
    /// When `cur` was taken, and the last blend's tick and alpha.
    at: Option<Instant>,
    last: Option<(u32, f32)>,
    stalled: bool,
}

impl Interp {
    /// Whether tick `frame` still has to be captured.
    pub fn needs(&self, frame: u32) -> bool {
        self.tick != Some(frame)
    }

    /// Start capturing tick `frame`: the current poses become the previous
    /// ones if it is the very next tick, otherwise there is nothing to blend from.
    pub fn begin(&mut self, frame: u32, cam: Cam) {
        let next = self.tick.map(|t| frame.wrapping_sub(t)) == Some(1);
        self.prev = std::mem::take(&mut self.cur);
        self.prev_cam = self.cur_cam.take();
        if !next {
            self.prev.clear();
            self.prev_cam = None;
        }
        self.cur_cam = Some(cam);
        self.tick = Some(frame);
        self.at = Some(Instant::now());
        self.stalled = false;
    }

    pub fn put(&mut self, key: u64, pose: Pose) {
        self.cur.insert(key, pose);
    }

    /// The fraction to draw at for display `alpha` (`acc / TICK`) at game
    /// tick `frame`: 1 (the current tick) once a tick has gone by without
    /// the game advancing.
    pub fn alpha(&mut self, frame: u32, alpha: f32) -> f32 {
        let alpha = alpha.clamp(0.0, 1.0);
        if let Some((f, a)) = self.last {
            // The accumulator wrapped but the game didn't move: it is stopped.
            if f == frame && alpha < a {
                self.stalled = true;
            }
        }
        if self.at.is_some_and(|t| t.elapsed() > STALL) {
            self.stalled = true;
        }
        self.last = Some((frame, alpha));
        if self.stalled || self.tick != Some(frame) {
            1.0
        } else {
            alpha
        }
    }

    /// This frame's camera at `t`, if there are two ticks to blend.
    pub fn cam(&self, t: f32) -> Option<Cam> {
        match (self.prev_cam, self.cur_cam) {
            (Some(a), Some(b)) if t < 1.0 => Some(blend_cam(a, b, t)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xf(pos: Vec3, yaw: f32) -> Mat4 {
        Mat4::from_translation(pos) * Mat4::from_rotation_y(yaw)
    }

    fn pose(pos: Vec3, yaw: f32, tag: u64, frame: f32) -> Pose {
        Pose {
            transform: xf(pos, yaw),
            parts: vec![xf(Vec3::Y, yaw)],
            tag,
            frame,
        }
    }

    #[test]
    fn blends_translation_and_rotation() {
        let a = xf(Vec3::ZERO, 0.0);
        let b = xf(Vec3::new(2.0, 0.0, 0.0), 1.0);
        let m = blend_mat(&a, &b, 0.5).unwrap();
        assert!(m.abs_diff_eq(xf(Vec3::new(1.0, 0.0, 0.0), 0.5), 1e-5));
        assert!(blend_mat(&a, &b, 0.0).unwrap().abs_diff_eq(a, 1e-5));
        assert!(blend_mat(&a, &b, 1.0).unwrap().abs_diff_eq(b, 1e-5));
    }

    #[test]
    fn blends_through_the_mirror() {
        // Scene transforms are mirrored (negative determinant).
        let m = Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        let a = m * xf(Vec3::ZERO, 0.2);
        let b = m * xf(Vec3::Z, 0.4);
        let r = blend_mat(&a, &b, 0.5).unwrap();
        assert!(r.abs_diff_eq(m * xf(Vec3::Z * 0.5, 0.3), 1e-4), "{r:?}");
    }

    #[test]
    fn rotation_goes_the_short_way() {
        let a = xf(Vec3::ZERO, 3.0);
        let b = xf(Vec3::ZERO, -3.0); // 0.28 rad away across +-pi
        let r = blend_mat(&a, &b, 0.5).unwrap();
        assert!(r.abs_diff_eq(xf(Vec3::ZERO, std::f32::consts::PI), 1e-4));
        assert!(
            (lerp_yaw(3.0, -3.0, 0.5).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI)
                .abs()
                < 1e-5
        );
        assert!((lerp_yaw(0.1, 0.3, 0.5) - 0.2).abs() < 1e-6);
    }

    #[test]
    fn teleports_and_flips_snap() {
        let a = xf(Vec3::ZERO, 0.0);
        assert!(blend_mat(&a, &xf(Vec3::X * (SNAP_DIST + 1.0), 0.0), 0.5).is_none());
        assert!(blend_mat(&a, &xf(Vec3::ZERO, 3.0), 0.5).is_none());
        assert!(blend_mat(&a, &xf(Vec3::X * (SNAP_DIST - 1.0), 0.5), 0.5).is_some());
    }

    #[test]
    fn degenerate_matrices_lerp() {
        let a = Mat4::from_scale(Vec3::ZERO);
        let b = Mat4::IDENTITY;
        let r = blend_mat(&a, &b, 0.5).unwrap();
        assert!(r.is_finite());
        assert!(r.abs_diff_eq(Mat4::from_scale(Vec3::splat(0.5)), 1e-5));
    }

    #[test]
    fn parts_snap_on_sequence_change_or_wrap() {
        let a = pose(Vec3::ZERO, 0.0, tag(1, 2), 3.0);
        let b = pose(Vec3::X, 0.5, tag(1, 2), 4.0);
        assert!(parts_blend(&a, &b));
        let other_seq = Pose {
            tag: tag(1, 3),
            ..b.clone()
        };
        assert!(!parts_blend(&a, &other_seq));
        let wrapped = Pose {
            frame: 0.0,
            ..b.clone()
        };
        assert!(!parts_blend(&a, &wrapped));
        let more_parts = Pose {
            parts: vec![Mat4::IDENTITY; 2],
            ..b.clone()
        };
        assert!(!parts_blend(&a, &more_parts));

        // A sequence change still blends the body, not the parts.
        let (mut t, mut p) = (other_seq.transform, other_seq.parts.clone());
        assert!(blend_pose(&a, &other_seq, 0.5, &mut t, &mut p));
        assert!(t.abs_diff_eq(xf(Vec3::X * 0.5, 0.25), 1e-4));
        assert_eq!(p, other_seq.parts);
        // The same sequence blends both.
        let (mut t, mut p) = (b.transform, b.parts.clone());
        assert!(blend_pose(&a, &b, 0.5, &mut t, &mut p));
        assert!(p[0].abs_diff_eq(xf(Vec3::Y, 0.25), 1e-4));
        // A teleport snaps everything.
        let far = pose(Vec3::X * 50.0, 0.5, tag(1, 2), 4.0);
        let (mut t, mut p) = (far.transform, far.parts.clone());
        assert!(!blend_pose(&a, &far, 0.5, &mut t, &mut p));
        assert_eq!((t, p), (far.transform, far.parts.clone()));
    }

    #[test]
    fn camera_blends_and_cuts() {
        let a = (Vec3::ZERO, Vec3::Z, 0.5);
        let b = (Vec3::X, Vec3::Z + Vec3::X, 0.5);
        let c = blend_cam(a, b, 0.25);
        assert!(c.0.abs_diff_eq(Vec3::X * 0.25, 1e-6));
        let cut = (Vec3::X * 50.0, Vec3::Z, 0.5);
        assert_eq!(blend_cam(a, cut, 0.25), cut);
        let near = (Vec3::X, Vec3::Z, 1.0);
        assert_eq!(blend_cam(a, near, 0.25), near);
    }

    #[test]
    fn only_consecutive_ticks_blend() {
        let cam = (Vec3::ZERO, Vec3::Z, 0.5);
        let mut i = Interp::default();
        i.begin(10, cam);
        i.put(1, pose(Vec3::ZERO, 0.0, 0, 0.0));
        assert!(i.prev.is_empty() && i.cam(0.5).is_none());
        i.begin(11, cam);
        assert!(i.prev.contains_key(&1) && i.cam(0.5).is_some());
        // A skipped tick (or a rebuilt scene) has nothing to blend from.
        i.begin(13, cam);
        assert!(i.prev.is_empty());
        assert!(!i.needs(13) && i.needs(14));
    }

    #[test]
    fn stalled_game_draws_the_current_tick() {
        let cam = (Vec3::ZERO, Vec3::Z, 0.5);
        let mut i = Interp::default();
        i.begin(1, cam);
        i.begin(2, cam);
        assert_eq!(i.alpha(2, 0.3), 0.3);
        assert_eq!(i.alpha(2, 0.6), 0.6);
        assert_eq!(i.alpha(2, 1.7), 1.0);
        // The accumulator wrapped with no new tick: the game is paused.
        assert_eq!(i.alpha(2, 0.1), 1.0);
        assert_eq!(i.alpha(2, 0.5), 1.0);
        // It ticks again.
        i.begin(3, cam);
        assert_eq!(i.alpha(3, 0.2), 0.2);
        // A tick the scene hasn't captured draws as it is.
        assert_eq!(i.alpha(4, 0.4), 1.0);
    }
}
