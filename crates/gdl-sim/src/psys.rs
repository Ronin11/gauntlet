//! mb_particle.c: the MB particle systems. A `Psys` hangs off a scene node;
//! each frame MBDrawPsys runs its emitter phases (delay, life, fade, dead),
//! spawns particles from the mode-selected position and direction generators
//! (setupNewPMode), retires the ones past life + fade, and draws the rest as
//! camera-facing sprites (DrawPsysSub) coloured and sized along their age.
//!
//! Particles keep the ring of `(position slot, direction slot, age delta)`
//! entries the original packs into u16s; a particle's place is its slot's
//! origin plus its slot's direction times its age (getPPos*), so it never
//! moves with the node once spawned. The clock is MBPsysStartFrame's: one
//! step per 30 Hz game frame.

use gdl_formats::psys::{enables as en, PsysRecord};
use glam::{Mat4, Vec3};
use std::collections::VecDeque;

/// psysInfo's per-parameter scale, minimum, maximum and default (blue,
/// green, red, intensity, width; 0x801287AC..).
const PARM_SCALE: [f32; 5] = [255.0, 255.0, 255.0, 255.0, 450.0];
const PARM_MIN: [f32; 5] = [0.0, 0.0, 0.0, 0.0, 1.0];
const PARM_MAX: [f32; 5] = [255.0, 255.0, 255.0, 255.0, 100_000.0];
const PARM_DEFAULT: [f32; 5] = [255.0, 255.0, 255.0, 255.0, 225.0];
const P_BLUE: usize = 0;
const P_GREEN: usize = 1;
const P_RED: usize = 2;
const P_INTENSITY: usize = 3;
const P_WIDTH: usize = 4;

/// MBInitPsys: the shared sphere-direction table (maxEntries).
const SPHERE_ENTRIES: usize = 1023;
/// MBInitPsys's block pool, and allocPsysMem's block rounding.
const POOL_BYTES: i32 = 120_000;
const PSYS_BYTES: i32 = 304;
/// DrawPsysSub stops after this many quads a frame (lbl_80345188 > 2048).
pub const MAX_QUADS: usize = 2048;
/// AddPsysObject: TOO MANY PSYS OBJECTS past 255 a frame.
pub const MAX_DRAWN: usize = 255;
/// createPsysNode's max_dist (lbl_80349220): MBDrawPsysTest's sphere.
const MAX_DIST: f32 = 10.0;

/// Psys.flags.
pub mod flags {
    /// Emit p_max at once, then stop.
    pub const ONESHOT: u16 = 0x1;
    /// Cycle the emitter's life and fade (or hold one at 0xFFFF frames).
    pub const FOREVER: u16 = 0x2;
    pub const POS_SET: u16 = 0x4;
    pub const DIR_SET: u16 = 0x8;
    /// pbBlitSetDrawRegs 0x200000 / 0x100000.
    pub const NOTEX_RGB: u16 = 0x10;
    pub const NOTEX_A: u16 = 0x20;
    /// Positions / directions from the node each frame (else once).
    pub const POS_FRAME: u16 = 0x40;
    pub const DIR_FRAME: u16 = 0x80;
}

/// Node (MBObject) flags the particle draw reads.
pub mod node_flags {
    /// fb_add: additive (and the particle2_xp default texture).
    pub const FB_ADD: u32 = 0x80_0000;
    pub const FB_MUL: u32 = 0x4000_0000;
    pub const SORT: u32 = 0x800;
    pub const NO_ZCOMPARE: u32 = 0x40;
    pub const NO_ZWRITE: u32 = 0x80;
}

/// pbRand: the low 15 bits of the game's LCG.
#[derive(Clone, Debug)]
pub struct PbRand(pub u32);

impl PbRand {
    pub fn rand(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(0x343FD).wrapping_add(0x269EC3);
        self.0 & 0x7FFF
    }
}

/// How a sprite blends (pbBlitSetDrawRegs: GXSetBlendMode src, dst).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    /// SRCALPHA, INVSRCALPHA.
    Alpha,
    /// fb_add: SRCALPHA, ONE.
    Add,
    /// fb_add without texture alpha: ONE, ONE.
    AddOne,
    /// No-texture-colour: ZERO, SRCALPHA (darkens).
    Darken,
    /// No-texture-alpha: ONE, ZERO.
    Replace,
}

/// One drawn psys: its sprites share a texture and draw state.
#[derive(Clone, Debug)]
pub struct SpriteBatch {
    /// Index into [`Particles::textures`].
    pub tex: Option<usize>,
    pub blend: Blend,
    pub ztest: bool,
    pub zwrite: bool,
    /// (centre, width, 0xAARRGGBB) per sprite, newest first.
    pub sprites: Vec<(Vec3, f32, u32)>,
}

/// psys->ppos_func.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PPos {
    Linear,
    Speed,
    Grav,
    SpeedGrav,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum PosMode {
    /// getNewPosSingle1 / 2: the node origin, once.
    Single,
    /// getNewPosFrame: the node origin, one slot a frame.
    Frame,
    RectUnique,
    RectShare,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum DirMode {
    Single,
    Frame,
    /// getNewDirSphere: the shared table.
    Sphere,
    ConeUnique,
    ConeShare,
}

/// One ring entry: its slots and its age less the next older one's.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Entry {
    pos: u16,
    dir: u16,
    delta: u16,
}

/// What a frame's MBDrawPsys calls share: pbRand, the sphere table and the
/// frame-mode slot caches (reset for each psys).
pub struct Ctx<'a> {
    rng: &'a mut PbRand,
    sphere: &'a mut [Vec3],
    sphere_use: &'a mut [u8],
    pool: &'a mut i32,
    dir_slot: i32,
    pos_slot: i32,
}

/// A particle system (PSYS).
#[derive(Clone, Debug)]
pub struct Psys {
    pub flags: u16,
    /// The node's flags (fb_add, sort, z modes).
    pub node_flags: u32,
    pub e_phase: u8,
    e_delay: u16,
    e_life: u16,
    e_fade: u16,
    /// Cone half-angle (radians); 0 a line, -1 a sphere.
    e_angle: f32,
    e_dir: Vec3,
    e_vol: Vec3,
    p_life: u8,
    p_fade: u8,
    p_speed: f32,
    p_gravity: f32,
    p_drag: f32,
    p_max: u16,
    dir_max: u16,
    pos_max: u16,
    /// e_rate: per frame; [start, end] of life and fade, then (setupParms)
    /// start and slope of each.
    e_rate: [f32; 4],
    e_rate_rand: f32,
    /// p_parms: blue, green, red, intensity, width (same layout).
    p_parms: [[f32; 4]; 5],
    pub tex: Option<usize>,
    pos_mode: PosMode,
    dir_mode: DirMode,
    ppos: PPos,
    pos_slots: Vec<Vec3>,
    dir_slots: Vec<Vec3>,
    pos_use: Vec<u8>,
    dir_use: Vec<u8>,
    pos_next: u16,
    pos_last: u16,
    dir_next: u16,
    dir_last: u16,
    age_mask: u16,
    ring: VecDeque<Entry>,
    oldest_age: u32,
    newest_age: u32,
    save_cnt: f32,
    e_age: u32,
    e_last_time: u32,
    /// MBDrawPsysTest's e_isvis and the last draw's p_nactive.
    pub e_isvis: bool,
    pub p_nactive: usize,
    /// Bytes this psys holds in the pool (none for world ones).
    pool_bytes: i32,
    world: bool,
    /// The node's world matrix and node->scale[1].
    pub node: Mat4,
    pub scale: f32,
    /// The node is hidden: emit nothing.
    pub hidden: bool,
}

/// setWorldParms' clamped seconds-to-frames conversions.
fn frames16(v: f32, lo: f64) -> u16 {
    let v = 30.0 * v as f64;
    v.clamp(lo, 65535.0) as i32 as u16
}

fn frames8(v: f32, lo: f64) -> u8 {
    let v = 30.0 * v as f64;
    v.clamp(lo, 255.0) as i32 as u8
}

/// setPTimeVal: the cone from degrees (half-angle radians; -1 a sphere).
fn cone_angle(a: f32) -> f32 {
    if a < 0.0 {
        -1.0
    } else if a < 1.0 {
        0.0
    } else if a < 359.0 {
        (std::f64::consts::PI * a as f64 / 360.0) as f32
    } else {
        -1.0
    }
}

