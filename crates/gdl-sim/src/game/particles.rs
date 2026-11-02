//! The game's side of the particle systems (mb_particle.c, in psys.rs):
//! CreateWorldNode's `PSYS<id>` world objects (WorldPsysActivate),
//! AtreeNodeInit's particle nodes on whatever the game shows, PsfxDoParticle
//! on the heroes, and the thrown weapons' streaks (SfxSetStreak,
//! UpdateFXStreak).
//!
//! A child of `game`, so it reaches the game's private state.

use crate::math::Det as _;
use super::*;
use crate::psys::{streak_quad, Owner, Psys, Streak, StreakQuad};
use gdl_formats::psys::enables;
use gdl_formats::Image;
use glam::Mat4;

/// WorldObj flags: PARTICLE_SYSTEM, WorldObj dynamic (MBNewWorldPsys's
/// `d`), and "starts off" (CreateWorldNode skips WorldPsysActivate).
const WO_PSYS: u32 = 0x800;
const WO_DYNAMIC: u32 = 0x1000;
const WO_PSYS_OFF: u32 = 0x1000_0000;

/// lbl_8011A188: a streak's intensity by char_type.
const STREAK_INTENSITY: [i32; 8] = [0x40, 0x40, 0xC0, 0x40, 0x40, 0x40, 0xC0, 0x40];

/// MBWorldSphereVisible3: a sphere inside the view-projection's frustum.
pub(crate) fn sphere_visible(vp: &Mat4, p: Vec3, r: f32) -> bool {
    let rows = [vp.row(0), vp.row(1), vp.row(2), vp.row(3)];
    let planes = [
        rows[3] + rows[0],
        rows[3] - rows[0],
        rows[3] + rows[1],
        rows[3] - rows[1],
        rows[3] + rows[2],
        rows[3] - rows[2],
    ];
    planes.iter().all(|pl| {
        let n = pl.truncate();
        let len = n.length();
        len == 0.0 || (n.dot(p) + pl.w) / len >= -r
    })
}

impl Game {
    /// MBInitPsys / allocPsys's default textures (STATIC's particle2_a and
    /// particle2_xp), WeaponStreakTex, the banks MBOX_FindTexture searches,
    /// and CreateWorldNode's world emitters.
    pub(crate) fn init_particles(&mut self, fs: &GameFs) {
        for dir in ["WEAPONS", "POWERUPS", "STATIC"] {
            let objects = fs
                .read(&format!("{dir}/objects.ngc"))
                .ok()
                .and_then(|d| Objects::parse(&d).ok());
            if let (Some(o), Ok(t)) = (objects, fs.read(&format!("{dir}/textures.ngc"))) {
                self.psys_banks.push(std::sync::Arc::new((o, t)));
            }
        }
        self.particles.default_tex = [
            self.psys_texture("particle2_a", None),
            self.psys_texture("particle2_xp", None),
        ];
        self.particles.streak_tex = self.psys_texture("WEP_STREAK", None);
        let starts: Vec<usize> = (self.level.worlds.world_objects.iter().enumerate())
            .filter(|(_, o)| o.flags & WO_PSYS != 0 && o.flags & WO_PSYS_OFF == 0)
            .map(|(i, _)| i)
            .collect();
        for i in starts {
            self.world_psys_activate(i);
        }
    }

    /// WorldPsysActivate: object `i`'s `PSYS<id>` template, aimed down its
    /// collision triangle's (negated) normal, running forever.
    fn world_psys_activate(&mut self, i: usize) {
        let w = &self.level.worlds;
        let o = &w.world_objects[i];
        let Some(rec) = w.psys_for(o).cloned() else {
            log::debug!("WorldPsysActivate: no psys for {}", o.name);
            return;
        };
        let over = (o.coll_tri_count > 0)
            .then(|| w.coll_tris.get(o.coll_tri_index as usize))
            .flatten()
            .map(|t| t.norm.map(|v| -v));
        let d = (o.flags & WO_DYNAMIC) as i32;
        let mut p = Psys::from_record(&rec, &self.particles.presets, over, d, true);
        if rec.enables & enables::TEXTURE != 0 {
            p.tex = self.psys_texture(&rec.tex_name, None);
        }
        p.node = self.world_object_matrix(i);
        self.particles.add(Owner::World(i), p);
    }

    /// A world object's node matrix (its animated placement when it moves).
    fn world_object_matrix(&self, i: usize) -> Mat4 {
        match self.world_anim.world.get(i) {
            Some(m) if self.world_anim.dynamic.contains(&i) => *m,
            _ => Mat4::from_translation(Vec3::from(
                self.level.world_pos.get(i).copied().unwrap_or_default(),
            )),
        }
    }

