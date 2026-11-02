//! items.c's breakable walls: obstacle items (type 10) of subtype 0x2A, the
//! levels' `*SHOOTW*` / `*RUNEWALL` objects that close off secret rooms.
//!
//! SetItem case 10 places them like any obstacle: the model is the object the
//! instance names (ItemFindMBObjectL1: the name, then +L1, then +ROOT; none
//! for instance flag 2), the collision is the instance's own triangles
//! (coltype 4), 25 health and armour 1 from their item info. fn_8005C1DC
//! takes a hit off their health (less the armour, at least 1); a wall still
//! standing flashes white (data.obsticle.flash 1) and plays the level's
//! `hitsnd`, one brought to 0 plays S_SECRETWALL and is deleted with its
//! collision, either way with fn_80094440's hit / die effect. What reaches
//! them: missiles (fn_8005ED44 at launch and in flight), the melee swing of a
//! hero touching one (PlayerCollideEnemies' collision item; message 20, "takes
//! multiple hits", since their info has no name) and blasts (StartExplosion);
//! poison gas and potion magic don't (fn_8005C1DC's 0x800 rule, SfxSkipItem's
//! 0x200), but an archer's potion (char_type & 3 == 3) at level 25 and up
//! shows them (flash 4, message 0x91), at 50 and up destroys them (message
//! 0x92) (fn_8005BA1C). Monsters and critters only bump into them.
//!
//! A child of `game`, so it reaches the game's private state.

use super::*;

/// The breakable wall's item subtype.
pub(crate) const SUB_WALL: i32 = 0x2A;
/// AudioExplodeWall's sound for a wall brought down (S_SECRETWALL).
const SFX_SECRETWALL: i32 = 46;
/// fn_80094440's effect rows by element: lbl_80122DAC for a hit,
/// lbl_80122DC0 for the blow that destroys.
const HIT_FX: [usize; 5] = [2, 11, 14, 17, 20];
const DIE_FX: [usize; 5] = [3, 12, 15, 18, 21];
/// msgPost: "takes multiple hits", "magic shows walls", "magic destroys
/// walls".
const MSG_MULTIPLE_HITS: usize = 20;
const MSG_SHOW_WALLS: usize = 0x91;
const MSG_DESTROY_WALLS: usize = 0x92;
/// fn_8005C1DC's v[]: the hit point is 2 above the collision position.
const HIT_RISE: f32 = 2.0;

/// One breakable wall (an Item of type 10, subtype 0x2A).
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    pub health: i32,
    pub armor: i32,
    /// The item info's radius and height (fn_8005F0F4).
    pub radius: f32,
    pub height: f32,
    /// coll_pos - pos: the info's coloffset, raised 1 (SetItem).
    pub coll: Vec3,
    /// The instance's collision triangles (ctriidx, nctris).
    pub tris: Option<(usize, usize)>,
    /// data.obsticle.flash and timer (60 Hz ticks): 1 flashes white for a
    /// frame (a hit), more blinks it (half of each second white) until 1.
    pub flash: i16,
    pub timer: f32,
    /// Drawn white (MBTreeSetAltTex whitetex) this frame.
    pub white: bool,
    /// fxhittime: the game time before which a blast can't hit it again.
    pub fxhit: f32,
}

impl Wall {
    /// SetItem for an instance of `info`.
    pub(crate) fn new(info: &gdl_formats::worlds::ItemInfo, tris: Option<(usize, usize)>) -> Wall {
        Wall {
            health: info.hit_points as i32,
            armor: info.armor as i32,
            radius: info.radius,
            height: info.height,
            coll: Vec3::from(info.coll_offset) + Vec3::Y,
            tris,
            flash: 0,
            timer: 0.0,
            white: false,
            fxhit: 0.0,
        }
    }

    /// UpdateItems case 10: one frame of the flash.
    fn update_flash(&mut self, dt: f32) {
        if self.flash > 1 {
            self.white = self.timer < 30.0;
            self.timer += dt * 60.0;
            if self.timer > 60.0 {
                self.timer = 0.0;
                self.flash -= 1;
                if self.flash == 1 {
                    self.flash = 0;
                }
            }
        } else if self.flash == 1 {
            self.white = true;
            self.flash = 0;
            self.timer = 0.0;
        } else {
            self.white = false;
        }
    }
}

