//! The gameplay camera: NEWCAM's standard camera (newcam.c `UpdateCam` /
//! `fn_8006DF34`), and the frustum clip that keeps the players in its view
//! (`CameraLimitPlayerDpos` -> `fn_8006DC64`).
//!
//! On the GameCube, `main.c` runs `do_camera` (camera.c, CAM_GAME) only when
//! `lbl_80343C5C == 0`. That word is 1 in the DOL and nothing writes it, so normal play
//! always uses NEWCAM. Here is how it works:
//! * The look-at point is the 9-frame moving average of the centre of the
//!   players' screen-space bounding box (their attention points, 4.4 above the
//!   feet), clamped to the level's CAMS bounds.
//! * Yaw and pitch come from the nearest CameraGame locator. The nearest is
//!   found in 3D, with hysteresis: a new one must be 2/3 as far as the current
//!   one. When the locator changes, the camera turns linearly over 50
//!   30-Hz frames.
//! * The distance is the smallest one that frames every player inside the
//!   view frustum, with a 10-unit zoom-in and 4-unit zoom-out margin, at
//!   least CAMS `minrad`. It is 9-frame averaged. With one player it is
//!   always `minrad`.
//! * Players are clipped against the frustum of a camera at the nominal
//!   distance (`0.9375 * maxrad`, or the locator's own distance). A move out
//!   through a plane slides along it, or stops.
//! * At a level start the camera holds at the CameraStart locator for 3 s,
//!   then takes 10 frames to reach the play camera (`fn_8006E654`).
//! * The view is 60 degrees wide (`MBWindowZoom(1.0471976)`). The 3D window is
//!   the screen less the bottom 64 of 448 lines (`InitCamera` / `ChangeWindow`,
//!   which the status panels cover), and is centred in that window.

use crate::game::{Game, PlayerState};
use glam::{Mat4, Vec3};
use std::f32::consts::PI;

/// lbl_80343CD0: the attention / distance history length.
pub const RING: usize = 9;
/// lbl_80343CEC: 30-Hz frames a yaw / pitch change takes.
const INTERP_FRAMES: f32 = 50.0;
/// lbl_80343CD4: frames of the level-start camera move.
const TRANSITION_FRAMES: i32 = 10;
/// lbl_80343CD8 / lbl_80343CDC: CalcDist's zoom-in and zoom-out margins.
const ZOOM_IN_MARGIN: f32 = 10.0;
const ZOOM_OUT_MARGIN: f32 = 4.0;
/// lbl_80343CE4 / lbl_80343CE8: nominal distance scales of maxrad / minrad.
const MAX_SCALE: f32 = 0.9375;
const MIN_SCALE: f32 = 0.8;
/// lbl_80343CF0: how far a slid move may still point out through the plane.
const SLOPE: f32 = 0.0001;
/// CamReset's zoom: the horizontal field of view.
pub const FOV_H: f32 = std::f32::consts::FRAC_PI_3;
/// plyr_data attny / coly (the same in every PDATA): the attention point and
/// the collision point above the feet.
pub const ATTN_Y: f32 = 4.4;
pub const COLL_Y: f32 = 2.5;
/// world_update: seconds into the level when the start camera lets go.
const INTRO_HOLD: f32 = 3.0;
/// The display (640x448) and the status strip below the 3D window.
const SCREEN_H: f32 = 448.0;
const STRIP: f32 = 64.0;

/// FixAngle: wrap into (-pi, pi].
pub fn wrap(a: f32) -> f32 {
    let mut a = a;
    while a > PI {
        a -= 2.0 * PI;
    }
    while a <= -PI {
        a += 2.0 * PI;
    }
    a
}

/// tan of half the horizontal and the vertical view angles. MBWindowZoom:
/// hang = 2 atan(0.75 * height / 448 * tan(ang / 2)) for the 384-line window.
pub fn half_tans() -> (f32, f32) {
    let th = (FOV_H * 0.5).tan();
    (th, 0.75 * ((SCREEN_H - STRIP) / SCREEN_H) * th)
}

/// The projection for a `aspect` (width / height) target. The camera axis
/// goes through the centre of the 3D window, which is 1/7 of the half-height
/// above the screen's centre. At 4:3 this is exactly the game's view.
pub fn projection(aspect: f32, near: f32, far: f32) -> Mat4 {
    let (_, tv) = half_tans();
    // The whole screen's half-height in tan units.
    let full = tv * SCREEN_H / (SCREEN_H - STRIP);
    let fovy = 2.0 * full.atan();
    let shift = STRIP / SCREEN_H;
    Mat4::from_translation(Vec3::new(0.0, shift, 0.0))
        * Mat4::perspective_rh(fovy, aspect, near, far)
}

/// The near plane (pbWindow near_z). There is no cut-away.
pub const NEAR: f32 = 1.0;

/// PitchVec3(YawVec3((0,0,1), -yaw), -pitch): the look direction. Negative
/// pitch looks down.
pub fn look_dir(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        yaw.cos() * pitch.cos(),
    )
}