/// getSinCos: the game's polynomial sin, and cos through sin(pi/2 - a).
fn sin_cos(ang: f32) -> (f32, f32) {
    let s = (std::f64::consts::FRAC_PI_2 - ang as f64) as f32;
    let a = if ang as f64 > std::f64::consts::FRAC_PI_2 {
        (std::f64::consts::PI - ang as f64) as f32
    } else {
        ang
    };
    let poly = |x: f32| {
        let x2 = x * x;
        x * (x2 * (-0.166_666_67 + x2 * (0.008_333_334 + x2 * -0.000_198_412_7))) + x
    };
    (poly(a), poly(s))
}

/// getOrthoVecs: two vectors across `d` (not normalised: they keep d's size).
fn ortho_vecs(d: Vec3) -> (Vec3, Vec3) {
    let (x, y, z) = (d.x, d.y, d.z);
    if (x as f64) < 0.1 && (y as f64) < 0.1 && z != 0.0 {
        (
            Vec3::new(-z, 0.0, x),
            Vec3::new(y * x, -(z * z) - x * x, y * z),
        )
    } else {
        (
            Vec3::new(y, -x, 0.0),
            Vec3::new(z * x, z * y, -(x * x) - y * y),
        )
    }
}

/// calcLineAvg: a linear rate's average over the start, end or all of its span.
fn line_avg(start: f32, slope: f32, total: f32, calc: f32, section: i32) -> f32 {
    match section {
        1 => {
            let s = start + slope * total;
            (s as f64 + 0.5 * (-slope * calc) as f64) as f32
        }
        2 => (start as f64 + 0.5 * (slope * total) as f64) as f32,
        _ => (start as f64 + 0.5 * (slope * calc) as f64) as f32,
    }
}

impl Psys {
    /// allocPsys's defaults and createPsysNode's dynamic modes (`d`: 0
    /// none, -1 directions, -2 positions, else both).
    fn alloc(d: i32) -> Psys {
        let mut p = Psys {
            flags: 0x4000,
            node_flags: 0,
            e_phase: 0,
            e_delay: 0,
            e_life: 300,
            e_fade: 300,
            e_angle: -1.0,
            e_dir: Vec3::Y,
            e_vol: Vec3::ZERO,
            p_life: 20,
            p_fade: 10,
            p_speed: 0.06,
            p_gravity: 0.0,
            p_drag: 0.0,
            p_max: 0,
            dir_max: 0,
            pos_max: 0,
            e_rate: [1.0; 4],
            e_rate_rand: 0.0,
            p_parms: PARM_DEFAULT.map(|v| [v; 4]),
            tex: None,
            pos_mode: PosMode::Single,
            dir_mode: DirMode::Single,
            ppos: PPos::Linear,
            pos_slots: Vec::new(),
            dir_slots: Vec::new(),
            pos_use: Vec::new(),
            dir_use: Vec::new(),
            pos_next: 0,
            pos_last: 0xFFFF,
            dir_next: 0,
            dir_last: 0xFFFF,
            age_mask: 0,
            ring: VecDeque::new(),
            oldest_age: 0,
            newest_age: 0,
            save_cnt: 0.0,
            e_age: 0,
            e_last_time: 0,
            e_isvis: false,
            p_nactive: 0,
            pool_bytes: 0,
            world: false,
            node: Mat4::IDENTITY,
            scale: 1.0,
            hidden: false,
        };
        match d {
            0 => {}
            -1 => p.flags |= flags::DIR_FRAME,
            -2 => p.flags |= flags::POS_FRAME,
            _ => p.flags |= flags::DIR_FRAME | flags::POS_FRAME,
        }
        p
    }

    /// MBNewPsysDefault: a psys without a record (the setters fill it in).
    pub fn new_default(d: i32) -> Psys {
        Self::alloc(d)
    }

    /// MBNewWorldPsys: a psys from a record, `over` the direction it takes
    /// when the record has none; a named (world) one runs forever.
    pub fn from_record(
        rec: &PsysRecord,
        presets: &[PsysRecord],
        over: Option<[f32; 3]>,
        d: i32,
        named: bool,
    ) -> Psys {
        let mut p = Self::alloc(d);
        p.world = named;
        p.set_world_parms(rec, presets, over, 0);
        if named {
            p.flags |= flags::FOREVER;
        }
        p
    }

    /// setWorldParms: apply a record's enabled fields, its preset first.
    fn set_world_parms(
        &mut self,
        wp: &PsysRecord,
        presets: &[PsysRecord],
        over: Option<[f32; 3]>,
        depth: u32,
    ) {
        if wp.version < 0x100 {
            return;
        }
        let used = wp.enables;
        if used & en::PRESET != 0 && depth < 2 {
            if let Some(pre) = presets
                .iter()
                .take_while(|p| p.preset != -1)
                .find(|p| p.preset == wp.preset)
            {
                self.set_world_parms(pre, presets, None, depth + 1);
            }
        }
        if used & en::MAX_PARTICLES != 0 {
            self.p_max = wp.max_particles as u16;
        }
        if used & en::MAX_POSITIONS != 0 {
            self.pos_max = wp.max_positions as u16;
        }
        if used & en::MAX_DIRECTIONS != 0 {
            self.dir_max = wp.max_directions as u16;
        }
        if used & en::E_LIFEFADE != 0 {
            self.e_life = frames16(wp.e_lifefade[0], 1.0);
            self.e_fade = frames16(wp.e_lifefade[1], 0.0);
            if wp.e_lifefade[0] < 0.0 {
                self.e_life = 0xFFFF;
                self.flags |= flags::FOREVER;
            } else if wp.e_lifefade[1] < 0.0 {
                self.e_fade = 0xFFFF;
                self.flags |= flags::FOREVER;
            }
        }
        if used & en::P_LIFEFADE != 0 {
            self.p_life = frames8(wp.p_lifefade[0], 1.0);
            self.p_fade = frames8(wp.p_lifefade[1], 0.0);
        }
        if used & en::E_ANGLE != 0 {
            self.e_angle = cone_angle(wp.e_angle);
        }
        if used & en::E_DELAY != 0 {
            let v = 30.0 * wp.e_delay as f64;
            self.e_delay = if v > 0.0 { v as i32 as u16 } else { 0 };
        }
        self.e_dir = if used & en::E_DIR != 0 {
            Vec3::from(wp.e_dir)
        } else {
            match over {
                Some(o) if o != [0.0; 3] => Vec3::from(o),
                _ => Vec3::Y,
            }
        };
        if used & en::E_VOL != 0 {
            self.e_vol = Vec3::from(wp.e_vol);
        }
        if used & en::E_RATE != 0 {
            self.e_rate = wp.e_rate.map(|r| ((1.0 / 30.0) * r as f64) as f32);
        }
        if used & en::E_RATE_RAND != 0 {
            self.e_rate_rand = (0.01 * wp.e_rate_rand as f64) as f32;
        }
        if used & en::GRAVITY != 0 {
            self.p_gravity = ((-32.0 / 900.0) * wp.gravity as f64) as f32;
        }
        if used & en::DRAG != 0 {
            self.p_drag = -wp.drag;
        }
        if used & en::SPEED != 0 {
            self.p_speed = ((1.0 / 30.0) * wp.speed as f64) as f32;
        }
        let lane =
            |c: u32, shift: u32| (0.003_921_568_627_450_98 * ((c >> shift) & 0xFF) as f64) as f32;
        if used & en::RGB != 0 {
            for (parm, shift) in [(P_RED, 16), (P_GREEN, 8), (P_BLUE, 0)] {
                self.set_p_parm(wp.rgba.map(|c| lane(c, shift)), parm);
            }
        }
        if used & en::ALPHA != 0 {
            self.set_p_parm(wp.rgba.map(|c| lane(c, 24)), P_INTENSITY);
        }
        if used & en::WIDTH != 0 {
            self.set_p_parm(wp.width, P_WIDTH);
        }
        if wp.flags & 8 != 0 && self.p_gravity == 0.0 {
            self.p_gravity = -0.035_555_556;
        }
        if wp.flags & 0x10 != 0 && self.p_drag == 0.0 {
            self.p_drag = -1.0;
        }
        let (mask, val) = (wp.flag_mask, wp.flags);
        let set16 = |f: u16, m: u32, bit: u16| if val & m != 0 { f | bit } else { f & !bit };
        let mut fl = self.flags;
        if mask & 1 != 0 {
            fl = set16(fl, 1, flags::POS_FRAME | flags::DIR_FRAME);
        }
        if mask & 2 != 0 {
            fl = set16(fl, 2, flags::ONESHOT);
        }
        if mask & 4 != 0 {
            fl = set16(fl, 4, flags::FOREVER);
        }
        if mask & 0x40 != 0 {
            fl = set16(fl, 0x40, flags::NOTEX_A);
        }
        if mask & 0x20 != 0 {
            fl = set16(fl, 0x20, flags::NOTEX_RGB);
        }
        self.flags = fl;
        let set32 = |f: u32, m: u32, bit: u32| if val & m != 0 { f | bit } else { f & !bit };
        let mut nf = self.node_flags;
        for (m, bit) in [
            (0x80, node_flags::FB_ADD),
            (0x100, node_flags::FB_MUL),
            (0x200, node_flags::SORT),
            (0x400, node_flags::NO_ZCOMPARE),
            (0x800, node_flags::NO_ZWRITE),
        ] {
            if mask & m != 0 {
                nf = set32(nf, m, bit);
            }
        }
        self.node_flags = nf;
    }