    /// MBOX_FindTexture for an emitter: `own` (its bank's), else the
    /// level's, WEAPONS', POWERUPS' or STATIC's.
    pub(crate) fn psys_texture(&mut self, name: &str, own: Option<&Image>) -> Option<usize> {
        if name.trim().is_empty() {
            return None;
        }
        if let Some(img) = own {
            return Some(self.particles.texture(name, img.clone()));
        }
        if let Some(i) = self.particles.texture_named(name) {
            return Some(i);
        }
        let img = crate::mesh::decode_bitmap(&self.level.objects, &self.level.textures_file, name)
            .or_else(|| {
                self.psys_banks
                    .iter()
                    .find_map(|b| crate::mesh::decode_bitmap(&b.0, &b.1, name))
            })?;
        Some(self.particles.texture(name, img))
    }

    /// An entity's node frame as drawn (the scene's transform).
    fn entity_xform(&self, e: &Entity) -> Mat4 {
        let look = matches!(e.kind, Kind::Effect { .. })
            .then(|| self.fx_look(e.id))
            .flatten();
        let scale = look.and_then(|l| l.scale).unwrap_or(Vec3::splat(e.scale));
        let mut m = Mat4::from_translation(e.pos)
            * Mat4::from_rotation_y(e.yaw + e.spin)
            * Mat4::from_rotation_x(e.pitch)
            * Mat4::from_scale(scale);
        if let Kind::Critter(c) = &e.kind {
            m *= Mat4::from_scale(Vec3::splat(c.scale));
        }
        m
    }

    /// AtreeNodeInit's particle nodes for an entity's tree.
    fn start_entity_psys(&mut self, id: u64, model: usize) {
        self.start_tree_psys(model, |node| Owner::Entity { id, model, node });
    }

    /// The particle nodes of an instance of `model`'s tree.
    fn start_tree_psys(&mut self, model: usize, owner: impl Fn(usize) -> Owner) {
        let nodes: Vec<_> = self.models[model].psys.clone();
        for (k, mp) in nodes.into_iter().enumerate() {
            let mut p =
                Psys::from_record(&mp.record, &self.particles.presets, Some(mp.dir), 1, false);
            if mp.record.enables & enables::TEXTURE != 0 {
                let key = (model, k);
                p.tex = match self.particles.tex_cache.get(&key) {
                    Some(t) => *t,
                    None => {
                        let t = self.psys_texture(&mp.record.tex_name, mp.image.as_ref());
                        self.particles.tex_cache.insert(key, t);
                        t
                    }
                };
            }
            if !self.particles.add(owner(mp.node), p) {
                break;
            }
        }
    }

    /// PsfxDoParticle: a plyr_sfx particle record (flags & 0x0F000000) on
    /// hero `pi`, at the node its snddesc names (else the hero), at its offset.
    pub(crate) fn psfx_particle(
        &mut self,
        pi: usize,
        rec: &gdl_formats::pdata::SfxRecord,
        tex: Option<usize>,
    ) {
        let f = rec.flags;
        // lbl_80347E28 / D98: rate = 30 x radius, speed = 0.01 x alphamod.
        let rate = (30.0 * rec.radius as f64) as f32;
        let speed = (0.01 * rec.alphamod as f64) as f32;
        let mut p = Psys::new_default(1);
        if f & 0x0F00_0000 == 0x0200_0000 {
            p.set_e_volume(0.2, 0.0);
            if rec.scale as f64 != 1.0 {
                p.scale_p_parm(rec.scale, 4);
            }
            p.set_e_rate4([rate; 4]);
            p.set_e_time(rec.maxlen, 0.034);
        } else {
            p.set_p_time(360.0);
            if rec.scale as f64 != 1.0 {
                p.scale_p_parm(rec.scale, 4);
            }
            p.set_e_rate4([rate; 4]);
            p.set_e_time(rec.maxlen, 0.034);
            p.set_e_volume(0.5, 0.5);
        }
        p.set_p_speed(speed);
        p.tex = tex;
        // Flag 0x40000 (the chain's effect node) and 0x2000 (the grab
        // partner's) hang it on the hero here.
        let node = if f & 0x4_2000 == 0 && !rec.snddesc.is_empty() {
            self.models[self.players[pi].model].node_index(&rec.snddesc)
        } else {
            None
        };
        let owner = Owner::Hero {
            player: pi,
            node,
            offset: rec.offset,
        };
        p.node = self.hero_node(pi, node, rec.offset);
        self.particles.add(owner, p);
    }