/// CamLookInDir: (right, up, forward), the camera rows. Forward is normalized.
pub fn look_basis(dir: Vec3) -> (Vec3, Vec3, Vec3) {
    let len = dir.length();
    if len < 0.001 {
        return (Vec3::X, Vec3::Y, Vec3::Z);
    }
    let f = dir / len;
    let up = if f.x * f.x + f.z * f.z < 0.0001 {
        if f.y > 0.0 {
            Vec3::Z
        } else {
            Vec3::NEG_Z
        }
    } else {
        Vec3::Y
    };
    let r = up.cross(f).normalize_or_zero();
    (r, f.cross(r), f)
}

/// CalcFrustrumNormals: the four outward (unnormalized) side-plane normals of
/// the view along `look`, with the horizontal axis from `yaw`. Index 3 is the
/// one CamLimitPlayerDpos tests with the feet.
pub fn frustum_normals(look: Vec3, yaw: f32) -> [Vec3; 4] {
    // YawVec3((-1, 0, 0), -yaw).
    let side = Vec3::new(-yaw.cos(), 0.0, yaw.sin());
    let c = look.cross(side);
    let (th, tv) = half_tans();
    let (h, v) = (side * th, c * tv);
    let r1 = look + h + v;
    let r2 = look - h + v;
    let r3 = look + h - v;
    let r4 = look - h - v;
    [r2.cross(r1), r1.cross(r3), r4.cross(r2), r3.cross(r4)]
}

/// The frustum a player is clipped against: a camera at `eye` (the
/// attention point less the look direction times the nominal distance).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frustum {
    pub eye: Vec3,
    pub planes: [Vec3; 4],
}

/// fn_8006DC64: clip a player's move `pt` (from feet `pos`) so it does not go
/// further out through any frustum plane. A plane the player is outside of
/// gives result 5 (else 1). A move out through it stops (`slide` false: the
/// player hit a wall), or loses its part along the plane's horizontal normal
/// and stops if `blocked` (the world refuses the slid move) or if it still
/// points out. The vertical part of the move is dropped once anything clips.
pub fn clip_move(
    f: &Frustum,
    pos: Vec3,
    mut pt: Vec3,
    slide: bool,
    blocked: impl Fn(Vec3, Vec3) -> bool,
) -> (Vec3, u32) {
    let coll = pos + Vec3::Y * COLL_Y;
    let feet = pos + pt - f.eye;
    let body = coll + pt - f.eye;
    let mut out_y = pt.y;
    let mut result = 1;
    for (i, n) in f.planes.iter().enumerate() {
        let d = if i == 3 { feet } else { body };
        if d.dot(*n) <= 0.0 {
            continue;
        }
        result = 5;
        if pt.dot(*n) <= 0.0 {
            continue;
        }
        out_y = 0.0;
        if !slide {
            pt = Vec3::ZERO;
            continue;
        }
        let hz = Vec3::new(n.x, 0.0, n.z).normalize_or_zero();
        pt -= hz * pt.dot(hz);
        if blocked(coll, pt) {
            pt = Vec3::ZERO;
        }
        let m = pt.dot(*n);
        if m * m > SLOPE * SLOPE * pt.length_squared() {
            pt = Vec3::ZERO;
        }
    }
    pt.y = out_y;
    (pt, result)
}

/// A CameraGame locator as sTriggerCameras holds it (type 0).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Marker {
    pub pos: Vec3,
    /// +0x14: the locator's pitch (positive looks down).
    pub pitch: f32,
    /// +0x18: the locator's yaw + pi.
    pub yaw: f32,
    /// +0x01 (the locator subtype): a fixed camera distance, or 0.
    pub dist: u8,
}

impl Marker {
    fn from_locator(l: &gdl_formats::worlds::Locator) -> Marker {
        Marker {
            pos: Vec3::from(l.pos),
            pitch: l.rot[0],
            yaw: wrap(l.rot[1] + PI),
            dist: l.delay,
        }
    }
}

/// The level's CAMS values NEWCAM reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub min: Vec3,
    pub max: Vec3,
    pub minrad: f32,
    pub maxrad: f32,
    pub minpitch: f32,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            min: Vec3::splat(-1e6),
            max: Vec3::splat(1e6),
            minrad: 24.0,
            maxrad: 32.0,
            minpitch: 0.0,
        }
    }
}

/// The NEWCAM camera object (NcCamera).
#[derive(Clone, Debug, PartialEq)]
pub struct StdCam {
    pub position: Vec3,
    pub planes: [Vec3; 4],
    pub attention: Vec3,
    pub velocity: Vec3,
    pub dist_current: f32,
    pub direction: Vec3,
    pub yaw: f32,
    pub yaw_rate: f32,
    pub distance: f32,
    pub dist_rate: f32,
    pub pitch: f32,
    pub pitch_rate: f32,
    pub ring_pos: [Vec3; RING],
    pub ring_dist: [f32; RING],
    pub ring_idx: usize,
    /// field_1A8: the locator the turn was last aimed at.
    pub aimed: i32,
    /// field_1AC: 30-Hz frames into the turn.
    pub turned: f32,
}

