//! GPU-side mirror of a [`Game`]: uploaded meshes and per-frame instance updates.

use crate::game::{EnemyState, Game, Kind, PlayerState};
use crate::gpu::{self, Gpu, GpuMesh, Instance, SpriteDraw, SpriteLayer, SpriteVertex};
use crate::interp::{Cam, Interp, Pose};
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
    /// The model of each shadow instance in `entities`, by key.
    shadow_models: HashMap<u64, usize>,
    /// This frame's particles and missile streaks.
    sprites: SpriteLayer,
    /// The last two ticks' poses (interp.rs), and this frame's blended
    /// camera (None: the game's own).
    interp: Interp,
    cam: Option<Cam>,
}

/// DrawPsysSub's screen limits on the GameCube's 640x480: both corners
/// within 16 pixels of the screen, at least a pixel across.
const SPRITE_MARGIN: [f32; 2] = [32.0 / 640.0, 32.0 / 480.0];
const SPRITE_MIN: [f32; 2] = [2.0 / 640.0, 2.0 / 480.0];
/// pbWindow's far plane.
const SPRITE_FAR: f32 = 1500.0;

/// 0xAARRGGBB as a vertex colour.
fn argb(c: u32) -> [f32; 4] {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
        (c >> 24) as f32 / 255.0,
    ]
}

