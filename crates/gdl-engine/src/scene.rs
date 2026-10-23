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
    /// Animated and rotating world objects (`WorldAnims::dynamic`), one part each.
    dynamic: Option<(GpuMesh, Instance)>,
    models: Vec<GpuMesh>,
    players: Vec<Instance>,
    entities: HashMap<u64, Instance>,
    /// How many world objects were hidden when the world mesh was built.
    hidden_count: usize,
    /// The boss's SEETHROUGH material, made on first use.
    seethrough: Option<std::sync::Arc<wgpu::BindGroup>>,
    /// This frame's worn power-up models: (instance key, model).
    worn: Vec<(u64, usize)>,
}

/// Instance key bit for a critter's GMETER bar.
const METER_KEY: u64 = 1 << 63;
/// Instance keys for a hero's X-ray content (| the player) and the compass.
const XRAY_KEY: u64 = 1 << 62;
const COMPASS_KEY: u64 = 1 << 61;
/// Instance keys for what a hero wears (| player << 32 | model).
const WORN_KEY: u64 = 1 << 60;
/// MBTreeSetAlpha's see-through chest (0xC0) and compass (lbl_80343C20 128):
/// invAlpha = 255 - alpha.
const XRAY_CHEST_ALPHA: f32 = 63.0 / 255.0;
const COMPASS_ALPHA: f32 = 127.0 / 255.0;

/// Where a world point lands on the game's 512x384 screen (the 3D view is 4:3),
/// or None behind the camera.
pub fn project(game: &Game, pos: Vec3) -> Option<(f32, f32)> {
    let (eye, at, near) = game.camera_view();
    let m = mirror();
    let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
    let proj = crate::camera::projection(4.0 / 3.0, near, 1500.0);
    let c = proj * view * m.transform_point3(pos).extend(1.0);
    if c.w <= 0.0 {
        return None;
    }
    let (x, y) = (c.x / c.w, c.y / c.w);
    Some(((x + 1.0) * 256.0, (1.0 - y) * 192.0))
}

/// MBWindowTo3D: the world point at view depth `depth` under a point of the
/// 512x384 screen.
pub fn unproject(game: &Game, sx: f32, sy: f32, depth: f32) -> Vec3 {
    let (eye, at, near) = game.camera_view();
    let m = mirror();
    let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
    let proj = crate::camera::projection(4.0 / 3.0, near, 1500.0);
    let ndc = Vec3::new(sx / 256.0 - 1.0, 1.0 - sy / 192.0, 0.5);
    let v = proj.inverse().project_point3(ndc);
    let v = v * (depth / -v.z);
    m.transform_point3(view.inverse().transform_point3(v))
}

