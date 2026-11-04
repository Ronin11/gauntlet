//! The screen saver (ui/screensaver.c: ScreenSaver, ScreenSaverStart,
//! ScreenSaverUpdateWeap). Outside the attract loop, movies and the ending,
//! ten minutes (36000 ticks) with no change in any controller's held buttons
//! blank the screen: four weapons from the POWERUPS bank (SAVER_AXE, _HAM,
//! _SWD, _WND) fly and tumble about in front of the camera, bouncing off the
//! edges of its view, until a held button changes. The game is frozen
//! meanwhile and the music stopped.

use crate::character::Model;
use crate::fs::GameFs;
use crate::gpu::{Gpu, GpuMesh, Instance};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use std::f32::consts::{PI, TAU};

/// lbl_80344A48 trips at 36000 (60 Hz) ticks.
pub const IDLE_TICKS: u32 = 36000;
/// lbl_8011D568 + 0x744 (weaponInit): the weapons' atrees, start points and
/// headings, in camera space (x right, y up, z ahead).
const NAMES: [&str; 4] = ["SAVER_AXE", "SAVER_HAM", "SAVER_SWD", "SAVER_WND"];
const START: [[f32; 3]; 4] = [
    [20.0, 0.0, 1.0],
    [-20.0, 0.0, 1.0],
    [0.0, 20.0, 1.0],
    [0.0, -20.0, 1.0],
];
const HEADING: [[f32; 3]; 4] = [
    [-1.0, 0.0, 1.0],
    [1.0, 0.0, 1.0],
    [0.0, -1.0, 1.0],
    [0.0, 1.0, 1.0],
];
/// lbl_80343CB8 (speed per 1/30 s), lbl_80343CB4 (spin per 1/30 s).
const SPEED: f32 = 10.0;
const SPIN: f32 = TAU;
/// lbl_80343CAC / lbl_80343CB0: the far and near bounce depths.
const FAR: f32 = 40.0;
const NEAR: f32 = 5.0;
/// lbl_80343CBC: the sideways nudge a bounce gives (x Random(2) - 1).
const SPREAD: f32 = 0.1;
/// lbl_80343CC0 / lbl_80343CC4: a weapon gone off the far side comes back
/// after DELAY x (0.5 + Random(1)) s; one may go off that way after RESET x
/// (0.5 + Random(1)) s.
const DELAY: f32 = 3.0;
const RESET: f32 = 10.0;

/// One weapon (ScreenSaverWeapon).
#[derive(Clone, Debug, Default)]
pub struct Weapon {
    pub pos: Vec3,
    pub vel: Vec3,
    pub angle: f32,
    /// collisionState: bounces count once it has been in view (> 0); a
    /// bounce sets it to -10, and it is armed again the next tick.
    coll: i32,
    pub elapsed: i32,
    /// Ticks until it (re)appears; 0 once it is flying.
    pub duration: i32,
    reset_at: i32,
    /// Its node exists (it is drawn).
    pub shown: bool,
}

/// The screen saver's weapons and their random numbers.
#[derive(Clone, Debug)]
pub struct Saver {
    pub weapons: [Weapon; 4],
    seed: u32,
    /// Seconds of the weapons' own animation (AnimateATree).
    pub frame: f32,
}

impl Saver {
    /// ScreenSaverStart: each weapon waits i x 30 + RandInt(15) + 1 ticks.
    pub fn new(seed: u32) -> Saver {
        let mut s = Saver {
            weapons: Default::default(),
            seed: seed | 1,
            frame: 0.0,
        };
        for i in 0..4 {
            let reset_at = s.delay(RESET);
            let wait = i as i32 * 30 + (s.random(15.0) as i32) + 1;
            let w = &mut s.weapons[i];
            w.pos = Vec3::from(START[i]);
            w.reset_at = reset_at;
            w.duration = wait;
        }
        s
    }

