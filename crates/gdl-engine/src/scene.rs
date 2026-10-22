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
}

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
        })
    }

    pub fn sync(&mut self, gpu: &Gpu, game: &Game) {
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
            if (e.pos - focus).length() > DRAW_DIST {
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
            (inst.parts, inst.hidden) = game.models[m].evaluate(e.animator.seq, e.animator.frame);
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
        }
        d
    }
}
