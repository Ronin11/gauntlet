//! The attract-mode flyby camera (camera.c camera_mode_dest / DiffRate /
//! CameraSupervisor, default path-following mode): from the level's
//! CameraAttractStart it flies at a constant 0.2 units a tick, always steering
//! for the second-nearest armed attract camera and disarming each one it
//! passes, while its look direction eases toward the nearest camera's angles.
//! It is done once only one camera is left armed.

use gdl_formats::worlds::{LocatorType, Worlds};
use glam::Vec3;
use std::f32::consts::{PI, TAU};

/// One attract camera (a type-1 trigger camera): position and look angles.
#[derive(Clone, Copy, Debug)]
struct Node {
    eye: Vec3,
    pitch: f32,
    yaw: f32,
    armed: bool,
}

/// FixAngle: wrap into (-pi, pi].
fn wrap(a: f32) -> f32 {
    let mut a = a;
    while a > PI {
        a -= TAU;
    }
    while a <= -PI {
        a += TAU;
    }
    a
}

/// YPR(p, y, 0) x (0, 0, 1): yaw 0 is +z, positive pitch up.
fn ypr(p: f32, y: f32) -> Vec3 {
    Vec3::new(y.sin() * p.cos(), p.sin(), y.cos() * p.cos())
}

/// get_yaw(a, b): the yaw of the direction from b to a.
fn get_yaw(a: Vec3, b: Vec3) -> f32 {
    let d = a - b;
    let dz = if d.z.abs() < 0.001 {
        0.001f32.copysign(d.z)
    } else {
        d.z
    };
    d.x.atan2(dz)
}

/// get_pitch(a, b): the pitch of the direction from a to b.
fn get_pitch(a: Vec3, b: Vec3) -> f32 {
    let d = b - a;
    let h = (d.x * d.x + d.z * d.z).sqrt().max(0.001);
    d.y.atan2(h)
}

const STEP: f32 = 0.2;
const ACCEL: f32 = 0.000_872_664_6;
const RATE_MAX: f32 = 0.008_726_646;
const RADIUS: f32 = 20.0;

pub struct FlybyCam {
    nodes: Vec<Node>,
    pub wpos: Vec3,
    /// Look pitch / yaw (cam.pyr).
    look_pitch: f32,
    look_yaw: f32,
    /// Heading pitch / yaw (p442C / y4430), their rates (pr / yr) and targets.
    head_pitch: f32,
    head_yaw: f32,
    pitch_rate: f32,
    yaw_rate: f32,
    head_pitch_tgt: f32,
    head_yaw_tgt: f32,
    /// Look pitch target / current (p4530 / p4408) and look yaw target (y4534).
    look_pitch_tgt: f32,
    look_pitch_cur: f32,
    look_yaw_tgt: f32,
    /// Look rates (444C yaw, 4450 pitch) and the previous ones (4454, 4458).
    rate_yaw: f32,
    rate_pitch: f32,
    prev_rate_yaw: f32,
    prev_rate_pitch: f32,
    /// dir4400: the look yaw's turning direction (0 settled).
    dir: i32,
    hist: [f32; 3],
    near: i32,
    next: i32,
    sel: i32,
    /// The done counter (lbl_8034429C).
    pub done: u32,
    /// Frames run (the first two hold still).
    frames: u32,
}