    /// Random(range): [0, range).
    fn random(&mut self, range: f32) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed >> 8) as f32 / (1u32 << 24) as f32 * range
    }

    /// 60 x k x (0.5 + Random(1)) ticks.
    fn delay(&mut self, k: f32) -> i32 {
        (60.0 * (k * (0.5 + self.random(1.0)))) as i32
    }

    /// One 60 Hz tick: ScreenSaverUpdateWeap for each weapon.
    pub fn tick(&mut self, half_tans: (f32, f32)) {
        self.frame += 1.0 / 60.0;
        for i in 0..4 {
            self.update(i, half_tans);
        }
    }

    fn update(&mut self, i: usize, (th, tv): (f32, f32)) {
        let step = 1.0 / 30.0;
        self.weapons[i].elapsed += 1;
        if self.weapons[i].elapsed < self.weapons[i].duration {
            return;
        }
        if self.weapons[i].duration > 0 {
            let w = &mut self.weapons[i];
            w.shown = true;
            w.pos = Vec3::from(START[i]);
            w.vel = Vec3::from(HEADING[i]).normalize();
            w.angle = 0.0;
            w.coll = 0;
            w.duration = 0;
        }
        let w = &mut self.weapons[i];
        w.pos += w.vel * (step * SPEED);
        w.angle -= SPIN * step;
        if w.angle > PI {
            w.angle -= TAU;
        } else if w.angle <= -PI {
            w.angle += TAU;
        }
        // MBWorldSphereClip of the point against the view: 1 left, 2 right,
        // 3 top, 4 bottom; 5 past the far depth, 6 short of the near one.
        let p = w.pos;
        let hit = if p.z > FAR {
            5
        } else if p.z < NEAR {
            6
        } else if p.x < -th * p.z {
            1
        } else if p.x > th * p.z {
            2
        } else if p.y > tv * p.z {
            3
        } else if p.y < -tv * p.z {
            4
        } else {
            0
        };
        if self.weapons[i].coll > 0 {
            let back = 0.5 + self.random(0.5);
            let nudge = -SPREAD + SPREAD * self.random(2.0);
            let w = &mut self.weapons[i];
            match hit {
                1 => (w.vel.x, w.vel.y) = (back, w.vel.y + nudge),
                2 => (w.vel.x, w.vel.y) = (-back, w.vel.y + nudge),
                3 => (w.vel.y, w.vel.x) = (-back, w.vel.x + nudge),
                4 => (w.vel.y, w.vel.x) = (back, w.vel.x + nudge),
                5 => {
                    w.vel.z = -back;
                    if w.elapsed > w.reset_at {
                        // Off it goes; back after a while.
                        w.pos = Vec3::from(START[i]);
                        w.elapsed = 0;
                        w.shown = false;
                        let d = self.delay(DELAY);
                        let r = self.delay(RESET);
                        let w = &mut self.weapons[i];
                        w.duration = d + 1;
                        w.reset_at = r;
                        return;
                    }
                }
                6 => w.vel.z = back,
                _ => {}
            }
            let w = &mut self.weapons[i];
            if hit != 0 {
                w.vel = w.vel.normalize_or_zero();
                w.coll = -10;
            }
        } else if self.weapons[i].coll < 0 {
            self.weapons[i].coll = 1;
        } else if hit == 0 {
            self.weapons[i].coll = 1;
        }
    }
}

/// CreateDirMatrix(dir, NULL): z along `dir`, x level.
fn dir_matrix(dir: Vec3) -> Mat4 {
    let z = dir.normalize_or_zero();
    let x = Vec3::Y.cross(z).try_normalize().unwrap_or(Vec3::X);
    let y = z.cross(x);
    Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), glam::Vec4::W)
}

/// The weapons' models, loaded on first use.
pub struct SaverModels {
    models: Vec<(Model, GpuMesh, Instance)>,
}