impl Default for StdCam {
    /// CamReset.
    fn default() -> Self {
        StdCam {
            position: Vec3::ZERO,
            planes: [Vec3::ZERO; 4],
            attention: Vec3::ZERO,
            velocity: Vec3::ZERO,
            dist_current: 0.0,
            direction: Vec3::ZERO,
            yaw: std::f32::consts::PI,
            yaw_rate: 0.0,
            distance: 0.0,
            dist_rate: 0.0,
            pitch: 0.0,
            pitch_rate: 0.0,
            ring_pos: [Vec3::ZERO; RING],
            ring_dist: [0.0; RING],
            ring_idx: 0,
            aimed: -1,
            turned: 0.0,
        }
    }
}

/// The camera last pushed to the renderer (MBCameraUpdate): what
/// MBWorldToScreen projects with.
#[derive(Clone, Copy, Debug, PartialEq)]
struct View {
    pos: Vec3,
    right: Vec3,
    up: Vec3,
    fwd: Vec3,
}

impl Default for View {
    fn default() -> Self {
        View {
            pos: Vec3::ZERO,
            right: Vec3::X,
            up: Vec3::Y,
            fwd: Vec3::Z,
        }
    }
}

impl View {
    fn of(cam: &StdCam) -> View {
        let (right, up, fwd) = look_basis(cam.direction);
        View {
            pos: cam.position,
            right,
            up,
            fwd,
        }
    }

    /// MBWorldToScreen, up to the window's scale and centre: (x/z, y/z, z).
    fn project(&self, p: Vec3) -> Vec3 {
        let d = p - self.pos;
        let z = d.dot(self.fwd).max(0.001);
        Vec3::new(d.dot(self.right) / z, d.dot(self.up) / z, z)
    }

    /// MBWorldToScreen3D.
    fn unproject(&self, s: Vec3) -> Vec3 {
        self.pos + self.right * (s.x * s.z) + self.up * (s.y * s.z) + self.fwd * s.z
    }
}

/// What the camera needs of a player: its feet, and whether the camera
/// tracks it (state 1 or 4).
#[derive(Clone, Copy, Debug)]
pub struct CamPlayer {
    pub pos: Vec3,
    pub tracked: bool,
}

/// The players as the camera sees them, and lbl_80344768 (heroes in play).
#[derive(Clone, Debug, Default)]
pub struct Party {
    pub players: Vec<CamPlayer>,
    pub count: usize,
}

/// The gameplay camera's state: the standard camera and NEWCAM's globals.
#[derive(Clone, Debug, Default)]
pub struct GameCamera {
    pub cam: StdCam,
    pub started: bool,
    markers: Vec<Marker>,
    /// lbl_80343CF8: the selected locator.
    sel: i32,
    view: View,
    params: Params,
    /// gScriptedCameraState.
    pub scripted: i32,
    /// lbl_803447B8: the level-start camera is up.
    pub intro: bool,
    /// lbl_80344A70: the start move's countdown.
    transition: i32,
    /// sMusicFadeBase - lbl_80344860: seconds into the level.
    time: f32,
    /// CurTransmitter: the CameraStart locator (as a trigger record).
    transmitter: Option<Marker>,
    /// Where each player was after last frame's clip.
    prev: Vec<Option<Vec3>>,
}

impl GameCamera {
    /// fn_8006FCDC: the nearest enabled CameraGame locator to `pos`. The current
    /// one is kept unless the best is within 4/9 of its squared distance.
    fn select(&mut self, pos: Vec3) -> Option<Marker> {
        if self.markers.is_empty() {
            return None;
        }
        let sel = self.sel;
        let mut best = -1i32;
        let mut best_d = 0.0f32;
        for (i, m) in self.markers.iter().enumerate() {
            if i as i32 == sel {
                continue;
            }
            let d = (pos - m.pos).length_squared();
            if best < 0 || d < best_d {
                best = i as i32;
                best_d = d;
            }
        }
        if sel < 0 {
            self.sel = best;
        } else if best >= 0 {
            let d = (pos - self.markers[sel as usize].pos).length_squared();
            if best_d <= 0.444_444_45 * d {
                self.sel = best;
            }
        }
        self.markers.get(self.sel.max(0) as usize).copied()
    }