impl Scene {
    pub fn new(gpu: &Gpu, game: &Game) -> Result<Scene> {
        let world = gpu.upload_mesh(&game.level.world_mesh(&Self::not_static(game)))?;
        let mut world_inst = gpu.new_instance(&world);
        world_inst.transform = mirror();
        let animated: std::collections::HashSet<usize> =
            game.world_anim.dynamic.iter().copied().collect();
        let mut movers = Vec::new();
        for m in &game.movers {
            let (mesh, placed) = game
                .level
                .objects_mesh(|i| m.objects.contains(&i) && !animated.contains(&i));
            movers.push(if placed > 0 {
                let mesh = gpu.upload_mesh(&mesh)?;
                let inst = gpu.new_instance(&mesh);
                Some((mesh, inst))
            } else {
                None
            });
        }
        let dynamic = if game.world_anim.dynamic.is_empty() {
            None
        } else {
            let mesh = gpu.upload_mesh(&game.level.parts_mesh(&game.world_anim.dynamic))?;
            let inst = gpu.new_instance(&mesh);
            Some((mesh, inst))
        };
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
            dynamic,
            models,
            players,
            entities: HashMap::new(),
            hidden_count: game.hidden.len(),
            seethrough: None,
            worn: Vec::new(),
        })
    }

    pub fn sync(&mut self, gpu: &Gpu, game: &Game) {
        // World objects hidden during play (SafeRockSetup's dirt mound and
        // glow): rebuild the world mesh without them.
        if game.hidden.len() != self.hidden_count {
            if let Ok(world) = gpu.upload_mesh(&game.level.world_mesh(&Self::not_static(game))) {
                let mut inst = gpu.new_instance(&world);
                inst.transform = mirror();
                self.world = world;
                self.world_inst = inst;
            }
            self.hidden_count = game.hidden.len();
        }
        // Global texmods (DoWorldAnimation / enemy_update / ... -> DoTexMods)
        // advance once per game frame.
        let inf_frame = game.frame;
        self.world.animate_textures(inf_frame);
        for m in &mut self.models {
            m.animate_textures(inf_frame);
        }
        for (slot, m) in self.movers.iter_mut().zip(&game.movers) {
            if let Some((mesh, inst)) = slot {
                mesh.animate_textures(inf_frame);
                inst.transform = mirror() * Mat4::from_translation(Vec3::Y * m.offset);
                inst.visible = m.shown;
            }
        }
        // DoWorldAnimation / rotators: each moving object's node.
        if let Some((mesh, inst)) = &mut self.dynamic {
            mesh.animate_textures(inf_frame);
            let wa = &game.world_anim;
            inst.transform = mirror();
            inst.parts = wa.dynamic.iter().map(|&o| wa.delta(o)).collect();
            inst.hidden = wa
                .dynamic
                .iter()
                .map(|&o| wa.hidden[o] || game.hidden.contains(&o))
                .collect();
        }
        for (pi, (inst, p)) in self.players.iter_mut().zip(&game.players).enumerate() {
            let model = &game.models[p.model];
            // p->node (PlayerApplyPowerupScale's size), the skeleton lifted by
            // levitation.
            let w = &p.worn;
            inst.transform =
                mirror() * game.player_xform(pi) * Mat4::from_translation(Vec3::Y * w.lift);
            (inst.parts, inst.hidden) = model.evaluate(p.animator.seq, p.animator.frame);
            // A held power-up object hides the weapon (weaphold_node), a held
            // shield the hero's own.
            if w.hide_weapon {
                for part in model.attachment_parts() {
                    inst.hidden[part as usize] = true;
                }
            }
            if let Some(part) = w.hide_node.and_then(|n| model.node_part(n)) {
                inst.hidden[part as usize] = true;
            }
            inst.part_fx = model.texmod_fx(p.animator.seq, p.animator.frame);
            let flash = p.hurt_flash;
            // DoTransporter fades the hero out and in.
            let alpha = game.transport.alpha(pi);
            inst.tint = Vec4::new(
                1.0 + flash,
                1.0 - flash * 0.5,
                1.0 - flash * 0.5,
                alpha * w.alpha,
            );
            inst.visible = match p.state {
                PlayerState::Dead { timer } => !(timer < 2.0 && (timer * 8.0).fract() < 0.5),
                PlayerState::Exited | PlayerState::InTower => false,
                _ => true,
            };
        }

        let focus = game.cam_focus;
        let mut live = std::collections::HashSet::new();
        // What the power-ups hang on each hero (Pojo stands in for its body).
        self.worn.clear();
        for (pi, p) in game.players.iter().enumerate() {
            let shown = self.players.get(pi).is_some_and(|i| i.visible);
            if !shown {
                continue;
            }
            let base = mirror() * game.player_xform(pi);
            let fade = game.transport.alpha(pi);
            for (m, xf, seq, frame, a) in game.worn_draws(pi) {
                let key = WORN_KEY | (pi as u64) << 32 | m as u64;
                let inst = self
                    .entities
                    .entry(key)
                    .or_insert_with(|| gpu.new_instance(&self.models[m]));
                inst.transform = base * xf;
                (inst.parts, inst.hidden) = game.models[m].evaluate(seq, frame);
                inst.part_fx = game.models[m].texmod_fx(seq, frame);
                inst.tint = Vec4::new(1.0, 1.0, 1.0, fade * a);
                inst.visible = true;
                live.insert(key);
                self.worn.push((key, m));
            }
            if p.worn.hide_body {
                if let Some(i) = self.players.get_mut(pi) {
                    i.visible = false;
                }
            }
        }
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
            inst.transform = mirror()
                * Mat4::from_translation(e.pos)
                * Mat4::from_rotation_y(yaw)
                * Mat4::from_rotation_x(e.pitch)
                * Mat4::from_scale(Vec3::splat(e.scale));
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
                inst.part_fx = model.texmod_fx(r.anim.seq, r.anim.frame);
                // CritterAnimate: while paused the boss shows its boss_texture
                // (the Dragon's SEETHROUGH), blinking off in the last 180 ticks.
                let see = game.boss.as_ref().and_then(|b| b.seethrough.as_ref());
                inst.alt_tex = None;
                if let (true, Some(img)) = (r.freeze > 0.0, see) {
                    let blink = r.freeze < 180.0 && (r.freeze as i32) & 8 != 0;
                    if !blink {
                        let mat = self
                            .seethrough
                            .get_or_insert_with(|| gpu.texture_material(img))
                            .clone();
                        if let Some(inst) = self.entities.get_mut(&e.id) {
                            inst.alt_tex = Some(mat);
                        }
                    }
                }
                let inst = self.entities.get_mut(&e.id).unwrap();
                let flash = c.parts.iter().map(|p| p.flash).fold(0.0f32, f32::max);
                inst.tint = if r.freeze > 0.0 && see.is_some() {
                    Vec4::new(1.0, 1.0, 1.0, c.alpha)
                } else if r.freeze > 0.0 {
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
            inst.part_fx = model.texmod_fx(e.animator.seq, e.animator.frame);
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
            if game.snd.xray.iter().flatten().any(|x| x.chest == e.id) {
                inst.tint.w *= XRAY_CHEST_ALPHA;
            }
            inst.visible = e.fade > 0.0;
        }
        // do_see_thru: what the chest holds, inside it at 0.65.
        for (pi, x) in game.snd.xray.iter().enumerate() {
            let Some(x) = x else { continue };
            let key = XRAY_KEY | pi as u64;
            let inst = self
                .entities
                .entry(key)
                .or_insert_with(|| gpu.new_instance(&self.models[x.model]));
            inst.transform = mirror()
                * Mat4::from_translation(x.pos)
                * Mat4::from_rotation_y(x.yaw)
                * Mat4::from_scale(Vec3::splat(0.65));
            (inst.parts, inst.hidden) = game.models[x.model].evaluate(0, 0.0);
            inst.part_fx = game.models[x.model].texmod_fx(0, 0.0);
            inst.tint = Vec4::ONE;
            live.insert(key);
        }
        // fn_80052134: the COMPASS at screen (64, 128), 10 in front of the
        // camera, 1.5 times its size and half see-through; it keeps the world's
        // orientation.
        if let Some(m) = game.snd.compass_model.filter(|_| game.snd.show_compass) {
            let at = unproject(game, 64.0, 128.0, 10.0);
            let inst = self
                .entities
                .entry(COMPASS_KEY)
                .or_insert_with(|| gpu.new_instance(&self.models[m]));
            inst.transform =
                mirror() * Mat4::from_translation(at) * Mat4::from_scale(Vec3::splat(1.5));
            (inst.parts, inst.hidden) = game.models[m].evaluate(0, 0.0);
            inst.tint = Vec4::new(1.0, 1.0, 1.0, COMPASS_ALPHA);
            live.insert(COMPASS_KEY);
        }
        self.entities.retain(|id, _| live.contains(id));
    }

    /// World objects left out of the static world mesh: trigger movers,
    /// animated and rotating objects, and the hidden.
    fn not_static(game: &Game) -> std::collections::HashSet<usize> {
        game.movers
            .iter()
            .flat_map(|m| m.objects.iter().copied())
            .chain(game.world_anim.dynamic.iter().copied())
            .chain(game.hidden.iter().copied())
            .collect()
    }

    pub fn globals(&self, game: &Game, aspect: f32) -> gpu::Globals {
        let (eye, at, near) = game.camera_view();
        let m = mirror();
        let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
        let proj = crate::camera::projection(aspect, near, 1500.0);
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
        d.extend(self.dynamic.iter().map(|(m, i)| (m, i)));
        for (inst, p) in self.players.iter().zip(&game.players) {
            d.push((&self.models[p.model], inst));
        }
        for &(key, m) in &self.worn {
            if let Some(inst) = self.entities.get(&key) {
                d.push((&self.models[m], inst));
            }
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
        for (pi, x) in game.snd.xray.iter().enumerate() {
            if let (Some(x), Some(inst)) = (x, self.entities.get(&(XRAY_KEY | pi as u64))) {
                d.push((&self.models[x.model], inst));
            }
        }
        if let (Some(m), Some(inst)) = (game.snd.compass_model, self.entities.get(&COMPASS_KEY)) {
            d.push((&self.models[m], inst));
        }
        d
    }
}