    fn settable(&self) -> bool {
        // "Setting PSYS attribute after draw begins".
        self.e_phase <= 1
    }

    /// MBPsysSetPParm: a parameter's four keys, scaled and clamped.
    pub fn set_p_parm(&mut self, v: [f32; 4], parm: usize) {
        if !self.settable() || parm >= 5 {
            return;
        }
        let (sc, lo, hi) = (PARM_SCALE[parm], PARM_MIN[parm], PARM_MAX[parm]);
        self.p_parms[parm] = v.map(|x| {
            let a = x * sc;
            if a < lo {
                lo
            } else if a > hi {
                hi
            } else {
                a
            }
        });
    }

    /// MBPsysScalePParm.
    pub fn scale_p_parm(&mut self, s: f32, parm: usize) {
        if self.settable() && parm < 5 {
            for v in &mut self.p_parms[parm] {
                *v *= s;
            }
        }
    }

    /// MBPsysSetPTime (it sets the cone, degrees).
    pub fn set_p_time(&mut self, deg: f32) {
        if self.settable() {
            self.e_angle = cone_angle(deg);
        }
    }

    /// MBPsysSetPSpeed: units a second.
    pub fn set_p_speed(&mut self, v: f32) {
        if self.settable() {
            self.p_speed = (v as f64 * (1.0 / 30.0)) as f32;
        }
    }

    /// MBPsysSetERate4: particles a second.
    pub fn set_e_rate4(&mut self, r: [f32; 4]) {
        if self.settable() {
            self.e_rate = r.map(|v| (v as f64 * (1.0 / 30.0)) as f32);
        }
    }

    /// MBPsysSetEVolume (it sets the particle life and fade, seconds).
    pub fn set_e_volume(&mut self, life: f32, fade: f32) {
        if self.settable() {
            self.p_life = frames8(life, 1.0);
            self.p_fade = frames8(fade, 0.0);
        }
    }

    /// MBPsysSetETime: emit for `dur` then fade over `rep` seconds; a
    /// negative one runs forever.
    pub fn set_e_time(&mut self, dur: f32, rep: f32) {
        if !self.settable() {
            return;
        }
        self.e_life = frames16(dur, 1.0);
        self.e_fade = frames16(rep, 0.0);
        self.flags &= !flags::ONESHOT;
        if dur < 0.0 {
            self.e_life = 0xFFFF;
            self.flags |= flags::FOREVER;
        } else if rep < 0.0 {
            self.e_fade = 0xFFFF;
            self.flags |= flags::FOREVER;
        }
    }

    /// MBRemovePsys: stop emitting; it goes once its particles have.
    pub fn remove(&mut self) {
        if self.e_phase < 6 {
            self.e_phase = 6;
        }
    }

    pub fn dead(&self) -> bool {
        self.e_phase >= 8
    }

    /// Live particles.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// setupParms: the rate and parameter keys become start and slope; the
    /// widest width for the cull.
    fn setup_parms(&mut self) {
        if self.e_life == 0 {
            self.e_life = 1;
        }
        let fade_rate = if self.e_fade == 0 {
            0.0
        } else {
            (1.0 / self.e_fade as f64) as f32
        };
        let [ls, le, fs, fe] = self.e_rate;
        let life_rate = (1.0 / self.e_life as f64) as f32;
        let fade_prod = self.e_life as f32 * fade_rate;
        self.e_rate[0] = ls;
        self.e_rate[1] = life_rate * (le - ls);
        if fade_rate == 0.0 {
            self.e_rate[2] = fs;
            self.e_rate[3] = 0.0;
        } else {
            self.e_rate[2] = fade_prod * (fs - fe) + fs;
            self.e_rate[3] = fade_rate * (fe - fs);
        }
        if self.p_life == 0 {
            self.p_life = 1;
        }
        let pfade_rate = if self.p_fade == 0 {
            0.0
        } else {
            (1.0 / self.p_fade as f64) as f32
        };
        let plife_rate = (1.0 / self.p_life as f64) as f32;
        let pfade_prod = self.p_life as f32 * pfade_rate;
        for parm in &mut self.p_parms {
            let [ls, le, fs, fe] = *parm;
            parm[0] = ls;
            parm[1] = if plife_rate == 0.0 || ls == le {
                0.0
            } else {
                plife_rate * (le - ls)
            };
            if pfade_rate == 0.0 || fs == fe {
                parm[2] = fs;
                parm[3] = 0.0;
            } else {
                parm[2] = pfade_prod * (fs - fe) + fs;
                parm[3] = pfade_rate * (fe - fs);
            }
        }
    }