    /// CamGetPlayerAvgPos: the centre of the tracked players' bounding box
    /// (bit 2: feet, bit 4: attention points), in screen space with bit 1, then
    /// clamped to the camera bounds. False if nobody is tracked.
    fn avg_pos(&self, party: &Party, flags: u32) -> (Vec3, bool) {
        let mut lo = Vec3::splat(1e20);
        let mut hi = Vec3::splat(-1e20);
        let mut n = 0;
        let pt = |p: Vec3| {
            if flags & 1 != 0 {
                self.view.project(p)
            } else {
                p
            }
        };
        for p in party.players.iter().filter(|p| p.tracked) {
            n += 1;
            if flags & 2 != 0 {
                let v = pt(p.pos);
                lo = lo.min(v);
                hi = hi.max(v);
            }
            if flags & 4 != 0 {
                let v = pt(p.pos + Vec3::Y * ATTN_Y);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        let mid = (lo + hi) * 0.5;
        let out = if flags & 1 != 0 && n > 0 {
            self.view.unproject(mid)
        } else {
            mid
        };
        (out.clamp(self.params.min, self.params.max), n > 0)
    }

    /// CalcDist: the distance that frames every tracked player (feet and
    /// attention point) about `point` along `look`, with the zoom margins.
    fn calc_dist(&self, cam: &mut StdCam, party: &Party, point: Vec3, current: f32) -> f32 {
        cam.planes = frustum_normals(cam.direction, cam.yaw);
        let p = &self.params;
        if p.minrad == p.maxrad {
            return p.minrad;
        }
        if party.count == 1 {
            return p.minrad;
        }
        let mut required = 0.0f32;
        for n in cam.planes {
            let inv = 1.0 / cam.direction.dot(n);
            let pd = point.dot(n);
            for pl in party.players.iter().filter(|p| p.tracked) {
                for q in [pl.pos + Vec3::Y * ATTN_Y, pl.pos] {
                    required = required.max(inv * (pd - q.dot(n)));
                }
            }
        }
        if required <= p.minrad - ZOOM_IN_MARGIN && p.minrad < current {
            p.minrad
        } else if required <= current - ZOOM_IN_MARGIN {
            required + ZOOM_IN_MARGIN
        } else if required <= current - ZOOM_OUT_MARGIN {
            current
        } else {
            required + ZOOM_OUT_MARGIN
        }
    }

    /// The target (yaw, pitch) for a locator, with the multiplayer pitch floor.
    fn aim(&self, m: Option<Marker>, party: &Party) -> (f32, f32) {
        let yaw = m.map_or(0.0, |m| wrap(m.yaw - PI));
        let mut pitch = m.map_or(0.0, |m| -m.pitch);
        if party.count > 1 {
            pitch = pitch.min(-self.params.minpitch);
        }
        (yaw, pitch)
    }

    /// fn_8006DF34: one frame of the standard camera. True while it moves.
    fn step(&mut self, cam: &mut StdCam, party: &Party, ticks: f32) -> bool {
        let (avg, any) = self.avg_pos(party, 5);
        if !any {
            return true;
        }
        cam.ring_idx = (cam.ring_idx + 1) % RING;
        cam.ring_pos[cam.ring_idx] = avg;
        let (feet, _) = self.avg_pos(party, 2);
        let marker = self.select(feet);
        let (ty, tp) = self.aim(marker, party);
        let turning = turn(cam, ty, tp, self.sel, 0.5 * ticks);
        // LimitTranslate.
        let mean = cam.ring_pos.iter().sum::<Vec3>() / RING as f32;
        let vel = mean - cam.attention;
        cam.attention += vel;
        let moved = vel != Vec3::ZERO;
        let p = self.params;
        cam.dist_current = if p.minrad >= p.maxrad {
            p.minrad
        } else if party.count == 1 {
            p.minrad * MIN_SCALE
        } else {
            match marker {
                Some(m) if m.dist != 0 => m.dist as f32,
                _ => p.maxrad * MAX_SCALE,
            }
        };
        let point = cam.ring_pos[cam.ring_idx];
        let current = cam.dist_current;
        cam.ring_dist[cam.ring_idx] = self.calc_dist(cam, party, point, current);
        let sd = cam.ring_dist.iter().map(|d| d - cam.distance).sum::<f32>() / RING as f32;
        cam.distance += sd;
        cam.direction = look_dir(cam.yaw, cam.pitch);
        cam.position = cam.attention - cam.direction * cam.distance;
        self.view = View::of(cam);
        sd != 0.0 || turning || moved
    }

    /// CalcDestination: step until nine frames in a row stand still, at most 100.
    fn converge(&mut self, cam: &mut StdCam, party: &Party) {
        let (mut ok, mut n) = (0, 0);
        while ok < RING && n < 100 {
            if self.step(cam, party, 1.0) {
                ok = 0;
            } else {
                ok += 1;
            }
            n += 1;
        }
    }

    /// fn_8006F418: put `cam` at the start camera, looking along it, its
    /// attention as far ahead as the players are.
    fn place_at(&mut self, cam: &mut StdCam, t: Option<Marker>, party: &Party) {
        let (yaw, pitch) = self.aim(t, party);
        cam.direction = look_dir(yaw, pitch);
        cam.yaw = yaw;
        cam.pitch = pitch;
        if let Some(t) = t {
            cam.position = t.pos;
        }
        cam.dist_current = self.params.maxrad;
        self.view = View::of(cam);
        let (avg, _) = self.avg_pos(party, 4);
        cam.distance = (cam.position - avg).length();
        cam.attention = cam.position + cam.direction * cam.distance;
    }

    /// fn_8006F16C(1): the camera on the players, converged.
    fn init_on_players(&mut self, party: &Party) {
        let mut cam = StdCam::default();
        let (avg, _) = self.avg_pos(party, 2);
        let m = self.select(avg);
        let (yaw, pitch) = self.aim(m, party);
        cam.yaw = yaw;
        cam.pitch = pitch;
        cam.direction = look_dir(yaw, pitch);
        cam.attention = avg;
        cam.distance = self.params.minrad;
        cam.dist_current = self.params.maxrad;
        cam.position = cam.attention - cam.direction * cam.distance;
        self.view = View::of(&cam);
        self.converge(&mut cam, party);
        self.cam = cam;
    }

    /// fn_8006E654: one frame of the move from the start camera to play.
    fn transition_step(&mut self, party: &Party) {
        let mut cam = std::mem::take(&mut self.cam);
        if self.transition == TRANSITION_FRAMES {
            let mut tmp = StdCam::default();
            self.place_at(&mut tmp, self.transmitter, party);
            self.converge(&mut tmp, party);
            let n = TRANSITION_FRAMES as f32;
            cam.yaw_rate = wrap(tmp.yaw - cam.yaw) / n;
            cam.pitch_rate = wrap(tmp.pitch - cam.pitch) / n;
            cam.dist_rate = (tmp.distance - cam.distance) / n;
            cam.velocity = (tmp.attention - cam.attention) / n;
        }
        cam.yaw = wrap(cam.yaw + cam.yaw_rate);
        cam.pitch = wrap(cam.pitch + cam.pitch_rate);
        cam.distance += cam.dist_rate;
        cam.attention += cam.velocity;
        cam.ring_idx = (cam.ring_idx + 1) % RING;
        cam.ring_pos[cam.ring_idx] = cam.attention;
        cam.ring_dist[cam.ring_idx] = cam.distance;
        cam.direction = look_dir(cam.yaw, cam.pitch);
        cam.position = cam.attention - cam.direction * cam.distance;
        self.view = View::of(&cam);
        if self.transition <= 0 {
            for _ in 0..RING {
                self.step(&mut cam, party, 1.0);
            }
            self.intro = false;
            self.transition -= 1;
            self.scripted = 0;
        }
        self.transition -= 1;
        self.cam = cam;
    }

    /// The level start (camera_mode_level + fn_8006F16C(inTower)).
    pub fn start(
        worlds: &gdl_formats::worlds::Worlds,
        params: Params,
        in_tower: bool,
        start_index: u16,
        party: &Party,
    ) -> GameCamera {
        use gdl_formats::worlds::LocatorType;
        let markers: Vec<Marker> = worlds
            .locators
            .iter()
            .filter(|l| l.kind == LocatorType::CameraGame)
            .map(Marker::from_locator)
            .collect();
        // AddLocatorInstList: outside the tower every CameraStart is start 0
        // (the last one wins); SetPlayerStart falls back to start 0.
        let starts: Vec<(u16, Marker)> = worlds
            .locators
            .iter()
            .filter(|l| l.kind == LocatorType::CameraStart)
            .map(|l| (if in_tower { l.next } else { 0 }, Marker::from_locator(l)))
            .collect();
        let pick = |i: u16| starts.iter().rev().find(|(n, _)| *n == i).map(|s| s.1);
        let transmitter = pick(start_index).or_else(|| pick(0));
        let mut c = GameCamera {
            markers,
            sel: -1,
            params,
            transmitter,
            transition: TRANSITION_FRAMES,
            started: true,
            ..Default::default()
        };
        if in_tower || transmitter.is_none() {
            c.init_on_players(party);
        } else {
            let mut cam = StdCam::default();
            c.place_at(&mut cam, transmitter, party);
            c.cam = cam;
            c.scripted = 91;
            c.intro = true;
        }
        c.prev = party.players.iter().map(|p| Some(p.pos)).collect();
        c
    }

    /// UpdateCam (plus world_update's start-camera timer): one frame of play.
    pub fn update(&mut self, party: &Party, dt: f32) {
        let ticks = (dt * 60.0).round().max(1.0);
        self.time += dt;
        if self.intro && self.time >= INTRO_HOLD && self.scripted > 1 {
            self.scripted = 1;
        }
        if self.scripted > 2 {
            self.scripted = 2;
        }
        if self.scripted == 1 {
            self.transition_step(party);
        }
        if self.scripted > 0 {
            return;
        }
        let mut cam = std::mem::take(&mut self.cam);
        self.step(&mut cam, party, ticks);
        self.cam = cam;
    }

    /// The frustum players are kept in (dist_current, not the actual distance).
    pub fn frustum(&self) -> Frustum {
        Frustum {
            eye: self.cam.attention - self.cam.direction * self.cam.dist_current,
            planes: self.cam.planes,
        }
    }

    /// Eye and look-at point.
    pub fn view(&self) -> (Vec3, Vec3) {
        (self.cam.position, self.cam.attention)
    }
}

/// fn_80070144: when the selected locator changes, aim a linear turn at its
/// angles over 50 30-Hz frames; advance it by `step` frames. True while turning.
fn turn(cam: &mut StdCam, yaw: f32, pitch: f32, sel: i32, step: f32) -> bool {
    if sel != cam.aimed {
        cam.aimed = sel;
        cam.yaw_rate = wrap(yaw - cam.yaw) / INTERP_FRAMES;
        cam.pitch_rate = wrap(pitch - cam.pitch) / INTERP_FRAMES;
        cam.turned = 0.0;
    }
    if cam.turned >= INTERP_FRAMES {
        return false;
    }
    cam.yaw = wrap(cam.yaw + cam.yaw_rate * step);
    cam.pitch = wrap(cam.pitch + cam.pitch_rate * step);
    cam.turned += step;
    true
}

impl Game {
    /// What the camera sees of the party now.
    pub(crate) fn camera_party(&self) -> Party {
        let players: Vec<CamPlayer> = self
            .players
            .iter()
            .map(|p| CamPlayer {
                pos: p.pos,
                tracked: p.state.active(),
            })
            .collect();
        // lbl_80344768: heroes neither absent nor dead.
        let count = self
            .players
            .iter()
            .filter(|p| matches!(p.state, PlayerState::Normal | PlayerState::Exiting { .. }))
            .count();
        Party { players, count }
    }

    /// The level's CAMS values, with camera_mode_level's defaults from the
    /// world bounds when the record sets none.
    pub(crate) fn camera_params(&self) -> Params {
        let Some(c) = self.wave.and_then(|w| self.worlds.cam(w)) else {
            return Params::default();
        };
        let (min, max) = if c.limits == 0 {
            let lo = Vec3::from(self.level.worlds.min_bounds);
            let hi = Vec3::from(self.level.worlds.max_bounds);
            (
                lo + Vec3::new(8.0, 0.0, 8.0),
                hi + Vec3::new(-8.0, 4.0, -8.0),
            )
        } else {
            (Vec3::from(c.min), Vec3::from(c.max))
        };
        Params {
            min,
            max,
            minrad: c.minrad,
            maxrad: c.maxrad,
            minpitch: c.minpitch,
        }
    }

    /// A camera started for the level as it stands.
    fn fresh_camera(&self) -> GameCamera {
        let in_tower = self.wave == Some(gdl_formats::wdata::TOWER_WAVE);
        GameCamera::start(
            &self.level.worlds,
            self.camera_params(),
            in_tower,
            self.start_index,
            &self.camera_party(),
        )
    }

    /// The per-frame camera: clip the players' moves, then run the camera
    /// unless a trigger camera, the boss camera or a fixed view has it.
    pub(crate) fn update_camera(&mut self, dt: f32) {
        if self.fixed_camera.is_some() {
            return;
        }
        if self.boss_camera_started() {
            return;
        }
        if !self.camera.started {
            self.camera = self.fresh_camera();
        }
        self.limit_players_to_view();
        if self.sumner.as_ref().is_some_and(|s| s.cam.is_some()) {
            return;
        }
        let party = self.camera_party();
        self.camera.update(&party, dt);
        let c = &self.camera.cam;
        // The controls are camera relative: cam_yaw is the yaw from the
        // attention point to the eye.
        self.cam_yaw = wrap(c.yaw + PI);
        self.cam_pitch = -c.pitch;
        self.cam_focus = c.attention;
    }

    /// The stick deflection that moves toward world direction `world` (x, z).
    pub fn stick_toward(&self, world: glam::Vec2) -> glam::Vec2 {
        let (f, r) = self.camera_basis();
        glam::Vec2::new(world.dot(r), world.dot(f))
    }

    fn boss_camera_started(&self) -> bool {
        self.boss
            .as_ref()
            .and_then(|b| b.cam.as_ref())
            .is_some_and(|c| c.started)
    }

    /// The view when nothing overrides it: (eye, look-at, near plane).
    pub(crate) fn camera_view_normal(&self) -> (Vec3, Vec3, f32) {
        let (eye, at) = if self.camera.started {
            self.camera.view()
        } else {
            self.fresh_camera().view()
        };
        (eye, at, NEAR)
    }

    /// Whether the world refuses a slid move of a player's collision point
    /// (PlayerWallCollide, PlayerCollideFloor).
    fn slide_blocked(&self, coll: Vec3, pt: Vec3) -> bool {
        let feet = coll - Vec3::Y * COLL_Y + pt;
        let pushed = self.collision.resolve_walls(
            feet,
            crate::game::PLAYER_RADIUS,
            crate::game::PLAYER_HEIGHT,
            crate::game::STEP_HEIGHT,
        );
        let wall = Vec3::new(pushed.x - feet.x, 0.0, pushed.z - feet.z).length() > 1e-3;
        let floor = self
            .collision
            .ground_height(feet, feet.y + crate::game::STEP_HEIGHT)
            .is_none();
        wall || floor
    }

    /// CameraLimitPlayerDpos with the standard camera: each player's move
    /// since last frame, clipped by the view frustum.
    fn limit_players_to_view(&mut self) {
        let f = self.camera.frustum();
        let prev = std::mem::take(&mut self.camera.prev);
        let mut now = Vec::with_capacity(self.players.len());
        for i in 0..self.players.len() {
            let p = &self.players[i];
            if let (Some(old), true) = (prev.get(i).copied().flatten(), p.state.active()) {
                if let Some(new) = self.clip_player(&f, old, p.pos) {
                    self.players[i].pos = new;
                }
            }
            now.push(Some(self.players[i].pos));
        }
        self.camera.prev = now;
    }

    /// Clip the move `old` -> `cur` (its horizontal part) by frustum `f`; the
    /// clipped position, if it changed. Shared with the boss camera.
    pub(crate) fn clip_player(&self, f: &Frustum, old: Vec3, cur: Vec3) -> Option<Vec3> {
        if f.planes.iter().all(|n| *n == Vec3::ZERO) {
            return None;
        }
        let d = Vec3::new(cur.x - old.x, 0.0, cur.z - old.z);
        let moved = d.length();
        // Teleports (exits, respawns) are not moves.
        if !(1e-5..=5.0).contains(&moved) {
            return None;
        }
        let base = Vec3::new(old.x, cur.y, old.z);
        let (pt, _) = clip_move(f, base, d, true, |c, pt| self.slide_blocked(c, pt));
        (pt != d).then(|| Vec3::new(old.x + pt.x, cur.y, old.z + pt.z))
    }

    /// CamLimitPlayerDpos with the boss camera (lbl_80343C5C set): the
    /// frustum of a camera `view_scale` back from the boss camera's attention
    /// point, along its look direction.
    pub(crate) fn boss_frustum(attn: Vec3, fwd: Vec3, view_scale: f32) -> Frustum {
        let yaw = fwd.x.atan2(fwd.z);
        Frustum {
            eye: attn - fwd * view_scale,
            planes: frustum_normals(fwd, yaw),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_angles() {
        let (th, tv) = half_tans();
        assert!((th - 0.577_35).abs() < 1e-4);
        // hang = 2 atan(0.642857 tan 30) = 40.73 degrees.
        assert!((2.0 * tv.atan() - 40.73f32.to_radians()).abs() < 1e-3);
        // The axis lands 1/7 above the screen centre; the window's top edge
        // at the screen's top.
        let p = projection(4.0 / 3.0, 1.0, 100.0);
        let c = p * Vec3::new(0.0, 0.0, -10.0).extend(1.0);
        assert!((c.y / c.w - 1.0 / 7.0).abs() < 1e-5);
        let top = p * Vec3::new(0.0, tv * 10.0, -10.0).extend(1.0);
        assert!((top.y / top.w - 1.0).abs() < 1e-4);
        let right = p * Vec3::new(th * 10.0, 0.0, -10.0).extend(1.0);
        assert!((right.x / right.w - 1.0).abs() < 1e-4);
    }

    #[test]
    fn frustum_matches_render() {
        // A point on each frustum edge lands on the matching edge of the
        // rendered 3D window (the renderer mirrors x).
        let dir = look_dir(0.9, -0.8);
        let eye = Vec3::new(5.0, 30.0, -2.0);
        let m = crate::scene::mirror();
        let view = Mat4::look_at_rh(
            m.transform_point3(eye),
            m.transform_point3(eye + dir),
            Vec3::Y,
        );
        let pm = projection(4.0 / 3.0, 1.0, 1000.0) * view;
        let (r, u, f) = look_basis(dir);
        let (th, tv) = half_tans();
        let ndc = |p: Vec3| {
            let c = pm * m.transform_point3(p).extend(1.0);
            (c.x / c.w, c.y / c.w)
        };
        let (_, top) = ndc(eye + (f + u * tv) * 10.0);
        assert!((top - 1.0).abs() < 1e-4);
        let (_, bottom) = ndc(eye + (f - u * tv) * 10.0);
        // The window's bottom: 384 of 448 lines down.
        assert!((bottom - (1.0 - 2.0 * 384.0 / 448.0)).abs() < 1e-4);
        let (x, _) = ndc(eye + (f + r * th) * 10.0);
        assert!((x.abs() - 1.0).abs() < 1e-4);
        // Points on the planes are on these edges.
        let n = frustum_normals(dir, 0.9);
        for p in [f + u * tv, f - u * tv, f + r * th, f - r * th] {
            assert!(n.iter().any(|q| p.dot(*q).abs() < 1e-4));
        }
    }

    #[test]
    fn look_conventions() {
        // Yaw 0 looks along +z; negative pitch looks down.
        let d = look_dir(0.0, -0.5);
        assert!(d.z > 0.0 && d.y < 0.0 && d.x.abs() < 1e-6);
        let d = look_dir(PI / 2.0, 0.0);
        assert!((d.x - 1.0).abs() < 1e-6);
        let (r, u, f) = look_basis(Vec3::Z);
        assert_eq!((r, u, f), (Vec3::X, Vec3::Y, Vec3::Z));
    }

    #[test]
    fn frustum_planes_face_out() {
        let look = look_dir(0.7, -0.9);
        let n = frustum_normals(look, 0.7);
        let (th, tv) = half_tans();
        let (r, u, f) = look_basis(look);
        let eye = Vec3::new(3.0, 20.0, -4.0);
        // On the axis: inside every plane.
        for p in n {
            assert!((f * 10.0).dot(p) < 0.0);
        }
        // Just beyond each edge: outside exactly one plane.
        for (x, y) in [(1.05, 0.0), (-1.05, 0.0), (0.0, 1.05), (0.0, -1.05)] {
            let q = eye + (f + r * (x * th) + u * (y * tv)) * 10.0;
            let out = n.iter().filter(|p| (q - eye).dot(**p) > 0.0).count();
            assert_eq!(out, 1, "{x} {y}");
        }
        // Just inside the edges: inside all.
        for (x, y) in [(0.95, 0.95), (-0.95, -0.95)] {
            let q = eye + (f + r * (x * th) + u * (y * tv)) * 10.0;
            assert!(n.iter().all(|p| (q - eye).dot(*p) <= 0.0));
        }
    }

    fn party(ps: &[Vec3]) -> Party {
        Party {
            players: ps
                .iter()
                .map(|&pos| CamPlayer { pos, tracked: true })
                .collect(),
            count: ps.len(),
        }
    }

    #[test]
    fn clip_slides_along_side() {
        let look = look_dir(0.0, -0.8);
        let f = Frustum {
            eye: Vec3::new(0.0, 20.0, -20.0),
            planes: frustum_normals(look, 0.0),
        };
        // Find a point past the left/right edge (screen x is -world x here).
        let (r, _, _) = look_basis(look);
        let mut pos = Vec3::ZERO;
        while f.planes.iter().all(|n| (pos - f.eye).dot(*n) <= 0.0) {
            pos += r;
        }
        // Moving further out sideways and forward: the outward part goes.
        let pt = r * 0.5 + Vec3::Z * 0.3;
        let (out, res) = clip_move(&f, pos, pt, true, |_, _| false);
        assert_eq!(res, 5);
        assert!(out.dot(r) < pt.dot(r));
        // Moving back in is free.
        let (back, res) = clip_move(&f, pos, -r * 0.5, true, |_, _| false);
        assert_eq!(res, 5);
        assert_eq!(back, -r * 0.5);
        // After a wall hit, an outward move stops.
        let (stop, _) = clip_move(&f, pos, pt, false, |_, _| false);
        assert_eq!(stop, Vec3::ZERO);
        // Well inside: untouched.
        let (free, res) = clip_move(&f, Vec3::new(0.0, 0.0, 0.0), pt, true, |_, _| false);
        assert_eq!((free, res), (pt, 1));
    }

    #[test]
    fn distance_frames_players() {
        let mut c = GameCamera {
            params: Params {
                minrad: 24.0,
                maxrad: 32.0,
                ..Params::default()
            },
            ..Default::default()
        };
        let mut cam = StdCam::default();
        cam.yaw = 0.3;
        cam.pitch = -0.9;
        cam.direction = look_dir(cam.yaw, cam.pitch);
        // One player: always minrad.
        let one = party(&[Vec3::ZERO]);
        assert_eq!(c.calc_dist(&mut cam, &one, Vec3::Y * ATTN_Y, 25.6), 24.0);
        // Two close players: minrad.
        let two = party(&[Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)]);
        assert_eq!(c.calc_dist(&mut cam, &two, Vec3::Y * 2.2, 30.0), 24.0);
        // Far apart: past the nominal distance, and everyone fits.
        let wide = party(&[Vec3::new(-25.0, 0.0, 0.0), Vec3::new(25.0, 0.0, 0.0)]);
        let point = Vec3::Y * 2.2;
        let d = c.calc_dist(&mut cam, &wide, point, 30.0);
        assert!(d > 30.0);
        let eye = point - cam.direction * d;
        for pl in &wide.players {
            for q in [pl.pos, pl.pos + Vec3::Y * ATTN_Y] {
                assert!(cam.planes.iter().all(|n| (q - eye).dot(*n) <= 1e-3));
            }
        }
        c.params.minrad = 40.0;
        c.params.maxrad = 40.0;
        assert_eq!(c.calc_dist(&mut cam, &wide, point, 30.0), 40.0);
    }

    #[test]
    fn turns_linearly_over_fifty_frames() {
        let mut cam = StdCam::default();
        cam.yaw = 3.0;
        cam.pitch = -0.5;
        // Across the +-pi seam the short way.
        let target = -3.0;
        let mut frames = 0;
        while turn(&mut cam, target, -1.0, 4, 0.5) {
            frames += 1;
        }
        assert_eq!(frames, 100);
        assert!((wrap(cam.yaw - target)).abs() < 1e-4);
        assert!((cam.pitch + 1.0).abs() < 1e-4);
        // The same locator again: no new turn.
        assert!(!turn(&mut cam, 0.0, 0.0, 4, 0.5));
    }

    #[test]
    fn selection_hysteresis() {
        let mk = |x: f32| Marker {
            pos: Vec3::new(x, 0.0, 0.0),
            ..Default::default()
        };
        let mut c = GameCamera {
            markers: vec![mk(0.0), mk(10.0)],
            sel: -1,
            ..Default::default()
        };
        assert_eq!(c.select(Vec3::new(1.0, 0.0, 0.0)).unwrap().pos.x, 0.0);
        // Past halfway, but not 2/3 as far: keep the first.
        assert_eq!(c.select(Vec3::new(5.5, 0.0, 0.0)).unwrap().pos.x, 0.0);
        assert_eq!(c.select(Vec3::new(6.5, 0.0, 0.0)).unwrap().pos.x, 10.0);
    }

    #[test]
    fn screen_round_trip() {
        let cam = StdCam {
            position: Vec3::new(1.0, 30.0, -20.0),
            direction: look_dir(0.4, -0.9),
            ..Default::default()
        };
        let v = View::of(&cam);
        let p = Vec3::new(3.0, 1.0, 4.0);
        assert!((v.unproject(v.project(p)) - p).length() < 1e-3);
    }
}