/// The effective subtype of an obstacle instance: SetItem case 10's
/// params[0] when positive, else the info's.
pub(crate) fn obstacle_subtype(info: &gdl_formats::worlds::ItemInfo, params: &[u8; 12]) -> i32 {
    let p0 = i16::from_le_bytes([params[0], params[1]]);
    if p0 > 0 {
        p0 as i32
    } else {
        info.subtype
    }
}

impl Game {
    /// SetItem case 10 for a breakable wall: its model (`model`, from the
    /// object the instance names) and its own collision triangles.
    pub(super) fn place_wall(
        &mut self,
        inst: &gdl_formats::worlds::ItemInstance,
        info: &gdl_formats::worlds::ItemInfo,
        model: Option<usize>,
        pos: Vec3,
        yaw: f32,
    ) -> u64 {
        let tris = (inst.coll_tri_count > 0 && inst.coll_tri_index >= 0)
            .then_some((inst.coll_tri_index as usize, inst.coll_tri_count as usize));
        self.spawn(Kind::Wall(Wall::new(info, tris)), model, pos, yaw)
    }

    /// The live wall entity a collision triangle belongs to.
    pub(crate) fn wall_of_tri(&self, tri: usize) -> Option<usize> {
        self.entities.iter().position(|e| {
            e.alive
                && matches!(&e.kind, Kind::Wall(w)
                    if w.tris.is_some_and(|(s, n)| (s..s + n).contains(&tri)))
        })
    }

    /// fn_8005C1DC for a breakable wall: a hit of `power` with damage type
    /// `flags`. Returns the health left (0: destroyed), -1 or -2 when it
    /// takes no harm (poison gas).
    pub(crate) fn damage_wall(
        &mut self,
        t: usize,
        power: f32,
        flags: u32,
        _owner: Option<usize>,
    ) -> f32 {
        let e = &mut self.entities[t];
        let (pos, yaw) = (e.pos, e.yaw);
        let Kind::Wall(w) = &mut e.kind else {
            return -1.0;
        };
        if !e.alive {
            return -1.0;
        }
        if flags & DmgType::POISON_GAS != 0 {
            return if power <= 2.0 { -1.0 } else { -2.0 };
        }
        let mut destroyed = false;
        if w.armor >= 0 {
            let mut power = power - w.armor as f32;
            if power <= 0.0 {
                power = 1.0;
            }
            w.health = (w.health - power.round() as i32).max(0);
            destroyed = w.health == 0;
        }
        if w.armor == -1 {
            return -1.0;
        }
        let health = w.health;
        if !destroyed {
            w.flash = 1;
        }
        let tris = w.tris;
        let at = pos + w.coll + Vec3::Y * HIT_RISE;
        // AudioExplodeWall.
        if health > 0 {
            let name = self
                .wave
                .and_then(|wv| self.worlds.level_sound(wv, true))
                .map(|n| n.chars().take(15).collect::<String>());
            if let Some(n) = name {
                self.sfx.push(n);
            }
        } else {
            self.sfx_id(SFX_SECRETWALL);
        }
        if destroyed {
            self.entities[t].alive = false;
            if let Some((s, n)) = tris {
                for i in s..(s + n).min(self.collision.enabled.len()) {
                    self.collision.enabled[i] = false;
                }
            }
        }
        let el = ((flags & 0xF) as usize).min(4);
        self.start_fx(
            if destroyed { DIE_FX[el] } else { HIT_FX[el] },
            at,
            yaw,
            0.0,
        );
        health as f32
    }