    /// A hero's node frame (p->node, or one of its tree's nodes) plus an offset.
    fn hero_node(&self, pi: usize, node: Option<usize>, offset: [f32; 3]) -> Mat4 {
        let p = &self.players[pi];
        let base = self.player_xform(pi) * Mat4::from_translation(Vec3::Y * p.worn.lift);
        let local = node
            .and_then(|n| {
                let m = &self.models[p.model];
                m.pose_layers(&[(None, p.animator.seq, p.animator.frame)])
                    .get(n)
                    .copied()
            })
            .unwrap_or(Mat4::IDENTITY);
        base * local * Mat4::from_translation(Vec3::from(offset))
    }

    /// SfxSetStreak for a hero's missile `id` (StartMissile, owner > 0):
    /// its colour by the hero's, intensity by class (+64 for a big one),
    /// white at 64 for a super shot.
    pub(crate) fn start_streak(
        &mut self,
        id: u64,
        pi: usize,
        damage_type: u32,
        radius: f32,
        maxtime: f32,
    ) {
        let p = &self.players[pi];
        let big = damage_type & crate::missile::DMG_TURBO != 0;
        let (color, alpha) = if damage_type & crate::missile::DMG_SUPER != 0 && !big {
            (0xFF_FFFF, 64)
        } else {
            let colour = match p.color.as_str() {
                "YEL" => 0,
                "BLU" => 1,
                "RED" => 2,
                _ => 3,
            };
            let mut a = STREAK_INTENSITY[p.class_id % 8];
            if big {
                a += 64;
                if a >= 255 {
                    a = 255;
                }
            }
            (effect::COLOUR_TINT[colour], a)
        };
        let fwdmul = self.pdata.get(p.model).map_or(1.0, |d| d.streak_fwd_mul);
        self.particles.streak_of.insert(
            id,
            Streak {
                color,
                alpha,
                maxtime,
                // lbl_80346328 is 0: streakscale is the colrad.
                scale: radius,
                fwdmul,
            },
        );
    }

    /// Testing aid (GDL_FX): start effect row `what` (a number, or GENHIT /
    /// GENDIE of the first monster type that has one) 4 ahead of player 0.
    pub fn debug_fx(&mut self, what: &str) -> bool {
        // EXP<type>: StartExplosion of that type (25 the poison barrel) 6
        // ahead on the floor, with its damage (10 x trap_damage).
        if let Some(ty) = what
            .strip_prefix("EXP")
            .and_then(|t| t.parse::<usize>().ok())
        {
            let p = &self.players[0];
            let at = p.pos + Vec3::new(p.yaw.det_sin(), 0.0, p.yaw.det_cos()) * 6.0;
            let dmg = 10.0 * self.tuning.trap_damage;
            self.start_explosion(at, ty, dmg);
            return true;
        }
        // BLOCK: StartBlockFX on player 0, as a full-strength clank.
        if what == "BLOCK" {
            return self.start_block_fx(0, 100.0).is_some();
        }
        let row = match what {
            "GENHIT" => self.effects.hitfx.iter().copied().find(|&r| r >= 0),
            "GENDIE" => self.effects.deathfx.iter().copied().find(|&r| r >= 0),
            n => n.parse::<i32>().ok(),
        };
        let Some(row) = row.and_then(|r| usize::try_from(r).ok()) else {
            return false;
        };
        let p = &self.players[0];
        let at = p.pos + Vec3::new(p.yaw.det_sin(), 0.0, p.yaw.det_cos()) * 4.0 + Vec3::Y * 2.0;
        let yaw = p.yaw;
        self.start_fx(row, at, yaw, 0.0).is_some()
    }

