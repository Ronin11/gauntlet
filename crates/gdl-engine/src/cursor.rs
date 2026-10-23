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
        // The front end's window (titleWindowZoom: 60 degrees across, as in
        // the game), through which MBWorldToScreen3D takes the screen point
        // back to view space at depth optmenu_icon_z.
        let proj = crate::camera::projection(4.0 / 3.0, 0.05, 10.0);
        let pos = unproject(proj, x, y, ICON_Z);
        let seq = self.model.sequence("").unwrap_or(0);
        (self.inst.parts, self.inst.hidden) = self.model.evaluate(seq, self.frame);
        // The node copies the camera's matrix, so the model sits in camera
        // space (x right, y up, z ahead) and only z turns round to the view's
        // -z; there is no world mirror here. The arrow lies along -x with its
        // tip at its origin, so it ends at the point, left of the menu text.
        // PitchMat3(angle) turns by -angle about camera x: +angle once z flips.
        self.inst.transform = Mat4::from_translation(pos)
            * Mat4::from_rotation_x(angle)
            * Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0))
            * Mat4::from_scale(Vec3::splat(ICON_SCALE));
        let g = crate::gpu::globals(proj, Vec3::ZERO, [0.0; 3], 0.0);
        gpu.render_over(target, depth, &g, &[(&self.mesh, &self.inst)], viewport);
    }
}

/// The view-space point `depth` in front of the camera that `proj` puts on
/// virtual screen point (x, y) of the 512x384 screen.
fn unproject(proj: Mat4, x: f32, y: f32, depth: f32) -> Vec3 {
    let (nx, ny) = (x / 256.0 - 1.0, 1.0 - y / 192.0);
    let inv = proj.inverse();
    let a = inv.project_point3(Vec3::new(nx, ny, 0.0));
    let b = inv.project_point3(Vec3::new(nx, ny, 1.0));
    a + (b - a) * ((-depth - a.z) / (b.z - a.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unproject_lands_on_the_point() {
        let proj = crate::camera::projection(4.0 / 3.0, 0.05, 10.0);
        for (x, y) in [(0.0, 0.0), (176.0, 336.0), (512.0, 384.0), (256.0, 192.0)] {
            let p = unproject(proj, x, y, ICON_Z);
            assert!((p.z + ICON_Z).abs() < 1e-5);
            let n = proj.project_point3(p);
            assert!((n.x - (x / 256.0 - 1.0)).abs() < 1e-4, "{x} {n}");
            assert!((n.y - (1.0 - y / 192.0)).abs() < 1e-4, "{y} {n}");
        }
    }
}