    /// The wall a swing of hero `pi` at `pos` facing `dir` lands on
    /// (PlayerGetTarget's collision item): one the hero touches (or, not
    /// moving, one 2 radii ahead touches), within 45 degrees of the facing
    /// and nearer than the swing's reach (2 + radius) less its own radius.
    pub(super) fn melee_wall(&self, pi: usize, pos: Vec3, dir: Vec2) -> Option<usize> {
        let mv = self.players[pi].trial_move;
        let still = Vec2::new(mv.x, mv.z).length() < 1e-4;
        let ahead = pos + Vec3::new(dir.x, 0.0, dir.y) * (2.0 * PLAYER_RADIUS);
        let mut best: Option<(usize, f32)> = None;
        for (i, e) in self.entities.iter().enumerate() {
            let Kind::Wall(w) = &e.kind else { continue };
            let Some((s, n)) = w.tris.filter(|_| e.alive) else {
                continue;
            };
            let touch = |p: Vec3| {
                self.collision
                    .touches(s..s + n, p, PLAYER_RADIUS + 0.05, PLAYER_HEIGHT)
            };
            if !(touch(pos) || still && touch(ahead)) {
                continue;
            }
            let c = e.pos + w.coll;
            let d = Vec2::new(c.x - pos.x, c.z - pos.z);
            let len = d.length();
            if len > 1e-4 && (d / len).dot(dir) < 0.707 {
                continue;
            }
            let dist = len - w.radius;
            if dist < 2.0 + PLAYER_RADIUS && best.is_none_or(|(_, b)| dist < b) {
                best = Some((i, dist));
            }
        }
        best.map(|(i, _)| i)
    }

    /// PlayerMotion_HitTarget on a wall: the hit, then message 20 (its info
    /// has no name) when it took it.
    pub(super) fn melee_hit_wall(&mut self, pi: usize, t: usize, dmg: f32, dtype: u32) {
        if self.damage_wall(t, dmg, dtype, Some(pi)) >= 0.0 {
            self.post(MSG_MULTIPLE_HITS, Some(pi), &[]);
        }
    }

    /// Whether the effect at `centre` with reach `radius` gets at wall `e`
    /// (ProcessEffects' item pass: the 2D reach plus the item's radius, the
    /// height, then fn_8005F0F4's triangle test with the reach as margin).
    pub(crate) fn effect_reaches_wall(&self, e: &Entity, centre: Vec3, radius: f32) -> bool {
        let Kind::Wall(w) = &e.kind else {
            return false;
        };
        let Some((s, n)) = w.tris.filter(|_| e.alive) else {
            return false;
        };
        let c = e.pos + w.coll;
        Vec2::new(c.x - centre.x, c.z - centre.z).length() <= radius + w.radius
            && (centre.y - c.y).abs() <= w.height + radius
            && self
                .collision
                .touches(s..s + n, centre - Vec3::Y * radius, radius, 2.0 * radius)
    }