    /// Each tick: start the particle nodes of what came into play, move the
    /// emitters with their nodes (MBRemovePsys when the owner goes), run the
    /// psys frame, and rebuild the streaks.
    pub(crate) fn update_particles(&mut self) {
        let index: BTreeMap<u64, usize> = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.alive)
            .map(|(i, e)| (e.id, i))
            .collect();
        // AtreeNodeInit: entities whose tree has particle nodes, once each.
        self.particles.started.retain(|(id, m)| {
            index
                .get(id)
                .is_some_and(|&i| self.entities[i].model == Some(*m))
        });
        let start: Vec<(u64, usize)> = self
            .entities
            .iter()
            .filter(|e| e.alive)
            .filter_map(|e| Some((e.id, e.model?)))
            .filter(|&(id, m)| {
                !self.models[m].psys.is_empty() && !self.particles.started.contains(&(id, m))
            })
            .collect();
        for (id, m) in start {
            self.particles.started.insert((id, m));
            self.start_entity_psys(id, m);
        }
        // The trees the heroes wear, placed as the scene draws them.
        let mut worn: BTreeMap<(usize, usize), Vec<Mat4>> = BTreeMap::new();
        for pi in 0..self.players.len() {
            for (m, xf, seq, frame, _) in self.worn_draws(pi) {
                if self.models[m].psys.is_empty() || worn.contains_key(&(pi, m)) {
                    continue;
                }
                let base = self.player_xform(pi) * xf;
                let nodes = self.models[m].pose_layers(&[(None, seq, frame)]);
                worn.insert((pi, m), nodes.into_iter().map(|n| base * n).collect());
            }
        }
        let have: BTreeSet<(usize, usize)> = self
            .particles
            .systems
            .iter()
            .filter_map(|(o, _)| match o {
                Owner::Worn { player, model, .. } => Some((*player, *model)),
                _ => None,
            })
            .collect();
        let mut keys: Vec<(usize, usize)> =
            worn.keys().copied().filter(|k| !have.contains(k)).collect();
        keys.sort();
        for (player, model) in keys {
            self.start_tree_psys(model, |node| Owner::Worn {
                player,
                model,
                node,
            });
        }
        // fn_80062A00: a trigger's PSYS target burns while it is triggered
        // (WorldPsysActivate / WorldPsysDeActivate).
        let toggles: Vec<(usize, bool)> = self
            .movers
            .iter()
            .filter(|m| {
                self.level
                    .worlds
                    .world_objects
                    .get(m.root)
                    .is_some_and(|o| o.flags & WO_PSYS != 0)
            })
            .map(|m| (m.root, m.active))
            .collect();
        for (o, on) in toggles {
            let owner = Owner::World(o);
            let live = self.particles.systems.iter().any(|(w, _)| *w == owner);
            if on && !live {
                self.world_psys_activate(o);
            } else if !on && live {
                self.particles.remove_owner(&owner);
            }
        }
        // Node matrices: each owner's tree posed once.
        let mut posed: BTreeMap<u64, (Mat4, Vec<Mat4>)> = BTreeMap::new();
        let mut systems = std::mem::take(&mut self.particles.systems);
        for (owner, p) in &mut systems {
            match *owner {
                Owner::Entity { id, model, node } => {
                    let Some(&i) = index
                        .get(&id)
                        .filter(|&&i| self.entities[i].model == Some(model))
                    else {
                        // MBRemoveNode of the tree: MBRemovePsys.
                        p.remove();
                        *owner = Owner::Detached;
                        continue;
                    };
                    let (xf, nodes) = posed.entry(id).or_insert_with(|| {
                        let e = &self.entities[i];
                        let m = &self.models[model];
                        let nodes = match &e.kind {
                            Kind::Critter(c) => c.pose(m),
                            _ => m.pose_layers(&[(None, e.animator.seq, e.animator.frame)]),
                        };
                        (self.entity_xform(e), nodes)
                    });
                    let world = *xf * nodes.get(node).copied().unwrap_or(Mat4::IDENTITY);
                    p.scale = world.y_axis.truncate().length();
                    p.node = world;
                    p.hidden = self.entities[i].fade <= 0.0;
                }
                Owner::World(o) => p.node = self.world_object_matrix(o),
                Owner::Worn {
                    player,
                    model,
                    node,
                } => match worn.get(&(player, model)) {
                    Some(nodes) => {
                        let world = nodes.get(node).copied().unwrap_or(Mat4::IDENTITY);
                        p.scale = world.y_axis.truncate().length();
                        p.node = world;
                    }
                    None => {
                        p.remove();
                        *owner = Owner::Detached;
                    }
                },
                Owner::Hero {
                    player,
                    node,
                    offset,
                } => {
                    if player < self.players.len() {
                        p.node = self.hero_node(player, node, offset);
                    }
                }
                Owner::Detached => {}
            }
        }
        self.particles.systems = systems;
        // MBPsysStartFrame: a psys frame each 30 Hz game frame.
        let (eye, at, near) = self.camera_view();
        let vp =
            crate::camera::projection(4.0 / 3.0, near, 1500.0) * Mat4::look_at_rh(eye, at, Vec3::Y);
        self.particles
            .update(self.frame / 2, |p, r| sphere_visible(&vp, p, r));
        // UpdateFXStreak.
        let cam_z = (at - eye).normalize_or_zero();
        let tex = self.particles.streak_tex;
        let mut quads = Vec::new();
        let entities = &self.entities;
        self.particles.streak_of.retain(|id, s| {
            let Some(e) = index.get(id).map(|&i| &entities[i]) else {
                return false;
            };
            let Kind::Projectile { vel, life, .. } = &e.kind else {
                return false;
            };
            quads.push(StreakQuad {
                corners: streak_quad(e.pos, *vel, s.maxtime - *life, s, cam_z),
                color: s.argb(),
                tex,
            });
            true
        });
        self.particles.streaks = quads;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    fn tick(g: &mut Game, n: usize) {
        for _ in 0..n {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
    }

    #[test]
    fn world_emitters_run_where_their_objects_are() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelH1", &[("WAR", "RED")]).unwrap();
        let w = &g.level.worlds;
        let want = w
            .world_objects
            .iter()
            .filter(|o| o.flags & WO_PSYS != 0 && w.psys_for(o).is_some())
            .count();
        let world = |g: &Game| {
            g.particles
                .systems
                .iter()
                .filter(|(o, _)| matches!(o, Owner::World(_)))
                .count()
        };
        assert!(want > 100);
        assert_eq!(world(&g), want);
        // POOLFIRE and the torches' flames are WEAPONS textures.
        assert!(g.particles.texture_named("POOLFIRE").is_some());
        assert!(g.particles.default_tex.iter().all(|t| t.is_some()));
        tick(&mut g, 60);
        // World emitters never end; those in view are burning.
        assert_eq!(world(&g), want);
        let burning = g
            .particles
            .systems
            .iter()
            .filter(|(o, p)| matches!(o, Owner::World(_)) && !p.is_empty())
            .count();
        assert!(burning > 0);
        assert!(g.particles.batches().iter().any(|b| !b.sprites.is_empty()));
    }

