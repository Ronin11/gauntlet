//! The menus' 3D selection arrow (ICON_ARROW from the POWERUPS bank), drawn over
//! the 2D overlay as show_optmenu places it: at a screen point, 1.1 units in front
//! of the camera, scaled 0.05, facing the camera and pitched by an angle.

use crate::character::Model;
use crate::fs::GameFs;
use crate::gpu::{Gpu, GpuMesh, Instance};
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};

/// optmenu_icon_z and the menus' icon scale.
const ICON_Z: f32 = 1.1;
const ICON_SCALE: f32 = 0.05;
/// The front end's camera zoom (titleWindowZoom, 60 degrees).
const FOV: f32 = 1.047_197_6;

pub struct Cursor {
    model: Model,
    mesh: GpuMesh,
    inst: Instance,
    frame: f32,
}

impl Cursor {
    pub fn load(fs: &GameFs, gpu: &Gpu) -> Result<Cursor> {
        let objects = gdl_formats::Objects::parse(&fs.read("POWERUPS/objects.ngc")?)?;
        let textures = fs.read("POWERUPS/textures.ngc")?;
        let anim = gdl_formats::anim::Anim::parse(&fs.read("POWERUPS/ANIM.PS2")?)?;
        let model = Model::from_anim(&objects, &textures, &anim, Some("ICON_ARROW"))
            .context("ICON_ARROW")?;
        let mesh = gpu.upload_mesh(&model.mesh)?;
        let inst = gpu.new_instance(&mesh);
        Ok(Cursor {
            model,
            mesh,
            inst,
            frame: 0.0,
        })
    }

    /// Advance the arrow's own animation (AnimateATree) by one 60 Hz tick.
    pub fn tick(&mut self) {
        self.frame += 1.0 / 60.0;
    }

    /// Draw the arrow at virtual screen point (x, y), pitched by `angle`, into the
    /// 4:3 `viewport` of the target.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        viewport: [f32; 4],
        (x, y, angle): (f32, f32, f32),
    ) {
        let aspect = 4.0 / 3.0;
        let t = (FOV * 0.5).tan();
        let ndc = (x / 512.0 * 2.0 - 1.0, 1.0 - y / 384.0 * 2.0);
        let pos = Vec3::new(ndc.0 * ICON_Z * t * aspect, ndc.1 * ICON_Z * t, -ICON_Z);
        let proj = Mat4::perspective_rh(FOV, aspect, 0.05, 10.0);
        let seq = self.model.sequence("").unwrap_or(0);
        (self.inst.parts, self.inst.hidden) = self.model.evaluate(seq, self.frame);
        self.inst.transform = Mat4::from_translation(pos)
            * Mat4::from_rotation_x(angle)
            * Mat4::from_scale(Vec3::splat(ICON_SCALE))
            * crate::scene::mirror();
        let g = crate::gpu::globals(proj, Vec3::ZERO, [0.0; 3], 0.0);
        gpu.render_over(target, depth, &g, &[(&self.mesh, &self.inst)], viewport);
    }
}