    /// The item part of a blast (ProcessEffects, flags 2) on walls: an
    /// explosion reaches 1.5 less far (the caller's `r`); SfxSkipItem
    /// leaves them to effects with flag 0x1000 and magic damage.
    pub(super) fn blast_walls(&mut self, b: &super::items::Blast, r: f32, dmg: f32, age: f32) {
        let (pos, dtype, flags, owner) = (b.pos, b.dtype, b.flags, b.owner);
        if flags & 0x1000 != 0 || dtype & DmgType::MAGIC != 0 || r <= 0.0 {
            return;
        }
        let hits: Vec<(usize, u64)> = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                matches!(&e.kind, Kind::Wall(w)
                    if flags & 0x100_0000 != 0 || self.time >= w.fxhit)
                    && self.effect_reaches_wall(e, pos, r)
            })
            .map(|(i, e)| (i, e.id))
            .collect();
        let until = self.time + age;
        for (i, _) in hits {
            self.damage_wall(i, dmg, dtype, owner);
            if let (true, Kind::Wall(w)) = (dmg > 2.0, &mut self.entities[i].kind) {
                w.fxhit = until;
            }
        }
    }

    /// fn_8005BA1C's obstacle case for a DMG_HEAL potion of hero `pi`
    /// reaching wall `t`: an archer-type hero (char_type & 3 == 3) at level
    /// 50 brings it down, below that makes it blink (flash 4).
    pub(crate) fn wall_magic(&mut self, pi: usize, t: usize) {
        let Some(p) = self.players.get(pi) else {
            return;
        };
        if p.class_id & 3 != 3 || !self.entities[t].alive {
            return;
        }
        let msg = if p.level >= 50 {
            self.damage_wall(t, 9999.0, 0, Some(pi));
            MSG_DESTROY_WALLS
        } else {
            if let Kind::Wall(w) = &mut self.entities[t].kind {
                w.flash = 4;
            }
            MSG_SHOW_WALLS
        };
        self.post(msg, Some(pi), &[]);
    }

    /// UpdateItems case 10 for the walls: their flash.
    pub(super) fn update_walls(&mut self, dt: f32) {
        for e in &mut self.entities {
            if let Kind::Wall(w) = &mut e.kind {
                w.update_flash(dt);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(hp: i16, armor: i16) -> gdl_formats::worlds::ItemInfo {
        gdl_formats::worlds::ItemInfo {
            item_type: ItemType::Obstacle,
            subtype: SUB_WALL,
            coll_type: 4,
            coll_flags: 0,
            radius: 10.0,
            height: 10.0,
            x_dim: 0.0,
            z_dim: 0.0,
            coll_offset: [0.0; 3],
            name: String::new(),
            properties: 0,
            value: 0,
            armor,
            hit_points: hp,
            active_type: 0x40,
            active_off: 0,
            active_on: 0,
            random_indices: Vec::new(),
        }
    }

    #[test]
    fn params_override_the_subtype() {
        let i = info(25, 1);
        let mut p = [0u8; 12];
        assert_eq!(obstacle_subtype(&i, &p), SUB_WALL);
        p[0] = 0x28;
        assert_eq!(obstacle_subtype(&i, &p), 0x28);
        p[1] = 0xFF;
        assert_eq!(obstacle_subtype(&i, &p), SUB_WALL);
    }

    #[test]
    fn a_hit_flashes_one_frame_and_magic_blinks_three_times() {
        let mut w = Wall::new(&info(25, 1), None);
        w.flash = 1;
        w.update_flash(1.0 / 60.0);
        assert!(w.white);
        w.update_flash(1.0 / 60.0);
        assert!(!w.white);
        w.flash = 4;
        let mut white = 0;
        let mut frames = 0;
        while w.flash > 0 && frames < 1000 {
            w.update_flash(1.0 / 60.0);
            white += w.white as usize;
            frames += 1;
        }
        // Three seconds, half of each white.
        assert!((179..=186).contains(&frames), "{frames}");
        assert!((88..=96).contains(&white), "{white}");
    }

    fn data() -> Option<GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        GameFs::open(&root).ok()
    }

    /// levelA1 with nothing about, the hero facing A1SHOOTW#1 from in front.
    fn a1() -> Option<(Game, usize)> {
        let fs = data()?;
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).ok()?;
        g.pending_enemies.clear();
        g.entities.retain(|e| {
            !matches!(
                e.kind,
                Kind::Enemy { .. } | Kind::Generator { .. } | Kind::Critter(_)
            )
        });
        g.players[0].health = 10_000.0;
        let walls: Vec<usize> = (0..g.entities.len())
            .filter(|&i| matches!(g.entities[i].kind, Kind::Wall(_)))
            .collect();
        assert_eq!(walls.len(), 9, "levelA1 has nine breakable walls");
        let t = walls
            .into_iter()
            .find(|&i| (g.entities[i].pos - Vec3::new(56.77, -2.0, -0.2)).length() < 0.1)?;
        Some((g, t))
    }

    fn wall(g: &Game, t: usize) -> &Wall {
        match &g.entities[t].kind {
            Kind::Wall(w) => w,
            _ => panic!("not a wall"),
        }
    }

    #[test]
    fn walls_are_placed_with_their_model_and_collision() {
        let Some((g, t)) = a1() else { return };
        let w = wall(&g, t);
        assert_eq!((w.health, w.armor), (25, 1));
        assert!(g.entities[t].model.is_some());
        let (s, n) = w.tris.expect("its own triangles");
        assert!((s..s + n).all(|i| g.collision.enabled[i]));
        assert_eq!(g.wall_of_tri(s), Some(t));
    }

    #[test]
    fn hits_wear_a_wall_down_and_its_collision_goes_with_it() {
        let Some((mut g, t)) = a1() else { return };
        let (s, n) = wall(&g, t).tris.unwrap();
        // Armour 1 takes 1 off each hit, at least 1 gets through.
        assert_eq!(g.damage_wall(t, 10.0, 0, Some(0)), 16.0);
        assert_eq!(wall(&g, t).flash, 1);
        assert_eq!(g.damage_wall(t, 0.5, 0, Some(0)), 15.0);
        // Poison gas does nothing.
        assert_eq!(g.damage_wall(t, 50.0, DmgType::POISON_GAS, None), -2.0);
        assert_eq!(wall(&g, t).health, 15);
        assert!((s..s + n).all(|i| g.collision.enabled[i]));
        g.sfx.clear();
        assert_eq!(g.damage_wall(t, 30.0, 0, Some(0)), 0.0);
        assert!(!g.entities[t].alive);
        assert!((s..s + n).all(|i| !g.collision.enabled[i]));
        assert!(
            g.sfx.iter().any(|s| s == "#2e"),
            "S_SECRETWALL: {:?}",
            g.sfx
        );
        assert_eq!(g.damage_wall(t, 30.0, 0, Some(0)), -1.0);
        assert_eq!(g.wall_of_tri(s), None);
    }

    /// The entity with this id, while it stands.
    fn standing(g: &Game, id: u64) -> Option<&Wall> {
        g.entities.iter().find_map(|e| match &e.kind {
            Kind::Wall(w) if e.id == id && e.alive => Some(w),
            _ => None,
        })
    }

    #[test]
    fn explosions_wear_walls_down() {
        let Some((mut g, t)) = a1() else { return };
        let id = g.entities[t].id;
        let at = g.entities[t].pos + Vec3::new(0.0, 1.0, -2.0);
        // Explosive barrels' blasts (30 x trap_damage, falling off as they
        // spread): each takes a share, a couple bring it down.
        let mut blasts = 0;
        while standing(&g, id).is_some() && blasts < 5 {
            g.start_explosion(at, 24, 30.0 * g.tuning.trap_damage);
            for _ in 0..90 {
                g.update(1.0 / 60.0, &[PlayerInput::default()]);
            }
            blasts += 1;
            if blasts == 1 {
                let w = standing(&g, id).map_or(0, |w| w.health);
                assert!(w < 25, "the first blast takes some off: {w}");
            }
        }
        assert!(standing(&g, id).is_none(), "{blasts} blasts");
        // Poison gas leaves them be.
        let Some((mut g, t)) = a1() else { return };
        let id = g.entities[t].id;
        let at = g.entities[t].pos + Vec3::new(0.0, 1.0, -2.0);
        g.start_explosion(at, 25, 10.0 * g.tuning.trap_damage);
        for _ in 0..240 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert_eq!(standing(&g, id).map(|w| w.health), Some(25));
    }

    #[test]
    fn missiles_hit_walls_at_launch_and_in_flight() {
        let Some((mut g, t)) = a1() else { return };
        let id = g.entities[t].id;
        // A1SHOOTW#1 faces -x with floor at -7 on that side.
        let floor = Vec3::new(52.7, -7.04, -0.2);
        let dir = Vec3::X;
        // From 7 off: the shot flies to it.
        g.players[0].pos = floor - dir * 3.5;
        let hand = g.players[0].pos + Vec3::Y * 2.5;
        assert_eq!(g.start_missile_fan(0, hand, dir, 0.0, 10.0, 0, 1.0, 1.0), 1);
        for _ in 0..60 {
            g.update(1.0 / 60.0, &[PlayerInput::default()]);
        }
        assert_eq!(
            standing(&g, id).map(|w| w.health),
            Some(16),
            "hit in flight"
        );
        // Right in front: the launch sweep finds it, no missile flies.
        let close = floor + dir * 1.8;
        g.players[0].pos = close;
        let hand = close + Vec3::Y * 2.5;
        let t = g.entities.iter().position(|e| e.id == id).unwrap();
        assert_eq!(g.start_missile_fan(0, hand, dir, 0.0, 10.0, 0, 1.0, 1.0), 0);
        assert_eq!(wall(&g, t).health, 7, "hit at launch");
    }

    #[test]
    fn an_archers_potion_shows_or_destroys_walls() {
        let Some((mut g, t)) = a1() else { return };
        g.players[0].class_id = 3;
        g.players[0].level = 30;
        g.wall_magic(0, t);
        assert_eq!(wall(&g, t).flash, 4);
        assert!(g.entities[t].alive);
        g.players[0].level = 50;
        g.wall_magic(0, t);
        assert!(!g.entities[t].alive);
        // Other classes do nothing.
        let Some((mut g, t)) = a1() else { return };
        g.players[0].class_id = 0;
        g.players[0].level = 99;
        g.wall_magic(0, t);
        assert_eq!(wall(&g, t).flash, 0);
    }

    /// Walk the hero into the wall from in front and swing until it falls,
    /// then walk through where it stood.
    #[test]
    fn swings_break_a_wall_open() {
        let Some((mut g, t)) = a1() else { return };
        let id = g.entities[t].id;
        let c = g.entities[t].pos;
        let (s, n) = wall(&g, t).tris.unwrap();
        // The wall's plane and which side is open: start 4 out on the side
        // with floor.
        let tri = g.collision.tris[s];
        let normal = Vec2::new(tri.n.x, tri.n.z).normalize_or_zero();
        let mut start = None;
        for side in [1.0, -1.0] {
            let p =
                Vec3::new(c.x, c.y + 3.0, c.z) + Vec3::new(normal.x, 0.0, normal.y) * 4.0 * side;
            if let Some(y) = g.collision.ground_height(p, p.y + 2.0) {
                start = Some((Vec3::new(p.x, y, p.z), -normal * side));
                break;
            }
        }
        let Some((p0, toward)) = start else {
            panic!("no floor beside the wall")
        };
        let _ = n;
        g.players[0].pos = p0;
        g.players[0].yaw = toward.x.det_atan2(toward.y);
        // The stick for a world direction (camera relative).
        let stick = |g: &Game| {
            let (f, r) = g.camera_basis();
            Vec2::new(r.dot(toward), f.dot(toward))
        };
        for _ in 0..60 {
            let push = PlayerInput {
                stick: stick(&g),
                ..Default::default()
            };
            g.update(1.0 / 60.0, std::slice::from_ref(&push));
        }
        let stopped = g.players[0].pos;
        assert!(
            Vec2::new(stopped.x - p0.x, stopped.z - p0.z).length() < 4.0,
            "the wall stops the hero"
        );
        let mut frames = 0;
        let mut hits = 0;
        let mut last = 25;
        while standing(&g, id).is_some() && frames < 60 * 30 {
            let attack = PlayerInput {
                stick: stick(&g),
                attack: frames % 20 < 2,
                attack_pressed: frames % 20 == 0,
                ..Default::default()
            };
            g.update(1.0 / 60.0, std::slice::from_ref(&attack));
            frames += 1;
            let h = standing(&g, id).map_or(0, |w| w.health);
            hits += (h < last) as usize;
            last = h;
        }
        assert!(
            standing(&g, id).is_none(),
            "swings bring it down: {last} left"
        );
        assert!(hits > 1, "it takes several swings ({hits})");
        for _ in 0..90 {
            let push = PlayerInput {
                stick: stick(&g),
                ..Default::default()
            };
            g.update(1.0 / 60.0, std::slice::from_ref(&push));
        }
        let through = g.players[0].pos - p0;
        assert!(
            Vec2::new(through.x, through.z).dot(toward) > 5.0,
            "the hero walks through: {through:?}"
        );
    }
}