    #[test]
    fn gem_sparkles_burst_and_go() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        tick(&mut g, 2);
        let at = g.players[0].pos + Vec3::Y * 2.0;
        let id = g.start_gem_fx(at, 2).expect("GETGEMRED");
        let mine = |g: &Game| {
            g.particles
                .systems
                .iter()
                .filter(|(o, _)| matches!(o, Owner::Entity { id: i, .. } if *i == id))
                .map(|(_, p)| p.len())
                .collect::<Vec<_>>()
        };
        tick(&mut g, 1);
        // GETGEMRED: two emitters on record 5.
        assert_eq!(mine(&g).len(), 2);
        tick(&mut g, 20);
        assert!(mine(&g).iter().sum::<usize>() > 0, "{:?}", mine(&g));
        // When the effect ends its emitters stop and live out their particles.
        tick(&mut g, 600);
        assert!(mine(&g).is_empty());
        assert!(!g.entities.iter().any(|e| e.id == id && e.alive));
    }

    #[test]
    fn wizard_wrists_spark() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WIZ", "BLU")]).unwrap();
        tick(&mut g, 2);
        // sfx[0]: WIZ_HEAD_Y at R_WRIST, chaining to [1] at L_WRIST.
        g.do_plyr_sfx_sub(0, 0, None, false, None, 0);
        let hero: Vec<_> = g
            .particles
            .systems
            .iter()
            .filter_map(|(o, p)| match o {
                Owner::Hero { node, .. } => Some((*node, p.tex)),
                _ => None,
            })
            .collect();
        assert_eq!(hero.len(), 2);
        let m = &g.models[g.players[0].model];
        assert_eq!(hero[0].0, m.node_index("R_WRIST"));
        assert_eq!(hero[1].0, m.node_index("L_WRIST"));
        assert!(hero.iter().all(|h| h.0.is_some() && h.1.is_some()));
        tick(&mut g, 10);
        let n: usize = g.particles.systems.iter().map(|(_, p)| p.len()).sum();
        assert!(n > 0);
        // 0.5 s of emitting and 0.2 s particles: gone within a second.
        tick(&mut g, 60);
        assert!(!g
            .particles
            .systems
            .iter()
            .any(|(o, _)| matches!(o, Owner::Hero { .. })));
    }

    #[test]
    fn spheres_cull_against_the_view() {
        let vp = crate::camera::projection(4.0 / 3.0, 1.0, 1500.0)
            * Mat4::look_at_rh(Vec3::ZERO, Vec3::new(0.0, 0.0, -10.0), Vec3::Y);
        assert!(sphere_visible(&vp, Vec3::new(0.0, 0.0, -20.0), 1.0));
        assert!(!sphere_visible(&vp, Vec3::new(0.0, 0.0, 20.0), 1.0));
        // Just off the side, but within its radius of it.
        assert!(!sphere_visible(&vp, Vec3::new(40.0, 0.0, -20.0), 1.0));
        assert!(sphere_visible(&vp, Vec3::new(40.0, 0.0, -20.0), 30.0));
    }
}
