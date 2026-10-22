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
    models: Vec<GpuMesh>,
    players: Vec<Instance>,
    entities: HashMap<u64, Instance>,
}

impl Scene {
    pub fn new(gpu: &Gpu, game: &Game) -> Result<Scene> {
        let world = gpu.upload_mesh(&game.level.world_mesh())?;
        let mut world_inst = gpu.new_instance(&world);
        world_inst.transform = mirror();
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
            models,
            players,
            entities: HashMap::new(),
        })
    }

    pub fn sync(&mut self, gpu: &Gpu, game: &Game) {
        for (inst, p) in self.players.iter_mut().zip(&game.players) {
            let model = &game.models[p.model];
            inst.transform =
                mirror() * Mat4::from_translation(p.pos) * Mat4::from_rotation_y(p.yaw);
            (inst.parts, inst.hidden) = model.evaluate(p.animator.seq, p.animator.frame);
            let flash = p.hurt_flash;
            inst.tint = Vec4::new(1.0 + flash, 1.0 - flash * 0.5, 1.0 - flash * 0.5, 1.0);
            inst.visible = !matches!(p.state, PlayerState::Dead { timer } if timer < 2.0 && (timer * 8.0).fract() < 0.5);
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