    /// setupNewPMode: the ring size, the generators and their slot counts,
    /// and the integrator. False when it cannot be built.
    fn setup_new_pmode(&mut self, cx: &mut Ctx) -> bool {
        let fl = self.flags;
        let mut est = 0.0f32;
        let mut pl = 0.0f32;
        let _ = &mut pl;
        let n: i32 = if fl & flags::ONESHOT != 0 || self.p_max != 0 {
            if self.p_max == 0 {
                (30.0 * self.e_rate[0] as f64) as i32
            } else {
                self.p_max as i32
            }
        } else {
            pl = self.p_life as f32 + self.p_fade as f32;
            let el = self.e_life as f32 + self.e_fade as f32;
            let (elife, efade) = (self.e_life as f32, self.e_fade as f32);
            let [a0, c0, b0, d0] = self.e_rate;
            let mut part1 = el;
            if el > pl {
                if c0 <= 0.0 && d0 < 0.0 {
                    est = if pl > elife {
                        let rem = pl - elife;
                        let q0 = line_avg(a0, c0, elife, elife, 2);
                        let q1 = line_avg(b0, d0, efade, rem, 0) * rem;
                        (q0 * elife + q1) / pl
                    } else {
                        line_avg(a0, c0, elife, pl, 0)
                    };
                } else if c0 <= 0.0 {
                    part1 = if pl > elife {
                        let rem = pl - elife;
                        let q0 = line_avg(a0, c0, elife, elife, 2);
                        let q1 = line_avg(b0, d0, efade, rem, 0) * rem;
                        (q0 * elife + q1) / pl
                    } else {
                        line_avg(a0, c0, elife, pl, 0)
                    };
                    est = if pl > efade {
                        let rem = pl - efade;
                        let q2 = line_avg(a0, c0, elife, rem, 1);
                        let q1 = line_avg(b0, d0, efade, efade, 2) * efade;
                        (q2 * rem + q1) / pl
                    } else {
                        line_avg(b0, d0, efade, pl, 1)
                    };
                    if part1 > est {
                        est = part1;
                    }
                } else if d0 < 0.0 {
                    let x = -(d0 / b0);
                    let mut up = pl * x;
                    let mut dn = pl - up;
                    if up > elife {
                        up = elife;
                        dn = pl - elife;
                    } else if dn > efade {
                        dn = efade;
                        up = pl - efade;
                    }
                    let q2 = line_avg(a0, c0, elife, up, 1);
                    let q1 = line_avg(b0, d0, efade, dn, 0) * dn;
                    est = (q2 * up + q1) / pl;
                } else {
                    est = if pl > efade {
                        let rem = pl - efade;
                        let q2 = line_avg(a0, c0, elife, rem, 1);
                        let q1 = line_avg(b0, d0, efade, efade, 2) * efade;
                        (q2 * rem + q1) / pl
                    } else {
                        line_avg(b0, d0, efade, pl, 1)
                    };
                }
            } else {
                pl = part1;
                let q0 = line_avg(a0, c0, elife, elife, 2);
                let q1 = line_avg(b0, d0, efade, efade, 2) * efade;
                est = (q0 * elife + q1) / part1;
            }
            let n32 = (est * pl) as i32;
            let lim = if self.p_max == 0 {
                300
            } else {
                self.p_max as i32
            };
            n32.min(lim)
        };
        // [CHOICE] The frame modes size their slot cycles by the particles'
        // life (frames); the reconstruction leaves `pl` 0 when p_max is
        // given, which would leave them no slot to emit from.
        let frame_slots = if pl == 0.0 {
            self.p_life as f32 + self.p_fade as f32
        } else {
            pl
        };
        let n = n.max(1);

        let has_vol = self.e_vol != Vec3::ZERO;
        let angle_mode = |a: f32| {
            if a == 0.0 {
                0
            } else if a == -1.0 {
                2
            } else {
                4
            }
        };
        let mut dir_mode = angle_mode(self.e_angle) | (fl & flags::DIR_FRAME != 0) as i32;
        let mut pos_mode = if has_vol { 2 } else { 0 } | (fl & flags::POS_FRAME != 0) as i32;
        if fl & flags::ONESHOT != 0 {
            dir_mode = match angle_mode(self.e_angle) {
                0 => 0,
                2 => 3,
                _ => 5,
            };
            pos_mode = if has_vol { 3 } else { 0 };
        }
        let spread = |n: i32| -> i32 {
            let t = if pl < 10.0 { n } else { (10.0 * est) as i32 };
            t.max(8).clamp(1, n)
        };

        let pm = self.pos_max as i32;
        let pos_cap = if pm != 0 { pm } else { 100 }.min(n);
        let (mut n_pos, pos_mode) = match pos_mode {
            0 => (1, PosMode::Single),
            1 => (frame_slots as i32, PosMode::Frame),
            2 => (if pm != 0 { pm } else { spread(n) }, PosMode::RectUnique),
            _ => (n, PosMode::RectShare),
        };
        n_pos = n_pos.min(pos_cap);
        let dm = self.dir_max as i32;
        let mut dir_cap = if dm != 0 { dm } else { 31 }.min(n);
        let (mut n_dir, dir_mode) = match dir_mode {
            0 => (1, DirMode::Single),
            1 => (frame_slots as i32, DirMode::Frame),
            2 | 3 => {
                dir_cap = SPHERE_ENTRIES as i32;
                (SPHERE_ENTRIES as i32, DirMode::Sphere)
            }
            4 => (if dm != 0 { dm } else { spread(n) }, DirMode::ConeUnique),
            _ => (n, DirMode::ConeShare),
        };
        n_dir = n_dir.min(dir_cap).clamp(1, 31);
        let lim = if self.pos_max != 0 {
            self.pos_max as i32
        } else {
            100
        };
        n_pos = n_pos.min(lim);

        let bits = (dir_mode == DirMode::Sphere) as u32
            | ((self.p_drag != 0.0) as u32) << 1
            | ((self.p_gravity != 0.0) as u32) << 2;
        self.ppos = match bits {
            0 | 2 => PPos::Linear,
            1 | 3 => PPos::Speed,
            4 | 6 => PPos::Grav,
            _ => PPos::SpeedGrav,
        };
        self.p_max = n as u16;
        self.pos_max = n_pos.max(0) as u16;
        self.dir_max = n_dir as u16;
        self.pos_mode = pos_mode;
        self.dir_mode = dir_mode;
        let own_dirs = dir_mode != DirMode::Sphere;
        let pos_use = pos_mode == PosMode::RectUnique;
        let dir_use = dir_mode == DirMode::ConeUnique;
        // allocPsysMem's buffer: ring, slots and use counts.
        let total = ((n + 1) & !1) * 2
            + if own_dirs { n_dir * 12 } else { 0 }
            + n_pos.max(0) * 12
            + if dir_use { n_dir } else { 0 }
            + if pos_use { n_pos.max(0) } else { 0 };
        if !self.world {
            let need = (total + 0x1F) & !0xF;
            if need > *cx.pool {
                return false;
            }
            *cx.pool -= need;
            self.pool_bytes += need;
        }
        self.pos_slots = vec![Vec3::ZERO; n_pos.max(0) as usize];
        self.dir_slots = if own_dirs {
            vec![Vec3::ZERO; n_dir as usize]
        } else {
            Vec::new()
        };
        self.pos_use = if pos_use {
            vec![0; n_pos.max(0) as usize]
        } else {
            Vec::new()
        };
        self.dir_use = if dir_use {
            vec![0; n_dir as usize]
        } else {
            Vec::new()
        };
        let bits_for = |v: i32| (0..16).find(|b| (1i32 << b) > v).unwrap_or(16);
        let (dir_bits, pos_bits) = (bits_for(n_dir), bits_for(n_pos.max(0)));
        let age_bits = 16 - dir_bits - pos_bits;
        if age_bits < 1 {
            if self.e_phase < 7 {
                self.e_phase = 7;
            }
            return true;
        }
        self.age_mask = ((1u32 << age_bits) - 1) as u16;
        self.dir_next = (n_dir - 1) as u16;
        self.pos_next = (n_pos - 1) as u16;
        self.dir_last = 0xFFFF;
        self.pos_last = 0xFFFF;
        true
    }

    /// getCurrentDir: the base direction in the node's frame at p_speed.
    fn current_dir(&self) -> Vec3 {
        let d = self.node.transform_vector3(self.e_dir);
        let mag = d.length_squared();
        let s = if (mag as f64) < 0.7 || (mag as f64) > 1.3 {
            self.p_speed / mag.sqrt()
        } else {
            self.p_speed
        };
        d * s
    }

    /// The cycling (share / frame) slot allocation: -1 once it would reach
    /// the oldest live particle's slot.
    fn cycle(next: &mut u16, last: u16, max: u16) -> i32 {
        let idx = *next;
        if idx == last {
            return -1;
        }
        *next = if idx == 0 {
            max.wrapping_sub(1)
        } else {
            idx - 1
        };
        idx as i32
    }

    /// The unique-slot allocation: a random slot, else the next one not
    /// full; whether it was already in use.
    fn unique(use_: &mut [u8], count: usize, rng: &mut PbRand) -> (usize, bool) {
        let mut idx =
            (3.051_850_947_599_719e-5 * count as f32 as f64 * rng.rand() as f32 as f64) as usize;
        if idx >= count {
            idx = count - 1;
        }
        if use_[idx] == 0xFF {
            let first = idx;
            loop {
                idx = if idx == 0 { count - 1 } else { idx - 1 };
                if idx == first {
                    return (0, use_[0] > 1);
                }
                if use_[idx] != 0xFF {
                    break;
                }
            }
        }
        use_[idx] += 1;
        (idx, use_[idx] > 1)
    }

    fn rect_pos(&self, cx: &mut Ctx) -> Vec3 {
        let mut r = || ((3.051_850_947_599_719e-5 * cx.rng.rand() as f32 as f64) - 0.5) as f32;
        let v = Vec3::new(self.e_vol.x * r(), self.e_vol.y * r(), self.e_vol.z * r());
        self.node.transform_point3(v)
    }

    /// pos_func: a position slot for a new particle, or -1.
    fn new_pos(&mut self, cx: &mut Ctx) -> i32 {
        match self.pos_mode {
            PosMode::Single => {
                if self.flags & flags::POS_SET != 0 {
                    self.flags &= !flags::POS_SET;
                    self.pos_slots[0] = self.node.w_axis.truncate();
                }
                0
            }
            PosMode::Frame => {
                if cx.pos_slot >= 0 {
                    return cx.pos_slot;
                }
                let i = Self::cycle(&mut self.pos_next, self.pos_last, self.pos_max);
                if i >= 0 {
                    self.pos_slots[i as usize] = self.node.w_axis.truncate();
                    cx.pos_slot = i;
                }
                i
            }
            PosMode::RectShare => {
                let i = Self::cycle(&mut self.pos_next, self.pos_last, self.pos_max);
                if i >= 0 {
                    self.pos_slots[i as usize] = self.rect_pos(cx);
                }
                i
            }
            PosMode::RectUnique => {
                let n = self.pos_use.len();
                let (i, reused) = Self::unique(&mut self.pos_use, n, cx.rng);
                if !reused {
                    self.pos_slots[i] = self.rect_pos(cx);
                }
                i as i32
            }
        }
    }

    /// getNewDirCone*: a direction within e_angle of the current one.
    fn cone_dir(&self, cx: &mut Ctx) -> Vec3 {
        let dir = self.current_dir();
        let (ux, uy) = ortho_vecs(dir);
        let arg =
            (3.051_850_947_599_719e-5 * self.e_angle as f64 * cx.rng.rand() as f32 as f64) as f32;
        let (cs, sn) = sin_cos(arg);
        let arg = (9.587_672_783_631_622e-5 * cx.rng.rand() as f32 as f64) as f32;
        let (mut cs2, mut sn2) = sin_cos(arg);
        if cx.rng.rand() & 4 != 0 {
            cs2 = -cs2;
        }
        cs2 *= cs;
        sn2 *= cs;
        dir * sn + ux * sn2 + uy * cs2
    }