impl SaverModels {
    pub fn load(fs: &GameFs, gpu: &Gpu) -> Result<SaverModels> {
        let objects = gdl_formats::Objects::parse(&fs.read("POWERUPS/objects.ngc")?)?;
        let textures = fs.read("POWERUPS/textures.ngc")?;
        let anim = gdl_formats::anim::Anim::parse(&fs.read("POWERUPS/ANIM.PS2")?)?;
        let mut models = Vec::new();
        for name in NAMES {
            let model = Model::from_anim(&objects, &textures, &anim, Some(name)).context(name)?;
            let mesh = gpu.upload_mesh(&model.mesh)?;
            let inst = gpu.new_instance(&mesh);
            models.push((model, mesh, inst));
        }
        Ok(SaverModels { models })
    }

    /// Draw the shown weapons over `target` (cleared to black first) with
    /// the game camera's projection, in the 4:3 `viewport`.
    pub fn draw(
        &mut self,
        saver: &Saver,
        gpu: &Gpu,
        target: &wgpu::TextureView,
        depth: &crate::gpu::Depth,
        viewport: [f32; 4],
    ) {
        let blank = crate::gpu::globals(Mat4::IDENTITY, Vec3::ZERO, [0.0; 3], 0.0);
        gpu.render(target, depth, &blank, [0.0; 3], &[]);
        let proj = crate::camera::projection(4.0 / 3.0, 1.0, 100.0);
        for (w, (model, _, inst)) in saver.weapons.iter().zip(&mut self.models) {
            if !w.shown {
                continue;
            }
            (inst.parts, inst.hidden) = model.evaluate(0, saver.frame);
            // Camera space (z ahead) to the view's -z.
            let at = Vec3::new(w.pos.x, w.pos.y, -w.pos.z);
            inst.transform = Mat4::from_translation(at)
                * Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0))
                * dir_matrix(w.vel)
                * Mat4::from_rotation_x(w.angle)
                * crate::scene::mirror();
        }
        let draws: Vec<(&GpuMesh, &Instance)> = saver
            .weapons
            .iter()
            .zip(&self.models)
            .filter(|(w, _)| w.shown)
            .map(|(_, (_, m, i))| (m, i))
            .collect();
        let g = crate::gpu::globals(proj, Vec3::ZERO, [0.0; 3], 0.0);
        gpu.render_over(target, depth, &g, &draws, viewport);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The weapons come in one after another, stay within the view's depth
    /// band once in it, and tumble.
    #[test]
    fn weapons_fly_in_and_bounce_inside_the_view() {
        let mut s = Saver::new(7);
        let tans = crate::camera::half_tans();
        assert!(s.weapons.iter().all(|w| !w.shown));
        for _ in 0..15 {
            s.tick(tans);
        }
        assert!(s.weapons[0].shown && !s.weapons[3].shown);
        let mut seen = [false; 4];
        for _ in 0..60 * 120 {
            s.tick(tans);
            for (k, w) in s.weapons.iter().enumerate() {
                if w.shown && w.coll > 0 {
                    seen[k] = true;
                    // Never far past the band it bounces in.
                    assert!(w.pos.z < FAR + 1.0 && w.pos.z > NEAR - 1.0, "{w:?}");
                }
            }
        }
        assert_eq!(seen, [true; 4]);
    }

    #[test]
    fn saver_weapons_exist() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        let Ok(fs) = GameFs::open(&root) else { return };
        let objects =
            gdl_formats::Objects::parse(&fs.read("POWERUPS/objects.ngc").unwrap()).unwrap();
        let textures = fs.read("POWERUPS/textures.ngc").unwrap();
        let anim = gdl_formats::anim::Anim::parse(&fs.read("POWERUPS/ANIM.PS2").unwrap()).unwrap();
        for n in NAMES {
            let m = Model::from_anim(&objects, &textures, &anim, Some(n)).unwrap();
            assert!(!m.mesh.indices.is_empty(), "{n}");
        }
    }
}
