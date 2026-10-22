//! GPU-side mirror of a [`Game`]: uploaded meshes and per-frame instance updates.

use crate::game::Game;
use crate::gpu::{self, Gpu, GpuMesh, Instance};
use anyhow::Result;
use glam::{Mat4, Vec3};

/// The game is left-handed; mirror X so it looks right in a right-handed renderer.
pub fn mirror() -> Mat4 {
    Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))
}

pub const CLEAR: [f64; 3] = [0.02, 0.02, 0.04];

pub struct Scene {
    world: GpuMesh,
    world_inst: Instance,
    models: Vec<GpuMesh>,
    players: Vec<Instance>,
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
        })
    }

    pub fn sync(&mut self, game: &Game) {
        for (inst, p) in self.players.iter_mut().zip(&game.players) {
            let model = &game.models[p.model];
            inst.transform =
                mirror() * Mat4::from_translation(p.pos) * Mat4::from_rotation_y(p.yaw);
            (inst.parts, inst.hidden) = model.evaluate(p.animator.seq, p.animator.frame);
        }
    }

    pub fn globals(&self, game: &Game, aspect: f32) -> gpu::Globals {
        let (eye, at) = game.camera();
        let m = mirror();
        let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
        let proj = Mat4::perspective_rh(45f32.to_radians(), aspect, 1.0, 1500.0);
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
        d
    }
}
