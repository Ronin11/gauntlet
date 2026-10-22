//! GPU-side mirror of a [`Game`]: uploaded meshes and per-frame instance updates.

use crate::game::{EnemyState, Game, Kind, PlayerState};
use crate::gpu::{self, Gpu, GpuMesh, Instance};
use anyhow::Result;
use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;

/// The game is left-handed; mirror X so it looks right in a right-handed renderer.
pub fn mirror() -> Mat4 {
    Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))
}

pub const CLEAR: [f64; 3] = [0.02, 0.02, 0.04];

/// How far from the camera focus entities are drawn.
const DRAW_DIST: f32 = 90.0;

pub struct Scene {
    world: GpuMesh,
    world_inst: Instance,
    /// Geometry moved by triggers, by index into `Game::movers`.
    movers: Vec<Option<(GpuMesh, Instance)>>,
    models: Vec<GpuMesh>,
    players: Vec<Instance>,
    entities: HashMap<u64, Instance>,
    /// How many world objects were hidden when the world mesh was built.
    hidden_count: usize,
}

/// Instance key bit for a critter's GMETER bar.
const METER_KEY: u64 = 1 << 63;

/// Where a world point lands on the game's 512x384 screen (the 3D view is 4:3),
/// or None behind the camera.
pub fn project(game: &Game, pos: Vec3) -> Option<(f32, f32)> {
    let (eye, at, near) = game.camera_view();
    let m = mirror();
    let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
    let proj = Mat4::perspective_rh(45f32.to_radians(), 4.0 / 3.0, near, 1500.0);
    let c = proj * view * m.transform_point3(pos).extend(1.0);
    if c.w <= 0.0 {
        return None;
    }
    let (x, y) = (c.x / c.w, c.y / c.w);
    Some(((x + 1.0) * 256.0, (1.0 - y) * 192.0))
}

impl Scene {
    pub fn new(gpu: &Gpu, game: &Game) -> Result<Scene> {
        let moving: std::collections::HashSet<usize> = game
            .movers
            .iter()
            .flat_map(|m| m.objects.iter().copied())
            .chain(game.hidden.iter().copied())
            .collect();
        let world = gpu.upload_mesh(&game.level.world_mesh(&moving))?;
        let mut world_inst = gpu.new_instance(&world);
        world_inst.transform = mirror();
        let mut movers = Vec::new();
        for m in &game.movers {
            let (mesh, placed) = game.level.objects_mesh(|i| m.objects.contains(&i));
            movers.push(if placed > 0 {
                let mesh = gpu.upload_mesh(&mesh)?;
                let inst = gpu.new_instance(&mesh);
                Some((mesh, inst))
            } else {
                None
            });
        }
        let models = game
            .models
            .iter()
            .map(|m| gpu.upload_mesh(&m.mesh))
            .collect::<Result<Vec<_>>>()?;
        let players = game
            .players
            .iter()
            .map(|p| gpu.new_instance(&models[p.model]))
            .collect();
        Ok(Scene {
            world,
            world_inst,
            movers,
            models,
            players,
            entities: HashMap::new(),
            hidden_count: game.hidden.len(),
        })
    }