    /// dir_func: a direction slot for a new particle, or -1.
    fn new_dir(&mut self, cx: &mut Ctx) -> i32 {
        match self.dir_mode {
            DirMode::Single => {
                if self.flags & flags::DIR_SET != 0 {
                    self.flags &= !flags::DIR_SET;
                    self.dir_slots[0] = self.current_dir();
                }
                0
            }
            DirMode::Frame => {
                if cx.dir_slot >= 0 {
                    return cx.dir_slot;
                }
                let i = Self::cycle(&mut self.dir_next, self.dir_last, self.dir_max);
                if i >= 0 {
                    self.dir_slots[i as usize] = self.current_dir();
                    cx.dir_slot = i;
                }
                i
            }
            DirMode::ConeShare => {
                let i = Self::cycle(&mut self.dir_next, self.dir_last, self.dir_max);
                if i >= 0 {
                    self.dir_slots[i as usize] = self.cone_dir(cx);
                }
                i
            }
            DirMode::ConeUnique => {
                let n = self.dir_use.len();
                let (i, reused) = Self::unique(&mut self.dir_use, n, cx.rng);
                if !reused {
                    self.dir_slots[i] = self.cone_dir(cx);
                }
                i as i32
            }
            DirMode::Sphere => {
                let n = self.dir_max as usize;
                let (i, reused) = Self::unique(cx.sphere_use, n, cx.rng);
                if !reused {
                    let mut r =
                        || (6.103_701_986_151_684e-6 * cx.rng.rand() as f32 as f64 - 0.1) as f32;
                    let d = Vec3::new(r(), r(), r());
                    let len = d.length();
                    cx.sphere[i] = if len > 0.0 { d * (1.0 / len) } else { d };
                }
                i as i32
            }
        }
    }

    /// A retired particle gives its slots back.
    fn release(&mut self, e: Entry, cx: &mut Ctx) {
        if let Some(u) = self.pos_use.get_mut(e.pos as usize) {
            *u = u.wrapping_sub(1);
        }
        match self.dir_mode {
            DirMode::ConeUnique => {
                if let Some(u) = self.dir_use.get_mut(e.dir as usize) {
                    *u = u.wrapping_sub(1);
                }
            }
            DirMode::Sphere => {
                if let Some(u) = cx.sphere_use.get_mut(e.dir as usize) {
                    *u = u.wrapping_sub(1);
                }
            }
            _ => {}
        }
    }

    /// Retire the psys (listFindHandle onto the retired list).
    fn retire(&mut self, cx: &mut Ctx) {
        while let Some(e) = self.ring.pop_front() {
            self.release(e, cx);
        }
        *cx.pool += self.pool_bytes;
        self.pool_bytes = 0;
        self.e_phase = 8;
    }

    /// MBDrawPsys's simulation at psys frame `frame`: the emitter phases,
    /// retiring and emitting. Drawing is [`Psys::sprites`].
    fn step(&mut self, cx: &mut Ctx, frame: u32) {
        cx.dir_slot = -1;
        cx.pos_slot = -1;
        let mut dt = frame.wrapping_sub(self.e_last_time);
        if dt > 15 {
            dt = 1;
        }
        let age_tmp = self.e_age.wrapping_add(dt);
        self.e_last_time = frame;
        let mut agef = age_tmp as f32;
        let mut age = age_tmp;
        let mut rate;
        let [life_start, life_slope, fade_start, fade_slope];
        let mut ph = self.e_phase;
        if ph == 0 {
            let d = self.e_delay as u32;
            if d != 0 {
                if age <= d {
                    self.e_age = age;
                    return;
                }
                age -= d;
                dt = age;
            }
            ph = 1;
            self.e_phase = 1;
        }
        if ph == 1 {
            self.setup_parms();
            self.flags |= flags::POS_SET | flags::DIR_SET;
            if !self.setup_new_pmode(cx) {
                self.retire(cx);
                return;
            }
            self.ring.clear();
            self.oldest_age = 0;
            self.newest_age = 0;
            self.save_cnt = 0.0;
            if self.e_phase >= 7 {
                self.retire(cx);
                return;
            }
            if dt == 0 {
                dt = 1;
                age += 1;
                agef += 1.0;
            }
            self.e_phase = 2;
            ph = 2;
            if self.flags & flags::ONESHOT != 0 {
                dt = 1;
                self.e_phase = 6;
                ph = 9;
                age = 1;
            } else if self.flags & flags::FOREVER != 0 && self.e_life == 0xFFFF {
                self.e_phase = 4;
                ph = 4;
            }
        }
        [life_start, life_slope, fade_start, fade_slope] = self.e_rate;
        let forever = self.flags & flags::FOREVER != 0;
        loop {
            match ph {
                // The one shot: everything at once.
                9 => {
                    rate = 100_000.0;
                    break;
                }
                4 => {
                    rate = life_start;
                    break;
                }
                5 => {
                    rate = fade_start;
                    break;
                }
                2 => {
                    if age <= self.e_life as u32 {
                        rate = life_slope * agef + life_start;
                        break;
                    }
                    if forever && self.e_fade == 0xFFFF {
                        self.e_phase = 5;
                        ph = 5;
                        continue;
                    }
                    self.e_phase = 3;
                    ph = 3;
                }
                3 => {
                    if age <= self.e_life as u32 + self.e_fade as u32 {
                        rate = fade_slope * agef + fade_start;
                        break;
                    }
                    if forever {
                        self.e_phase = 2;
                        age = 0;
                        rate = life_start;
                        break;
                    }
                    self.e_phase = 6;
                    ph = 6;
                }
                6 => {
                    self.save_cnt = 0.0;
                    rate = 0.0;
                    if self.ring.is_empty() {
                        self.retire(cx);
                        return;
                    }
                    break;
                }
                _ => {
                    self.retire(cx);
                    return;
                }
            }
        }
        if !self.e_isvis || self.hidden {
            rate = 0.0;
        }
        self.e_age = age;
        if rate > 0.0 && self.e_rate_rand > 0.0 {
            let r = cx.rng.rand();
            rate = (rate as f64
                * (self.e_rate_rand as f64 * (6.103_701_895_199_438e-5 * r as f32 as f64 - 1.0)
                    + 1.0)) as f32;
        }
        self.retire_old(cx, dt);
        self.emit(cx, rate, dt);
        self.p_nactive = self.ring.len();
    }

    /// The retire half of MBDrawPsys: particles reaching life + fade go.
    fn retire_old(&mut self, cx: &mut Ctx, dt: u32) {
        let plsum = self.p_life as i32 + self.p_fade as i32;
        let am = self.age_mask as i32;
        let mut over = dt as i32 + self.oldest_age as i32 - plsum;
        if over >= 0 && !self.ring.is_empty() {
            loop {
                let e = self.ring.pop_front().unwrap();
                self.release(e, cx);
                let Some(f) = self.ring.front() else { break };
                over -= am & f.delta as i32;
                if over < 0 {
                    break;
                }
            }
            match self.ring.front() {
                Some(f) => {
                    self.pos_last = f.pos;
                    self.dir_last = f.dir;
                }
                None => {
                    self.pos_last = 0xFFFF;
                    self.dir_last = 0xFFFF;
                }
            }
        }
        self.oldest_age = (plsum + over).max(0) as u32;
    }

    /// The emit half: rate x dt particles (the fraction carried), their ages
    /// spread back over the frames they stand for.
    fn emit(&mut self, cx: &mut Ctx, rate: f32, dt: u32) {
        let mut budget = rate * dt as f32 + self.save_cnt;
        let cap = self.p_max as usize;
        if self.ring.len() >= cap {
            self.save_cnt = 0.0;
            self.newest_age += dt;
            return;
        }
        let had = self.ring.len();
        let start = budget;
        while budget > 0.0 {
            let pos = self.new_pos(cx);
            if pos < 0 {
                break;
            }
            let dir = self.new_dir(cx);
            if dir < 0 {
                break;
            }
            budget = (budget as f64 - 1.0) as f32;
            self.ring.push_back(Entry {
                pos: pos as u16,
                dir: dir as u16,
                delta: 0,
            });
            if self.ring.len() >= cap {
                break;
            }
        }
        self.save_cnt = if budget > 0.0 { 0.0 } else { budget };
        let emitted = self.ring.len() - had;
        if budget == start || emitted == 0 {
            self.newest_age += dt;
            return;
        }
        let am = self.age_mask;
        // From the newest back: a gap each time the accumulated rate runs out.
        let mut acc = rate;
        let mut filled = 0u32;
        for k in (had + 1..self.ring.len()).rev() {
            acc -= 1.0;
            if acc <= 0.0 {
                let mut n = 0u16;
                loop {
                    acc += rate;
                    n += 1;
                    filled += 1;
                    if !(acc <= 0.0 && n != am) {
                        break;
                    }
                }
                self.ring[k].delta |= n;
            }
        }
        if had == 0 {
            self.oldest_age = filled;
            self.newest_age = 0;
            return;
        }
        // The first new one's gap to the previous newest, spilling into the
        // newer ones' when it is past the age bits.
        let mut carry = self.newest_age as i64 + dt as i64 - filled as i64;
        self.newest_age = 0;
        let mut k = had;
        while carry > am as i64 {
            self.ring[k].delta = am;
            k += 1;
            if k == self.ring.len() {
                self.newest_age = carry as u32;
                return;
            }
            carry += (am & self.ring[k].delta) as i64 - am as i64;
        }
        let e = &mut self.ring[k];
        e.delta = (e.delta & !am) | carry.max(0) as u16;
    }