impl FlybyCam {
    /// InitCamera for MA_FLYBY: start at the level's CameraAttractStart.
    pub fn new(worlds: &Worlds) -> Option<FlybyCam> {
        let mut start = None;
        let mut nodes = Vec::new();
        for l in &worlds.locators {
            if matches!(
                l.kind,
                LocatorType::CameraAttractStart | LocatorType::CameraAttract
            ) {
                if l.kind == LocatorType::CameraAttractStart {
                    start = Some(nodes.len());
                }
                nodes.push(Node {
                    eye: Vec3::from(l.pos),
                    pitch: -l.rot[0],
                    yaw: l.rot[1],
                    armed: true,
                });
            }
        }
        let s = nodes[start?];
        let mut c = FlybyCam {
            nodes,
            wpos: s.eye,
            look_pitch: s.pitch,
            look_yaw: s.yaw,
            head_pitch: 0.0,
            head_yaw: 0.0,
            pitch_rate: 0.0,
            yaw_rate: 0.0,
            head_pitch_tgt: 0.0,
            head_yaw_tgt: 0.0,
            look_pitch_tgt: 0.7,
            look_pitch_cur: 0.7,
            look_yaw_tgt: 0.0,
            rate_yaw: 0.0,
            rate_pitch: 0.0,
            prev_rate_yaw: 0.0,
            prev_rate_pitch: 0.0,
            dir: 1,
            hist: [0.0; 3],
            near: -1,
            next: -1,
            sel: -1,
            done: 0,
            frames: 0,
        };
        c.supervise(1);
        if c.near != c.next && c.next >= 0 {
            let n = c.nodes[c.next as usize].eye;
            c.head_pitch_tgt = get_pitch(c.wpos, n);
            c.head_yaw_tgt = get_yaw(c.wpos, n);
        }
        c.head_pitch = c.head_pitch_tgt;
        c.head_yaw = c.head_yaw_tgt;
        c.look_pitch_tgt = c.look_pitch;
        c.look_pitch_cur = c.look_pitch;
        // The common tail's y4534 = lbl_80118B60[0] (overwritten next frame).
        c.look_yaw_tgt = 0.0;
        Some(c)
    }

    /// Eye and look-at.
    pub fn view(&self) -> (Vec3, Vec3) {
        let d = ypr(self.look_pitch, self.look_yaw).normalize_or_zero();
        (self.wpos, self.wpos + d * RADIUS)
    }