    pub fn sync(&mut self, gpu: &Gpu, game: &Game) {
        // World objects hidden during play (SafeRockSetup's dirt mound and
        // glow): rebuild the world mesh without them.
        if game.hidden.len() != self.hidden_count {
            let skip: std::collections::HashSet<usize> = game
                .movers
                .iter()
                .flat_map(|m| m.objects.iter().copied())
                .chain(game.hidden.iter().copied())
                .collect();
            if let Ok(world) = gpu.upload_mesh(&game.level.world_mesh(&skip)) {
                let mut inst = gpu.new_instance(&world);
                inst.transform = mirror();
                self.world = world;
                self.world_inst = inst;
            }
            self.hidden_count = game.hidden.len();
        }
        for (slot, m) in self.movers.iter_mut().zip(&game.movers) {
            if let Some((_, inst)) = slot {
                inst.transform = mirror() * Mat4::from_translation(Vec3::Y * m.offset);
                inst.visible = m.shown;
            }
        }
        for (inst, p) in self.players.iter_mut().zip(&game.players) {
            let model = &game.models[p.model];
            inst.transform =
                mirror() * Mat4::from_translation(p.pos) * Mat4::from_rotation_y(p.yaw);
            (inst.parts, inst.hidden) = model.evaluate(p.animator.seq, p.animator.frame);
            let flash = p.hurt_flash;
            inst.tint = Vec4::new(1.0 + flash, 1.0 - flash * 0.5, 1.0 - flash * 0.5, 1.0);
            inst.visible = match p.state {
                PlayerState::Dead { timer } => !(timer < 2.0 && (timer * 8.0).fract() < 0.5),
                PlayerState::Exited | PlayerState::InTower => false,
                _ => true,
            };
        }

        let focus = game.cam_focus;
        let mut live = std::collections::HashSet::new();
        for e in &game.entities {
            let Some(m) = e.model else { continue };
            let far = if matches!(e.kind, Kind::Critter(_)) {
                DRAW_DIST * 2.0
            } else {
                DRAW_DIST
            };
            if (e.pos - focus).length() > far {
                continue;
            }
            live.insert(e.id);
            let mesh = &self.models[m];
            let inst = self
                .entities
                .entry(e.id)
                .or_insert_with(|| gpu.new_instance(mesh));
            let yaw = e.yaw + e.spin;
            inst.transform = mirror() * Mat4::from_translation(e.pos) * Mat4::from_rotation_y(yaw);
            let model = &game.models[m];
            if let Kind::Critter(c) = &e.kind {
                inst.transform = inst.transform * Mat4::from_scale(Vec3::splat(c.scale));
                // The GMETER bar: at meterdelta over it, the red fill scaled to
                // its health; gone when it dies. [CHOICE] It faces the camera.
                if let Some((gm, node)) = c.gmeter.filter(|_| c.parts[0].health > 0.0) {
                    let t = &c.wad.types[c.parts[0].ty];
                    let f = (c.parts[0].health / c.parts[0].health0.max(1.0)).clamp(0.0, 1.0);
                    let gmodel = &game.models[gm];
                    let mut world = gmodel.pose_layers(&[(None, 0, 0.0)]);
                    if let Some(w) = world.get_mut(node) {
                        *w *= Mat4::from_scale(Vec3::new(f.max(0.001), 1.0, 1.0));
                    }
                    let at = e.pos
                        + Mat4::from_rotation_y(e.yaw).transform_vector3(Vec3::from(t.meterdelta));
                    let minst = self
                        .entities
                        .entry(e.id | METER_KEY)
                        .or_insert_with(|| gpu.new_instance(&self.models[gm]));
                    minst.transform =
                        mirror() * Mat4::from_translation(at) * Mat4::from_rotation_y(game.cam_yaw);
                    (minst.parts, minst.hidden) = gmodel.evaluate_nodes(0, 0.0, &world, &[], &[]);
                    minst.tint = Vec4::ONE;
                    live.insert(e.id | METER_KEY);
                }
                let inst = self.entities.get_mut(&e.id).unwrap();
                // The root's sequence with each child's branch on top; broken
                // plates swapped, dead heads hidden.
                let world = model.pose_layers(&c.layers());
                let hidden = c.hidden_nodes(model);
                let r = &c.parts[0];
                (inst.parts, inst.hidden) =
                    model.evaluate_nodes(r.anim.seq, r.anim.frame, &world, &hidden, &c.swapped);
                let flash = c.parts.iter().map(|p| p.flash).fold(0.0f32, f32::max);
                inst.tint = if r.freeze > 0.0 {
                    // Frozen (pausecnt): blinking blue-white.
                    Vec4::new(0.7, 0.8, 1.6, c.alpha)
                } else if flash > 0.0 {
                    Vec4::new(2.0, 2.0, 2.0, c.alpha)
                } else if let Some(t) = c.tint {
                    Vec4::new(t[0], t[1], t[2], t[3] * c.alpha)
                } else {
                    Vec4::new(1.0, 1.0, 1.0, c.alpha)
                };
                continue;
            }
            (inst.parts, inst.hidden) = model.evaluate(e.animator.seq, e.animator.frame);
            if let Some(w) = game
                .boss
                .as_ref()
                .and_then(|b| b.wizard.as_ref())
                .filter(|w| w.entity == Some(e.id))
            {
                inst.tint = Vec4::new(1.0, 1.0, 1.0, 1.0 - w.alpha as f32 / 255.0);
                continue;
            }
            inst.tint = match e.kind {
                Kind::Enemy {
                    state: EnemyState::Hurt { .. },
                    ..
                } => Vec4::new(2.0, 0.6, 0.6, 1.0),
                Kind::Enemy {
                    state: EnemyState::Dying { timer },
                    ..
                } => Vec4::new(1.0, 0.3, 0.3, (timer * 2.0).min(1.0)),
                _ => Vec4::ONE,
            };
            inst.tint.w *= e.fade;
            inst.visible = e.fade > 0.0;
        }
        self.entities.retain(|id, _| live.contains(id));
    }

    pub fn globals(&self, game: &Game, aspect: f32) -> gpu::Globals {
        let (eye, at, near) = game.camera_view();
        let m = mirror();
        let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
        let proj = Mat4::perspective_rh(45f32.to_radians(), aspect, near, 1500.0);
        gpu::globals(
            proj * view,
            m.transform_point3(eye),
            [CLEAR[0] as f32, CLEAR[1] as f32, CLEAR[2] as f32],
            0.002,
        )
    }

    pub fn draws<'a>(&'a self, game: &Game) -> Vec<(&'a GpuMesh, &'a Instance)> {
        let mut d = vec![(&self.world, &self.world_inst)];
        d.extend(self.movers.iter().flatten().map(|(m, i)| (m, i)));
        for (inst, p) in self.players.iter().zip(&game.players) {
            d.push((&self.models[p.model], inst));
        }
        for e in &game.entities {
            if let (Some(m), Some(inst)) = (e.model, self.entities.get(&e.id)) {
                d.push((&self.models[m], inst));
            }
            if let (Kind::Critter(c), Some(inst)) =
                (&e.kind, self.entities.get(&(e.id | METER_KEY)))
            {
                if let Some((gm, _)) = c.gmeter {
                    d.push((&self.models[gm], inst));
                }
            }
        }
        d
    }
}