    /// pbBlitSetDrawRegs for this psys: blend, z compare, z write.
    pub fn draw_state(&self) -> (Blend, bool, bool) {
        let nf = self.node_flags;
        let notex_a = self.flags & flags::NOTEX_A != 0;
        let notex_rgb = !notex_a && self.flags & flags::NOTEX_RGB != 0;
        let blend = if nf & node_flags::FB_ADD != 0 {
            if notex_a {
                Blend::AddOne
            } else {
                Blend::Add
            }
        } else if notex_rgb {
            Blend::Darken
        } else if notex_a {
            Blend::Replace
        } else {
            Blend::Alpha
        };
        (
            blend,
            nf & node_flags::NO_ZCOMPARE == 0,
            nf & node_flags::NO_ZWRITE == 0,
        )
    }

    /// psys->ppos_func: where a particle is at age `t` (frames).
    fn place(&self, dir: Vec3, org: Vec3, t: f32) -> Vec3 {
        match self.ppos {
            PPos::Linear => dir * t + org,
            PPos::Speed => (dir * self.p_speed) * t + org,
            PPos::Grav => Vec3::new(
                dir.x * t + org.x,
                t * (self.p_gravity * t + dir.y) + org.y,
                dir.z * t + org.z,
            ),
            PPos::SpeedGrav => Vec3::new(
                t * (self.p_speed * dir.x) + org.x,
                t * (self.p_gravity * t + self.p_speed * dir.y) + org.y,
                t * (self.p_speed * dir.z) + org.z,
            ),
        }
    }

    /// The draw half of MBDrawPsys: every live particle, newest first, at
    /// its place, width and colour (alpha & 0xFE) for its age.
    pub fn sprites(&self, sphere: &[Vec3]) -> Vec<(Vec3, f32, u32)> {
        let mut out = Vec::with_capacity(self.ring.len());
        let plf = self.p_life as f32;
        let mut page = self.newest_age as f32;
        for (i, e) in self.ring.iter().enumerate().rev() {
            // Life keys until p_life, then fade keys.
            let keys = |p: &[f32; 4]| {
                if page < plf {
                    (p[0], p[1])
                } else {
                    (p[2], p[3])
                }
            };
            let mut color = 0u32;
            for (lane, parm) in [P_BLUE, P_GREEN, P_RED, P_INTENSITY]
                .into_iter()
                .enumerate()
            {
                let (start, slope) = keys(&self.p_parms[parm]);
                let v = if slope == 0.0 {
                    start
                } else {
                    slope * page + start
                };
                color |= (v as i32 as u32).wrapping_shl(lane as u32 * 8);
            }
            let (ws, wsl) = keys(&self.p_parms[P_WIDTH]);
            let w = if wsl == 0.0 { ws } else { wsl * page + ws };
            let dir = match self.dir_mode {
                DirMode::Sphere => sphere.get(e.dir as usize).copied().unwrap_or(Vec3::Y),
                _ => self
                    .dir_slots
                    .get(e.dir as usize)
                    .copied()
                    .unwrap_or(Vec3::ZERO),
            };
            let org = self
                .pos_slots
                .get(e.pos as usize)
                .copied()
                .unwrap_or(Vec3::ZERO);
            let pos = self.place(dir, org, page);
            let a = (color >> 24) & 0xFE;
            out.push((
                pos,
                w / PARM_SCALE[P_WIDTH] * self.scale,
                (color & 0xFF_FFFF) | a << 24,
            ));
            if i > 0 {
                page += (self.age_mask & e.delta) as f32;
            }
        }
        out
    }
}

/// A thrown weapon's streak (SfxSetStreak): its poly's colour and alpha, and
/// the effect's maxtime, streakscale and streakfwdmul.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Streak {
    /// 0xRRGGBB.
    pub color: u32,
    /// MBPolyInstSetColorAlpha's alpha (0 opaque .. 255 clear).
    pub alpha: i32,
    pub maxtime: f32,
    pub scale: f32,
    pub fwdmul: f32,
}

impl Streak {
    /// The poly's colour as drawn: DoPolyInstSub's alpha is
    /// ((255 - alpha) >> 1) << 1.
    pub fn argb(&self) -> u32 {
        let a = (((255 - self.alpha.clamp(0, 255)) >> 1) << 1).min(255) as u32;
        (self.color & 0xFF_FFFF) | a << 24
    }
}

/// UpdateFXStreak: the streak's quad behind a missile at `pos` flying at
/// `vel` (a second) for `elapsed` seconds, `cam_z` the camera's view axis:
/// a narrow tail up to half a scale's seconds back, a wide head
/// streakfwdmul frames ahead, across the flight as seen by the camera.
pub fn streak_quad(pos: Vec3, vel: Vec3, elapsed: f32, s: &Streak, cam_z: Vec3) -> [Vec3; 4] {
    let side = (0.8 * s.scale as f64) as f32;
    let tail = (0.1 * s.scale as f64) as f32;
    let max_back = (0.499_999_999_5 * s.scale as f64) as f32;
    let back = if elapsed > max_back {
        max_back
    } else {
        elapsed
    };
    let v = vel.normalize_or_zero();
    let w = v.cross(cam_z);
    let len = w.length();
    let len = if len < 0.25 { 0.25 } else { len };
    let w = w.normalize_or_zero() * len;
    let p0 = vel * -back + pos;
    let p1 = vel * (0.033_333_333_3 * s.fwdmul as f64) as f32 + pos;
    [w * tail + p0, w * side + p1, w * -side + p1, w * -tail + p0]
}

/// One streak's quad this frame.
#[derive(Clone, Copy, Debug)]
pub struct StreakQuad {
    /// Tail +, head +, head -, tail - (the poly's UVs 0,0 1,0 1,1 0,1).
    pub corners: [Vec3; 4],
    pub color: u32,
    pub tex: Option<usize>,
}

/// Where a psys hangs: the game updates its node each frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Owner {
    /// A `PSYS<id>` world object.
    World(usize),
    /// A particle node of an entity's tree.
    Entity { id: u64, model: usize, node: usize },
    /// A particle node of a tree a hero wears (power-ups, held items).
    Worn {
        player: usize,
        model: usize,
        node: usize,
    },
    /// PsfxDoParticle on a hero (a node of its model, else its root) at an
    /// offset in that node's frame.
    Hero {
        player: usize,
        node: Option<usize>,
        offset: [f32; 3],
    },
    /// Removed from its owner: it only lives out its particles.
    Detached,
}

/// Every particle system of the level, with what their draws share.
#[derive(Clone)]
pub struct Particles {
    /// MBPsysStartFrame's frame counter.
    pub frame: u32,
    rng: PbRand,
    sphere: Vec<Vec3>,
    sphere_use: Vec<u8>,
    pool: i32,
    pub systems: Vec<(Owner, Psys)>,
    /// Particle textures by registry index, and their names.
    pub textures: Vec<gdl_formats::Image>,
    names: Vec<String>,
    /// allocPsys: particle2_a and particle2_xp (fb_add's).
    pub default_tex: [Option<usize>; 2],
    /// main.dol's psysPresetTable.
    pub presets: Vec<PsysRecord>,
    /// WeaponStreakTex (WEP_STREAK) and this frame's streaks.
    pub streak_tex: Option<usize>,
    pub streaks: Vec<StreakQuad>,
    /// Each live missile's streak, by entity id.
    pub streak_of: std::collections::BTreeMap<u64, Streak>,
    /// Particle-node textures resolved per (model, node).
    pub tex_cache: std::collections::BTreeMap<(usize, usize), Option<usize>>,
    /// Trees whose particle nodes were built (AtreeNodeInit runs once per
    /// instance, even when the pool was out): (entity id, model).
    pub started: std::collections::BTreeSet<(u64, usize)>,
}