    /// CameraSupervisor: the nearest and second-nearest armed cameras, the
    /// look targets and rates, and disarming the camera just passed.
    fn supervise(&mut self, ticks: u32) {
        let old_sel = self.sel;
        let old_near = self.near;
        let mut count = 0;
        let (mut dn, mut ds) = (100_000.0f32, 100_000.0f32);
        let (mut ny, mut np, mut sy, mut sp) = (0.0, 0.0, 0.0, 0.0);
        for (i, n) in self.nodes.iter().enumerate() {
            if !n.armed {
                continue;
            }
            let d = (self.wpos - n.eye).length();
            if d < dn {
                count += 1;
                self.next = self.near;
                ds = dn;
                (sy, sp) = (ny, np);
                self.near = i as i32;
                dn = d;
                (ny, np) = (n.yaw, n.pitch);
            } else if d < ds {
                count += 1;
                self.next = i as i32;
                ds = d;
                (sy, sp) = (n.yaw, n.pitch);
            }
        }
        if count == 1 {
            self.next = self.near;
            ds = dn;
            (sy, sp) = (ny, np);
        }
        if count != 0 {
            if count == 1 || dn + ds == 0.0 {
                self.done += ticks;
                return;
            }
            let a = self.nodes[self.near as usize].eye;
            let b = self.nodes[self.next as usize].eye;
            // PointLineColl, clamped to the segment.
            let ab = b - a;
            let t = ((self.wpos - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            let ratio = (ab * t).length() / ab.length().max(1e-6);
            if ratio <= 0.5 {
                self.look_yaw_tgt = ny;
                self.look_pitch_tgt = np;
                self.sel = self.near;
            } else {
                self.look_yaw_tgt = sy;
                self.look_pitch_tgt = sp;
                self.sel = self.next;
            }
            let d = self.look_yaw_tgt - self.look_yaw;
            self.dir = if d < -PI {
                1
            } else if d < 0.0 {
                -1
            } else if d < PI {
                1
            } else {
                -1
            };
            if self.sel != old_sel {
                // Rates sized to arrive in 5 ticks a unit, limited.
                let dd = 5.0 * (self.wpos - self.nodes[self.sel as usize].eye).length();
                if dd != 0.0 {
                    let dd = dd.max(1.0);
                    let mut d = self.look_yaw_tgt - self.look_yaw;
                    if d > PI {
                        d = TAU - d;
                    }
                    let mut r = (d / dd).abs().min(0.010_471_975);
                    if (r - self.prev_rate_yaw).abs() >= ACCEL {
                        r = self.prev_rate_yaw + ACCEL.copysign(r - self.prev_rate_yaw);
                    }
                    self.rate_yaw = r;
                    let mut r = ((self.look_pitch_tgt - self.look_pitch_cur) / dd)
                        .abs()
                        .min(0.006_283_185_4);
                    if (r - self.prev_rate_pitch).abs() >= 0.000_523_598_8 {
                        r = self.prev_rate_pitch
                            + 0.000_523_598_8f32.copysign(r - self.prev_rate_pitch);
                    }
                    self.rate_pitch = r;
                    self.prev_rate_yaw = self.rate_yaw;
                    self.prev_rate_pitch = self.rate_pitch;
                } else {
                    self.rate_yaw = 0.0;
                    self.prev_rate_yaw = 0.0;
                    self.rate_pitch = 0.0;
                    self.prev_rate_pitch = 0.0;
                }
            }
        }
        if old_near >= 0 && old_near == self.next {
            self.nodes[old_near as usize].armed = false;
        }
    }

    /// DiffRate: the look yaw eases toward its target.
    fn diff_rate(&mut self, ticks: u32) {
        let prev = self.look_yaw;
        self.hist = [prev, self.hist[0], self.hist[1]];
        self.supervise(ticks);
        let rate = self.rate_yaw * ticks as f32;
        let tgt = self.look_yaw_tgt;
        let cur = self.look_yaw;
        if self.dir > 0 && cur != tgt {
            let new = wrap(cur + rate);
            let past = if cur > new {
                tgt > cur || tgt <= new
            } else {
                new - tgt < PI && new >= tgt
            };
            self.look_yaw = new;
            if past {
                self.look_yaw = tgt;
                self.dir = 0;
            }
        } else if self.dir < 0 && cur != tgt {
            let new = wrap(cur - rate);
            let past = if cur < new {
                tgt < cur || tgt >= new
            } else {
                tgt - new < PI && new <= tgt
            };
            self.look_yaw = new;
            if past {
                self.look_yaw = tgt;
                self.dir = 0;
            }
        } else {
            self.look_yaw = tgt;
            self.dir = 0;
        }
        let y = self.look_yaw;
        if (y > 0.0 && self.hist[1] > 0.0 && self.hist[0] < 0.0)
            || (y < 0.0 && self.hist[1] < 0.0 && self.hist[0] > 0.0)
        {
            self.look_yaw = tgt;
            self.dir = 0;
        }
    }

    /// One game frame of `ticks` (retail runs the attract loop at 30 fps, 2).
    pub fn update(&mut self, ticks: u32) {
        // The first two frames hold still (ticks forced to 1).
        if self.frames < 2 {
            self.frames += 1;
            return;
        }
        let tf = ticks as f32;
        self.diff_rate(ticks);
        if self.near != self.next && self.next >= 0 {
            let n = self.nodes[self.next as usize].eye;
            self.head_pitch_tgt = get_pitch(self.wpos, n);
            self.head_yaw_tgt = get_yaw(self.wpos, n);
            // Heading pitch: acceleration-limited (it can overshoot).
            let reached;
            if self.head_pitch < self.head_pitch_tgt {
                if self.pitch_rate < RATE_MAX {
                    self.pitch_rate += ACCEL;
                }
                self.head_pitch += self.pitch_rate * tf;
                reached = self.head_pitch >= self.head_pitch_tgt;
            } else if self.head_pitch > self.head_pitch_tgt {
                if self.pitch_rate > -RATE_MAX {
                    self.pitch_rate -= ACCEL;
                }
                self.head_pitch += self.pitch_rate * tf;
                reached = self.head_pitch <= self.head_pitch_tgt;
            } else {
                reached = true;
            }
            if reached {
                self.pitch_rate -=
                    ACCEL.copysign(self.pitch_rate) * (self.pitch_rate != 0.0) as i32 as f32;
            }
            // Heading yaw: the step uses the rate from before this frame's change.
            let d = wrap(self.head_yaw_tgt - self.head_yaw);
            let ys = self.yaw_rate * tf;
            let reached = if d > 0.0 {
                if self.yaw_rate < RATE_MAX {
                    self.yaw_rate += ACCEL;
                }
                ys >= 0.0 && d <= ys
            } else if d < 0.0 {
                if self.yaw_rate > -RATE_MAX {
                    self.yaw_rate -= ACCEL;
                }
                ys <= 0.0 && d >= ys
            } else {
                self.yaw_rate -=
                    ACCEL.copysign(self.yaw_rate) * (self.yaw_rate != 0.0) as i32 as f32;
                false
            };
            if reached {
                self.yaw_rate -=
                    ACCEL.copysign(self.yaw_rate) * (self.yaw_rate != 0.0) as i32 as f32;
            }
            self.head_yaw = wrap(self.head_yaw + ys);
            self.wpos += ypr(self.head_pitch, wrap(PI + self.head_yaw)) * (STEP * tf);
        } else {
            self.done += ticks;
        }
        // Look pitch: approach without overshoot.
        let s = self.rate_pitch * tf;
        if self.look_pitch_tgt - self.look_pitch_cur > 0.0 {
            self.look_pitch_cur = (self.look_pitch_cur + s).min(self.look_pitch_tgt);
        } else {
            self.look_pitch_cur = (self.look_pitch_cur - s).max(self.look_pitch_tgt);
        }
        self.look_pitch = self.look_pitch_cur;
    }
}

/// The attract wave cursor (NextAttractWave): the startwave B1 first, then
/// each loaded realm in table order showing its attract-flagged level.
#[derive(Clone, Debug, Default)]
pub struct WaveCursor {
    wave: Option<u32>,
    next_level: [usize; 14],
}

/// sWorldDataTypes order (the loaded-world table).
const REALM_ORDER: [u32; 14] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 12, 0];

impl WaveCursor {
    pub fn next(&mut self, worlds: &gdl_formats::wdata::Worlds) -> u32 {
        let Some(cur) = self.wave else {
            self.wave = Some(0x200);
            return 0x200;
        };
        let mut wi = REALM_ORDER.iter().position(|&r| r == cur >> 8).unwrap_or(0);
        for _ in 0..14 {
            wi = (wi + 1) % 14;
            let realm = REALM_ORDER[wi];
            let Some(r) = worlds.realm(realm) else {
                continue;
            };
            let n = r.levels.len();
            if n == 0 {
                continue;
            }
            let mut level = self.next_level[wi];
            if level >= n {
                level = 0;
            }
            let orig = level;
            while r.levels[level].enabled & 2 == 0 {
                level = (level + 1) % n;
                if level == orig {
                    break;
                }
            }
            if r.levels[level].enabled & 2 == 0 {
                continue;
            }
            self.next_level[wi] = (level + 1) % n;
            let wave = realm << 8 | level as u32;
            self.wave = Some(wave);
            return wave;
        }
        cur
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waves_and_flight_times_match_the_reference_run() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let Ok(fs) = crate::fs::GameFs::open(&root) else {
            return;
        };
        let wd = gdl_formats::wdata::Worlds::load(|p| fs.read(p)).unwrap();
        let mut cur = WaveCursor::default();
        // (level, end tick at 2 ticks a frame) from the spec's port.
        let expect = [
            ("B1", 1012),
            ("C4", 1962),
            ("D1", 1290),
            ("F1", 630),
            ("G1", 1146),
            ("H2", 1230),
            ("I4", 842),
            ("J2", 1096),
            ("K1", 1604),
            ("L1", 1258),
            ("A2", 988),
            ("B5", 1288),
        ];
        for (name, end) in expect {
            let wave = cur.next(&wd);
            let dir = wd.dir(wave).unwrap();
            assert_eq!(dir, format!("level{name}"));
            let d = fs.read(&format!("LEVELS/{dir}/WORLDS.PS2")).unwrap();
            let w = gdl_formats::worlds::Worlds::parse(&d).unwrap();
            let mut cam = FlybyCam::new(&w).unwrap();
            let mut ticks = 0;
            while cam.done <= 30 && ticks < 3600 {
                cam.update(2);
                ticks += 2;
            }
            println!("{name}: ends at {ticks} (reference {end})");
            assert!(
                (ticks as i32 - end).abs() <= end / 5,
                "{name}: {ticks} vs {end}"
            );
        }
    }
}