/// Instance key bit for a critter's GMETER bar.
const METER_KEY: u64 = 1 << 63;
/// Instance keys for a hero's X-ray content (| the player) and the compass.
const XRAY_KEY: u64 = 1 << 62;
const COMPASS_KEY: u64 = 1 << 61;
/// Instance keys for what a hero wears (| player << 32 | model).
const WORN_KEY: u64 = 1 << 58;
/// Instance keys for a shadow: | the entity id, or | SHADOW_PLAYER | the hero.
const SHADOW_KEY: u64 = 1 << 60;
const SHADOW_PLAYER: u64 = 1 << 59;
/// Interpolation keys for the heroes, trigger movers and world animations
/// (which aren't in `entities`).
const PLAYER_KEY: u64 = 1 << 57;
const MOVER_KEY: u64 = 1 << 56;
const DYNAMIC_KEY: u64 = 1 << 55;
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
            shadow_models: HashMap::new(),
            sprites: gpu.new_sprite_layer(),
            interp: Interp::default(),
            cam: None,
        })
    }

    /// The particle and streak quads [`Scene::sync`] built.
    pub fn sprites(&self) -> &SpriteLayer {
        &self.sprites
    }

    /// MBDrawPsysObjects: each psys's sprites as camera-facing quads (its
    /// texture, blend and z), culled and capped as DrawPsysSub does, then
    /// the missile streaks.
    fn sync_sprites(&mut self, gpu: &Gpu, game: &Game) {
        let parts = &game.particles;
        while self.sprites.texture_count() < parts.textures.len() {
            let i = self.sprites.texture_count();
            gpu.add_sprite_texture(&mut self.sprites, &parts.textures[i]);
        }
        let (eye, at, near) = game.camera_view();
        let m = mirror();
        let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
        let vp = crate::camera::projection(4.0 / 3.0, near, SPRITE_FAR) * view;
        // The view's right and up rows (lbl_80344EE8's basis).
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        let on_screen = |c: Vec4| {
            let (x, y) = (c.x / c.w, c.y / c.w);
            (-1.0 - SPRITE_MARGIN[0]..1.0 + SPRITE_MARGIN[0]).contains(&x)
                && (-1.0 - SPRITE_MARGIN[1]..1.0 + SPRITE_MARGIN[1]).contains(&y)
        };
        let mut quads: Vec<[SpriteVertex; 4]> = Vec::new();
        let mut draws = Vec::new();
        for b in parts.batches() {
            let first = quads.len() as u32;
            // Half a texel in from each edge.
            let (tw, th) = b
                .tex
                .and_then(|t| parts.textures.get(t))
                .map_or((16.0, 16.0), |t| (t.width as f32, t.height as f32));
            let (u0, u1) = (0.5 / tw, (tw - 0.5) / tw);
            let (v0, v1) = (0.5 / th, (th - 0.5) / th);
            for &(pos, w, c) in &b.sprites {
                if quads.len() >= crate::psys::MAX_QUADS {
                    break;
                }
                let p = m.transform_point3(pos);
                let h = w * 0.5;
                let k1 = vp * (p - (right + up) * h).extend(1.0);
                if k1.w <= near || k1.w >= SPRITE_FAR || !on_screen(k1) {
                    continue;
                }
                let k2 = vp * (p + (right + up) * h).extend(1.0);
                if k2.w <= 0.0 || !on_screen(k2) {
                    continue;
                }
                if (k2.x / k2.w - k1.x / k1.w).abs() < SPRITE_MIN[0]
                    || (k2.y / k2.w - k1.y / k1.w).abs() < SPRITE_MIN[1]
                {
                    continue;
                }
                let color = argb(c);
                let v = |d: Vec3, uv: [f32; 2]| SpriteVertex {
                    pos: (p + d * h).to_array(),
                    uv,
                    color,
                };
                quads.push([
                    v(up - right, [u0, v0]),
                    v(up + right, [u1, v0]),
                    v(right - up, [u1, v1]),
                    v(-right - up, [u0, v1]),
                ]);
            }
            let count = quads.len() as u32 - first;
            if count > 0 {
                draws.push(SpriteDraw {
                    texture: b.tex,
                    blend: b.blend,
                    ztest: b.ztest,
                    zwrite: b.zwrite,
                    first,
                    count,
                });
            }
        }
        // UpdateFXStreak's quads (MBPolyInst: alpha blended, z written).
        for s in &parts.streaks {
            let color = argb(s.color);
            let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
            let q = std::array::from_fn(|i| SpriteVertex {
                pos: m.transform_point3(s.corners[i]).to_array(),
                uv: uv[i],
                color,
            });
            draws.push(SpriteDraw {
                texture: s.tex,
                blend: crate::psys::Blend::Alpha,
                ztest: true,
                zwrite: true,
                first: quads.len() as u32,
                count: 1,
            });
            quads.push(q);
        }
        gpu.set_sprites(&mut self.sprites, &quads, draws);
    }

    pub fn sync(&mut self, gpu: &Gpu, game: &Game) {
        self.cam = None;
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
        // What the power-ups hang on each hero.
        self.worn.clear();
        for pi in 0..game.players.len() {
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
            // An effect's ScaleFX may be uneven, and MBTreeSetColor tints it.
            let look = matches!(e.kind, Kind::Effect { .. })
                .then(|| game.fx_look(e.id))
                .flatten();
            let scale = look.and_then(|l| l.scale).unwrap_or(Vec3::splat(e.scale));
            inst.transform = mirror()
                * Mat4::from_translation(e.pos)
                * Mat4::from_rotation_y(yaw)
                * Mat4::from_rotation_x(e.pitch)
                * Mat4::from_scale(scale);
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
                let world = c.pose(model);
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
                // A breakable wall's flash (MBTreeSetAltTex whitetex).
                Kind::Wall(ref w) if w.white => Vec4::new(8.0, 8.0, 8.0, 1.0),
                _ => match look.and_then(|l| l.tint) {
                    Some([r, g, b]) => Vec4::new(r, g, b, 1.0),
                    None => Vec4::ONE,
                },
            };
            inst.tint.w *= e.fade;
            if game.snd.xray.iter().flatten().any(|x| x.chest == e.id) {
                inst.tint.w *= XRAY_CHEST_ALPHA;
            }
            inst.visible = e.fade > 0.0;
        }
        // Floor shadows (shadow.rs): each node fades with its body
        // (MBTreeSetAlpha on both) and is hidden with it.
        for (pi, p) in game.players.iter().enumerate() {
            let key = SHADOW_KEY | SHADOW_PLAYER | pi as u64;
            let body = self.players.get(pi).map(|i| (i.visible, i.tint.w));
            if let (Some(s), Some((true, alpha))) = (game.player_shadow(p), body) {
                Self::place_shadow(&mut self.entities, gpu, &self.models, key, s, alpha);
                self.shadow_models.insert(key, s.model);
                live.insert(key);
            }
        }
        for e in &game.entities {
            let body = self.entities.get(&e.id).filter(|_| live.contains(&e.id));
            let Some(body) = body.map(|i| (i.visible, i.tint.w)) else {
                continue;
            };
            if let (Some(s), (true, alpha)) = (game.entity_shadow(e), body) {
                let key = SHADOW_KEY | e.id;
                Self::place_shadow(&mut self.entities, gpu, &self.models, key, s, alpha);
                self.shadow_models.insert(key, s.model);
                live.insert(key);
            }
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
        self.shadow_models.retain(|id, _| live.contains(id));
        self.sync_sprites(gpu, game);
        if self.interp.needs(game.frame) {
            self.capture(game);
        }
    }

    /// Before the last game tick of a display frame: keep the tick about to
    /// be left behind, if the display hasn't drawn it (more than one tick
    /// this frame), so the next frame blends from it.
    pub fn pre_tick(&mut self, gpu: &Gpu, game: &Game) {
        if self.interp.needs(game.frame) {
            self.sync(gpu, game);
        }
    }

    /// Keep this tick's poses: every instance's transform and parts, tagged
    /// with the model and sequence its parts sample.
    fn capture(&mut self, game: &Game) {
        let ip = &mut self.interp;
        ip.begin(game.frame, game.camera_view());
        let pose = |i: &Instance, tag, frame| Pose {
            transform: i.transform,
            parts: i.parts.clone(),
            tag,
            frame,
        };
        for (pi, (inst, p)) in self.players.iter().zip(&game.players).enumerate() {
            let tag = crate::interp::tag(p.model, p.animator.seq);
            ip.put(PLAYER_KEY | pi as u64, pose(inst, tag, p.animator.frame));
        }
        for (i, slot) in self.movers.iter().enumerate() {
            if let Some((_, inst)) = slot {
                ip.put(MOVER_KEY | i as u64, pose(inst, 0, 0.0));
            }
        }
        if let Some((_, inst)) = &self.dynamic {
            ip.put(DYNAMIC_KEY, pose(inst, 0, 0.0));
        }
        let mut anims = HashMap::new();
        for e in &game.entities {
            let (Some(m), a) = (e.model, &e.animator) else {
                continue;
            };
            let a = match &e.kind {
                Kind::Critter(c) => &c.parts[0].anim,
                _ => a,
            };
            anims.insert(e.id, (crate::interp::tag(m, a.seq), a.frame));
        }
        for (&key, inst) in &self.entities {
            let (tag, frame) = anims.get(&key).copied().unwrap_or((0, 0.0));
            ip.put(key, pose(inst, tag, frame));
        }
    }

    /// Draw this frame `alpha` (the accumulator over a tick) of the way from
    /// the previous tick to the current one (interp.rs). Not calling this
    /// draws the current tick exactly.
    pub fn blend(&mut self, game: &Game, alpha: f32) {
        let t = self.interp.alpha(game.frame, alpha);
        if t >= 1.0 {
            return;
        }
        let ip = &self.interp;
        let one = |key: u64, inst: &mut Instance| {
            if let (Some(a), Some(b)) = (ip.prev.get(&key), ip.cur.get(&key)) {
                crate::interp::blend_pose(a, b, t, &mut inst.transform, &mut inst.parts);
            }
        };
        for (pi, inst) in self.players.iter_mut().enumerate() {
            one(PLAYER_KEY | pi as u64, inst);
        }
        for (i, slot) in self.movers.iter_mut().enumerate() {
            if let Some((_, inst)) = slot {
                one(MOVER_KEY | i as u64, inst);
            }
        }
        if let Some((_, inst)) = &mut self.dynamic {
            one(DYNAMIC_KEY, inst);
        }
        for (&key, inst) in self.entities.iter_mut() {
            one(key, inst);
        }
        self.cam = ip.cam(t);
    }

    /// Put shadow `key` where `s` says, at its body's opacity.
    fn place_shadow(
        entities: &mut HashMap<u64, Instance>,
        gpu: &Gpu,
        models: &[GpuMesh],
        key: u64,
        s: crate::shadow::Placed,
        alpha: f32,
    ) {
        let inst = entities
            .entry(key)
            .or_insert_with(|| gpu.new_instance(&models[s.model]));
        inst.transform = mirror() * s.transform;
        inst.tint = Vec4::new(1.0, 1.0, 1.0, alpha);
        inst.visible = alpha > 0.0;
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
        let (eye, at, near) = self.cam.unwrap_or_else(|| game.camera_view());
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
        // Shadows before the actors: their zsort_add (3) sorts them behind
        // whatever stands on them among the blended draws.
        let shadow_keys = (0..game.players.len())
            .map(|pi| SHADOW_KEY | SHADOW_PLAYER | pi as u64)
            .chain(game.entities.iter().map(|e| SHADOW_KEY | e.id));
        for key in shadow_keys {
            if let (Some(&m), Some(inst)) = (self.shadow_models.get(&key), self.entities.get(&key))
            {
                d.push((&self.models[m], inst));
            }
        }
        // Pojo stands in for the hero's body (its shadow stays).
        for (inst, p) in self.players.iter().zip(&game.players) {
            if !p.worn.hide_body {
                d.push((&self.models[p.model], inst));
            }
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