impl Default for Particles {
    fn default() -> Self {
        Particles {
            frame: 0,
            rng: PbRand(0x1234_5678),
            sphere: vec![Vec3::Y; SPHERE_ENTRIES],
            sphere_use: vec![0; SPHERE_ENTRIES],
            pool: POOL_BYTES,
            systems: Vec::new(),
            textures: Vec::new(),
            names: Vec::new(),
            default_tex: [None; 2],
            presets: presets(),
            streak_tex: None,
            streaks: Vec::new(),
            streak_of: Default::default(),
            tex_cache: Default::default(),
            started: Default::default(),
        }
    }
}

impl Particles {
    /// Add a psys (allocPsys: none once the pool is out, unless it is a
    /// world one, which comes from the level's arena).
    pub fn add(&mut self, owner: Owner, mut p: Psys) -> bool {
        if !p.world {
            let need = (PSYS_BYTES + 0x1F) & !0xF;
            if need > self.pool {
                return false;
            }
            self.pool -= need;
            p.pool_bytes = need;
        }
        p.e_last_time = self.frame;
        self.systems.push((owner, p));
        true
    }

    /// A texture by name, registering `img` under it once.
    pub fn texture(&mut self, name: &str, img: gdl_formats::Image) -> usize {
        if let Some(i) = self
            .names
            .iter()
            .zip(&self.textures)
            .position(|(n, t)| n == name && t.width == img.width && t.rgba == img.rgba)
        {
            return i;
        }
        self.names.push(name.to_string());
        self.textures.push(img);
        self.textures.len() - 1
    }

    /// The texture already registered under `name`, if any.
    pub fn texture_named(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.eq_ignore_ascii_case(name))
    }

    /// One game tick: MBPsysStartFrame's clock (one step a 30 Hz frame:
    /// `frame` is the psys frame now), then MBDrawPsysTest and MBDrawPsys
    /// for each psys. `visible` is MBWorldSphereVisible3 on a node.
    pub fn update(&mut self, frame: u32, visible: impl Fn(Vec3, f32) -> bool) {
        self.frame = frame;
        let mut cx = Ctx {
            rng: &mut self.rng,
            sphere: &mut self.sphere,
            sphere_use: &mut self.sphere_use,
            pool: &mut self.pool,
            dir_slot: -1,
            pos_slot: -1,
        };
        for (_, p) in &mut self.systems {
            let vis = p.e_phase >= 6 || visible(p.node.w_axis.truncate(), MAX_DIST);
            p.e_isvis = vis;
            if !vis && p.p_nactive == 0 {
                continue;
            }
            p.step(&mut cx, frame);
        }
        self.systems.retain(|(_, p)| !p.dead());
    }

    /// This frame's draws: up to [`MAX_DRAWN`] psys in view, each one batch.
    pub fn batches(&self) -> Vec<SpriteBatch> {
        let mut out = Vec::new();
        for (_, p) in &self.systems {
            if out.len() >= MAX_DRAWN {
                break;
            }
            if !p.e_isvis && p.p_nactive == 0 || p.ring.is_empty() || p.e_phase < 2 {
                continue;
            }
            let (blend, ztest, zwrite) = p.draw_state();
            let tex = p.tex.or_else(|| {
                if p.node_flags & node_flags::FB_ADD != 0 && self.default_tex[1].is_some() {
                    self.default_tex[1]
                } else {
                    self.default_tex[0]
                }
            });
            out.push(SpriteBatch {
                tex,
                blend,
                ztest,
                zwrite,
                sprites: p.sprites(&self.sphere),
            });
        }
        out
    }

    /// MBRemovePsys on everything an owner has.
    pub fn remove_owner(&mut self, owner: &Owner) {
        for (o, p) in &mut self.systems {
            if o == owner {
                p.remove();
                *o = Owner::Detached;
            }
        }
    }
}

/// psysPresetTable (main.dol 0x801287FC): the presets records inherit
/// (version 0x100; every one 999 s of emitter life).
pub fn presets() -> Vec<PsysRecord> {
    let p = |preset: i16,
             flags: u32,
             enables: u32,
             p_lifefade: [f32; 2],
             e_angle: f32,
             e_rate: f32,
             e_rate_rand: f32,
             gravity: f32,
             speed: f32,
             rgba: [u32; 4],
             width: [f32; 4]| PsysRecord {
        version: 0x100,
        preset,
        flags,
        flag_mask: flags,
        enables,
        e_lifefade: [999.0, 999.0],
        p_lifefade,
        e_angle,
        e_rate: [e_rate; 4],
        e_rate_rand,
        gravity,
        speed,
        rgba,
        width,
        ..Default::default()
    };
    let rgb_fire = [0xFFFF_0000, 0xFF00_00FF, 0xFF00_00FF, 0x0000_00FF];
    let white = [0xFFFF_FFFF; 4];
    let white_out = [0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF, 0x00FF_FFFF];
    let narrow = [0.1, 1.0, 1.0, 0.1];
    let puff = [1.0, 3.0, 3.0, 7.0];
    vec![
        p(
            0,
            0x800,
            0x72270,
            [0.4, 0.4],
            360.0,
            100.0,
            0.0,
            0.0,
            10.0,
            rgb_fire,
            narrow,
        ),
        p(
            1,
            0x800,
            0x72670,
            [10.0, 5.0],
            0.0,
            1.0,
            1.0,
            0.0,
            15.0,
            white,
            [1.0; 4],
        ),
        p(
            2,
            0x800,
            0x72270,
            [0.4, 0.3],
            20.0,
            60.0,
            0.0,
            0.0,
            15.0,
            white_out,
            [0.3; 4],
        ),
        p(
            3,
            0x800,
            0x72270,
            [1.0, 5.0],
            40.0,
            7.0,
            0.0,
            0.0,
            3.0,
            white_out,
            puff,
        ),
        p(
            4,
            0x808,
            0x72270,
            [0.4, 0.4],
            360.0,
            100.0,
            0.0,
            0.0,
            10.0,
            rgb_fire,
            narrow,
        ),
        p(
            5,
            0x808,
            0x72670,
            [10.0, 5.0],
            0.0,
            1.0,
            1.0,
            0.0,
            45.0,
            white,
            [1.0; 4],
        ),
        p(
            6,
            0x808,
            0x72270,
            [1.0, 0.0],
            20.0,
            60.0,
            0.0,
            0.0,
            30.0,
            white,
            [0.3; 4],
        ),
        p(
            7,
            0x808,
            0x72a70,
            [1.0, 5.0],
            360.0,
            7.0,
            0.0,
            -0.1,
            3.0,
            white,
            puff,
        ),
        PsysRecord {
            version: 0x100,
            preset: -1,
            ..Default::default()
        },
    ]
}

#[cfg(test)]
mod tests {
    use crate::math::Det as _;
    use super::*;

    fn ctx<'a>(
        rng: &'a mut PbRand,
        s: &'a mut [Vec3],
        u: &'a mut [u8],
        pool: &'a mut i32,
    ) -> Ctx<'a> {
        Ctx {
            rng,
            sphere: s,
            sphere_use: u,
            pool,
            dir_slot: -1,
            pos_slot: -1,
        }
    }

    #[test]
    fn presets_match_main_dol() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data/main.dol");
        let Ok(d) = std::fs::read(p) else { return };
        let dol = gdl_formats::dol::Dol::new(d);
        let mine = presets();
        for (k, want) in mine.iter().enumerate() {
            let at = 0x8012_87FC + (k * gdl_formats::psys::PSYS_RECORD_SIZE) as u32;
            let bytes: Vec<u8> = (0..gdl_formats::psys::PSYS_RECORD_SIZE as u32 / 4)
                .flat_map(|w| dol.u32_at(at + w * 4).unwrap().to_be_bytes())
                .collect();
            let got = PsysRecord::parse_be(&bytes).unwrap();
            assert_eq!(&got, want, "preset {k}");
        }
        // psysInfo's parameter scales.
        let f = |a: u32| dol.f32_at(a).unwrap();
        for k in 0..5u32 {
            let b = 0x8012_87AC + 16 * k;
            let i = k as usize;
            assert_eq!(
                [f(b), f(b + 4), f(b + 8), f(b + 12)],
                [PARM_SCALE[i], PARM_MIN[i], PARM_MAX[i], PARM_DEFAULT[i]]
            );
        }
    }

    #[test]
    fn world_torch_inherits_its_preset() {
        // H1's 'A' torch: preset 5, its own life, cone, speed and colours.
        let rec = PsysRecord {
            version: 0x101,
            preset: 5,
            id: b'A',
            flags: 0x288,
            flag_mask: 0x288,
            enables: 0x56be1,
            p_lifefade: [0.2, 0.22],
            e_angle: 80.0,
            tex_name: "P_TORCH".into(),
            e_dir: [0.0, 1.0, 0.0],
            e_vol: [0.1, 0.3, 0.1],
            e_rate: [25.0; 4],
            gravity: -0.38,
            speed: 4.0,
            rgba: [0, 0xFF_FFFF, 0xFF_FFFF, 0],
            width: [2.8, 2.0, 2.0, 0.1],
            ..Default::default()
        };
        let p = Psys::from_record(&rec, &presets(), Some([0.0, 1.0, 0.0]), 0, true);
        // The preset's 999 s emitter, forever by name.
        assert_eq!((p.e_life, p.e_fade), (29970, 29970));
        assert!(p.flags & flags::FOREVER != 0);
        assert_eq!((p.p_life, p.p_fade), (6, 6));
        assert!((p.e_angle - 80f32.to_radians() / 2.0).abs() < 1e-6);
        assert!((p.p_speed - 4.0 / 30.0).abs() < 1e-7);
        assert!((p.p_gravity - 0.38 * 32.0 / 900.0).abs() < 1e-7);
        // The preset's rate jitter (1 %), the record's 25 a second.
        assert!((p.e_rate_rand - 0.01).abs() < 1e-7 && (p.e_rate[0] - 25.0 / 30.0).abs() < 1e-6);
        // RGB from the record, alpha from the preset (white), width x 450.
        assert_eq!(p.p_parms[P_RED], [0.0, 255.0, 255.0, 0.0]);
        assert_eq!(p.p_parms[P_INTENSITY], [255.0; 4]);
        assert_eq!(p.p_parms[P_WIDTH], [1260.0, 900.0, 900.0, 45.0]);
        // fb_add and sort from the record, no z write from the preset.
        assert_eq!(
            p.node_flags,
            node_flags::FB_ADD | node_flags::SORT | node_flags::NO_ZWRITE
        );
        assert_eq!(p.draw_state(), (Blend::Add, true, false));
    }

    #[test]
    fn emission_carries_fractions_and_retires_on_time() {
        // Half a particle a frame, life 4 + fade 2 frames, a line along +x.
        let mut p = Psys::new_default(0);
        p.set_e_rate4([15.0; 4]);
        p.set_e_volume(4.0 / 30.0, 2.0 / 30.0);
        p.set_p_time(0.0);
        p.set_e_time(-1.0, 0.0);
        p.e_dir = Vec3::X;
        p.set_p_speed(30.0);
        p.e_isvis = true;
        let (mut rng, mut s, mut u, mut pool) =
            (PbRand(1), vec![Vec3::Y; 1023], vec![0; 1023], POOL_BYTES);
        let mut cx = ctx(&mut rng, &mut s, &mut u, &mut pool);
        let mut counts = Vec::new();
        for f in 1..=12 {
            p.step(&mut cx, f);
            counts.push(p.len());
        }
        // One every other frame; each lives 6 frames, so three at once.
        assert_eq!(counts, [1, 1, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3]);
        // Ages 0, 2, 4 frames: one unit a frame along +x from the origin.
        let xs: Vec<f32> = p.sprites(&[]).iter().map(|s| s.0.x).collect();
        assert_eq!(xs, [1.0, 3.0, 5.0]);
    }

    #[test]
    fn colour_and_width_follow_life_then_fade() {
        let mut p = Psys::new_default(0);
        p.set_e_rate4([30.0; 4]);
        p.set_e_volume(10.0 / 30.0, 10.0 / 30.0);
        p.set_p_time(0.0);
        p.set_e_time(-1.0, 0.0);
        // Alpha 1 -> 0.5 over life, 0.5 -> 0 over fade; width 1 -> 2.
        p.set_p_parm([1.0, 0.5, 0.5, 0.0], P_INTENSITY);
        p.set_p_parm([1.0, 2.0, 2.0, 2.0], P_WIDTH);
        p.e_isvis = true;
        let (mut rng, mut s, mut u, mut pool) =
            (PbRand(1), vec![Vec3::Y; 1023], vec![0; 1023], POOL_BYTES);
        let mut cx = ctx(&mut rng, &mut s, &mut u, &mut pool);
        for f in 1..=16 {
            p.step(&mut cx, f);
        }
        let s = p.sprites(&[]);
        // Newest first: ages 0..15.
        assert_eq!(s.len(), 16);
        let alpha = |k: usize| s[k].2 >> 24;
        assert_eq!(alpha(0), 254); // 255 & 0xFE
        assert_eq!(alpha(5), (255.0 - 12.75 * 5.0) as u32 & 0xFE);
        assert_eq!(alpha(15), (127.5 - 12.75 * 5.0) as u32 & 0xFE);
        assert!((s[0].1 - 1.0).abs() < 1e-6 && (s[5].1 - 1.5).abs() < 1e-6);
        assert!((s[12].1 - 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_burst_ends_and_retires() {
        // BLOODFX-like: 0.3 s of emitting, then the fade's zero rate.
        let rec = PsysRecord {
            version: 0x101,
            preset: 5,
            enables: 0x66a71,
            flags: 8,
            flag_mask: 8,
            e_lifefade: [0.3, 2.0],
            p_lifefade: [0.3, 0.8],
            e_angle: 180.0,
            e_rate: [25.0, 25.0, 0.0, 0.0],
            gravity: 0.7,
            speed: 7.0,
            rgba: [0xFFFF_FFFF; 4],
            width: [0.2; 4],
            ..Default::default()
        };
        let mut p = Psys::from_record(&rec, &presets(), Some([0.0, 0.5, 0.866]), 1, false);
        assert_eq!((p.e_life, p.e_fade, p.p_life, p.p_fade), (9, 60, 9, 24));
        p.e_isvis = true;
        let (mut rng, mut s, mut u, mut pool) =
            (PbRand(7), vec![Vec3::Y; 1023], vec![0; 1023], POOL_BYTES);
        let mut cx = ctx(&mut rng, &mut s, &mut u, &mut pool);
        let mut most = 0;
        let mut f = 0;
        while !p.dead() && f < 200 {
            f += 1;
            p.step(&mut cx, f);
            most = most.max(p.len());
        }
        // About 25/30 x 9 frames of particles, all gone by emitter life +
        // fade (69 frames) plus a particle's 33.
        assert!((6..=9).contains(&most), "{most}");
        assert!(p.dead() && f <= 69 + 34, "{f}");
        // Its buffer went back to the pool.
        assert_eq!(*cx.pool, POOL_BYTES);
    }

    #[test]
    fn streak_trails_its_missile() {
        let s = Streak {
            color: 0xFF_0000,
            alpha: 0x40,
            maxtime: 3.0,
            scale: 1.0,
            fwdmul: 1.0,
        };
        // 190 / 255 of the red.
        assert_eq!(s.argb(), 0xBEFF_0000);
        // Flying +z at 30 a second, seen from above: across is x.
        let q = streak_quad(Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0), 0.1, &s, Vec3::Y);
        // The tail 0.1 s back (3), 0.1 wide; the head a frame ahead, 0.8 wide.
        assert!((q[0] - Vec3::new(-0.1, 0.0, -3.0)).length() < 1e-4, "{q:?}");
        assert!((q[1] - Vec3::new(-0.8, 0.0, 1.0)).length() < 1e-4);
        assert!((q[2] - Vec3::new(0.8, 0.0, 1.0)).length() < 1e-4);
        assert!((q[3] - Vec3::new(0.1, 0.0, -3.0)).length() < 1e-4);
        // Later the tail stops at half a second; flying at the camera it
        // keeps a quarter of its width.
        let q = streak_quad(Vec3::ZERO, Vec3::new(0.0, 0.0, 30.0), 2.0, &s, Vec3::Z);
        assert!((q[0].z + 15.0).abs() < 1e-3);
        assert!((q[1] - q[2]).length() - 0.4 < 1e-4);
    }

    #[test]
    fn cones_keep_the_orthogonals_unnormalised() {
        // getOrthoVecs on a slow upward direction: x across, z shrunk to y^2.
        let (a, b) = ortho_vecs(Vec3::new(0.0, 0.2, 0.0));
        assert_eq!(a, Vec3::new(0.2, 0.0, 0.0));
        assert!((b - Vec3::new(0.0, 0.0, -0.04)).length() < 1e-7);
        let (s, c) = sin_cos(0.5);
        assert!((s - 0.5f32.det_sin()).abs() < 1e-5 && (c - 0.5f32.det_cos()).abs() < 1e-5);
        assert_eq!(PbRand(0).rand(), 0x269EC3 & 0x7FFF);
    }
}
