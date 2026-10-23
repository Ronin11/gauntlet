//! Levers, pressure pads and the world geometry they drive (items.c: trigger items,
//! RegisterItemWobj and the per-frame world-object update). A trigger's target is a
//! world object that slides up or down between two heights, or fades in and out.

use crate::collision::Collision;
use crate::level::Level;
use crate::worldanim::{wflags, WorldAnims};
use glam::Vec3;

/// Rate moving world objects slide at (lbl_803470F0, units per second).
const SLIDE_SPEED: f32 = 4.0;
/// Toggle triggers wait this long before they can flip their target again (0x78 ticks).
const TOGGLE_IDLE: f32 = 2.0;

/// How a trigger drives its target (triggerdata.flags & 7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TriggerMode {
    /// flags & 1: deactivates the target while held.
    Release,
    /// flags & 2: activates once and stays.
    Latch,
    /// flags & 4: each press flips the target.
    Toggle,
    /// Otherwise: activates the target.
    Set,
}

/// Trigger behaviour from an item subtype and its instance flags (the table in the
/// item constructor, items.c case 5). Returns the mode and the target's flag byte.
pub fn trigger_flags(subtype: i32, params_flags: u16) -> (TriggerMode, u8) {
    let flags = trigger_bits(subtype, params_flags);
    let mode = if flags & 1 != 0 {
        TriggerMode::Release
    } else if flags & 2 != 0 {
        TriggerMode::Latch
    } else if flags & 4 != 0 {
        TriggerMode::Toggle
    } else {
        TriggerMode::Set
    };
    (mode, (flags & 0xFF) as u8)
}

/// triggerdata.flags bits above the target's byte (SetItem case 5 keeps
/// params[2] & ~0xFF; item update case 5 reads them).
pub mod tflags {
    /// A hero must stand on the target (or one of its children).
    pub const ON_TARGET: u16 = 0x100;
    /// Every hero in play must be on the pad (messages 126/127).
    pub const ALL_HEROES: u16 = 0x400;
    /// A toggle left empty may flip again after (heroes - 1) seconds.
    pub const SHARE_IDLE: u16 = 0x800;
    /// Setting it off shakes the camera.
    pub const SHAKE: u16 = 0x1000;
    /// Setting it off wakes the nearest statue (an ambush).
    pub const AMBUSH: u16 = 0x2000;
}

/// The whole triggerdata.flags word: the subtype's table byte (or params[2]
/// | 8) with params[2]'s high bits.
pub fn trigger_bits(subtype: i32, params_flags: u16) -> u16 {
    let base: u16 = match subtype {
        20 => 0x10,  // bridge_pad
        21 => 8,     // door_pad
        22 => 0x12,  // bridge_switch
        23 => 10,    // door_switch
        25 => 0x804, // elevator_pad
        26 => 2,     // elevator_switch
        27 => 0x80C, // lift_pad
        28 => 9,     // lift_start
        29 => 10,    // lift_end
        _ => params_flags | 8,
    };
    base | (params_flags & !0xFF)
}

/// Item update case 5's daction: whether the trigger shows as set off (2)
/// this frame, from its mode, whether a hero holds it (`mask`), its target's
/// triggered state after this frame's press (`active`; None without a
/// target) and last frame's daction (`was`: a target-less non-toggle stays
/// set off once it has been).
pub fn daction(mode: TriggerMode, mask: bool, active: Option<bool>, was: bool) -> bool {
    match active {
        Some(active) => {
            mask || match mode {
                TriggerMode::Release => !active,
                TriggerMode::Toggle => false,
                TriggerMode::Latch | TriggerMode::Set => active,
            }
        }
        None if mode == TriggerMode::Toggle => mask,
        None => mask || was,
    }
}

/// Flag 0x100: a hero's floor (floor_name2) is the target or one of its
/// direct children (fn_8005D730 case 5 walks child_index, then next_index).
pub fn on_target(
    objs: &[gdl_formats::worlds::WorldObject],
    root: usize,
    floor: Option<i32>,
) -> bool {
    let Some(f) = floor.filter(|&f| f >= 0).map(|f| f as usize) else {
        return false;
    };
    if f == root {
        return true;
    }
    let mut c = objs.get(root).map_or(-1, |o| o.child_index);
    for _ in 0..objs.len() {
        if c < 0 {
            break;
        }
        if c as usize == f {
            return true;
        }
        c = objs.get(c as usize).map_or(-1, |o| o.next_index);
    }
    false
}

/// TriggerCameraActivate / TriggerCamUpdate (boss/bosscam.c:348, :265): a
/// fixed camera from a type-9 locator. The controls go (and monsters hold)
/// at once; the view switches after `delay` ticks and is letterboxed.
#[derive(Clone, Copy, Debug)]
pub struct TriggerCamera {
    pub eye: Vec3,
    pub dir: Vec3,
    /// lbl_803443B8: ticks before the view switches.
    pub delay: i32,
    /// lbl_803443BC: ticks the view shows (duration x 6; 40 for 0).
    pub main: i32,
    /// lbl_803443C0: further ticks once `main` is out.
    pub hold: i32,
    /// lbl_803443C4: the trigger's target (a mover); the view stays until
    /// it stops moving (WorldObj flag 0x8000000).
    pub target: Option<usize>,
}

impl TriggerCamera {
    pub fn new(eye: Vec3, dir: Vec3, duration: i32, delay: i32, hold: i32) -> Self {
        TriggerCamera {
            eye,
            dir,
            delay,
            main: match duration {
                d if d < 0 => 1_000_000,
                0 => 40,
                d => d * 6,
            },
            hold,
            target: None,
        }
    }

    /// The view has switched (TriggerCamUpdate is past its delay).
    pub fn showing(&self) -> bool {
        self.delay <= 0
    }

    /// One TriggerCamUpdate of `ticks`; false once it has ended.
    pub fn tick(&mut self, ticks: i32, target_moving: bool) -> bool {
        if self.delay > 0 {
            self.delay -= ticks;
            return true;
        }
        self.main -= ticks;
        if self.main <= 0 {
            if self.hold > 0 {
                self.hold -= ticks;
            } else if !target_moving {
                return false;
            }
        }
        true
    }
}

/// The camera for trigger id `id` (LinkTriggerToCam: the last type-9 locator
/// whose index is `id`, its subtype the duration): eye, view direction and
/// duration. The view is Yaw(pi - (yaw + pi)) then Pitch(pitch), as the
/// tower's (tower::locator_cam).
pub fn trigger_camera(worlds: &gdl_formats::worlds::Worlds, id: u8) -> Option<(Vec3, Vec3, u8)> {
    use gdl_formats::worlds::LocatorType;
    let l = worlds
        .locators
        .iter()
        .rev()
        .find(|l| l.kind == LocatorType::TriggerCamera && l.next == id as u16)?;
    let (pitch, yaw) = (l.rot[0], l.rot[1]);
    let dir = Vec3::new(
        yaw.sin() * pitch.cos(),
        -pitch.sin(),
        yaw.cos() * pitch.cos(),
    );
    Some((Vec3::from(l.pos), dir, l.delay))
}

/// How far an ambush looks for a statue (sItemSearchDistance; fn_80062FF0
/// measures to the statue's edge).
pub const AMBUSH_RANGE: f32 = 10.0;

/// A placed golem, gargoyle or Death waiting as its statue (SetItem case 4:
/// GOL_STATUE, GAR_STATUE, DEATHSTATUE1/2). fn_80060114 makes the monster
/// only once enemydata.flags bit 1 is set: a hero bumps it (fn_8005D730
/// case 4), it is struck (item damage case 4) or an ambush trigger finds it.
/// Then, once in view, the statue plays its ACTIVE sequence (daction 1) and
/// the monster takes its place when that is done.
#[derive(Clone, Copy, Debug)]
pub struct Statue {
    /// The placement (the item's collision cylinder).
    pub pos: Vec3,
    pub radius: f32,
    pub height: f32,
    /// enemydata.rad >= 0: bumping it wakes it.
    pub touch: bool,
    /// Instance flag 1 (item active 0x40): an ambush finds it off screen.
    pub always: bool,
    /// enemydata.flags bit 1.
    pub woken: bool,
    /// Item active bit 1: its ACTIVE sequence has started.
    pub rising: bool,
    /// The statue's model in the world (an entity), if it has one.
    pub prop: Option<u64>,
}

/// SetItem case 4's statue for a placement and whether it starts woken:
/// golems and gargoyles, and Death, strength 0-1 as DEATHSTATUE1 and 2-3 as
/// DEATHSTATUE2 (1 and 3 set flags bit 1 at once).
pub fn statue_model(etype: usize, strength: i16) -> Option<(&'static str, bool)> {
    match (etype, strength) {
        (29, _) => Some(("GOL_STATUE", false)),
        (32, _) => Some(("GAR_STATUE", false)),
        (30, 0) => Some(("DEATHSTATUE1", false)),
        (30, 1) => Some(("DEATHSTATUE1", true)),
        (30, 2) => Some(("DEATHSTATUE2", false)),
        (30, 3) => Some(("DEATHSTATUE2", true)),
        _ => None,
    }
}

/// In the tower (sMusicTrackHi 13) LinkTriggerToCam keeps these locator
/// indices for Sumner's, the crystal and the window cameras instead.
pub fn tower_camera_index(id: u8) -> bool {
    id > 200 || id == 198 || (170..184).contains(&id)
}

/// A world object (with its children) moved by triggers.
pub struct Mover {
    pub root: usize,
    pub objects: Vec<usize>,
    /// Collision triangles that move with it, and their resting vertices.
    tris: Vec<(usize, [Vec3; 3])>,
    /// All its (and its children's) collision triangles: a fading target's
    /// nocol switches them off while it is faded out (fn_80062A00), animated
    /// or not.
    owned: Vec<usize>,
    /// Current vertical offset from the modelled position.
    pub offset: f32,
    off: f32,
    on: f32,
    /// Triggered (triggerstate bit 0x20).
    pub active: bool,
    /// A player is holding a trigger for it this frame (triggerstate low bits).
    pub held: bool,
    /// Target flag byte from the trigger subtype.
    flags8: u8,
    pub moving: bool,
    /// Drawn (fading objects disappear when not triggered).
    pub shown: bool,
    /// Where it is, for sounds.
    pub pos: Vec3,
    /// The target has a world animation (flag 0x2000000): triggers steer the
    /// animation instead of sliding it.
    pub animated: bool,
    /// The target also turns or animates (it is in `WorldAnims::dynamic`):
    /// the slide moves its node (mat[3][1]) and the collision follows that.
    node: bool,
}

impl Mover {
    pub fn new(
        level: &Level,
        collision: &Collision,
        root: usize,
        flags8: u8,
        off: f32,
        on: f32,
    ) -> Mover {
        let objects = level.subtree(root);
        let tris: Vec<(usize, [Vec3; 3])> = collision
            .tris
            .iter()
            .enumerate()
            .filter(|(_, t)| t.owner >= 0 && objects.contains(&(t.owner as usize)))
            .map(|(i, t)| (i, t.v))
            .collect();
        let owned = tris.iter().map(|&(i, _)| i).collect();
        Mover {
            root,
            objects,
            tris,
            owned,
            offset: off,
            off,
            on,
            active: false,
            held: false,
            flags8,
            moving: false,
            // A fade target with flag 0x20 starts shown (fn_80062A00's
            // `on` is st & 0x2F); one without it starts hidden.
            shown: flags8 & 0x10 == 0 || flags8 & 0x20 != 0,
            pos: Vec3::from(level.world_pos[root]),
            animated: false,
            node: false,
        }
    }

    /// A slider whose target also turns (a rotator's elevator): slide its
    /// node instead of its triangles.
    pub fn on_node(&mut self, wa: &mut WorldAnims) {
        self.node = true;
        self.tris.clear();
        wa.slide(self.root, self.offset);
    }

    /// A target with a world animation: SetItem case 5 sets it to play
    /// backwards (0x100000) at rest; its collision moves with the animation.
    pub fn animate(&mut self, wa: &mut WorldAnims) {
        self.animated = true;
        self.tris.clear();
        self.offset = 0.0;
        if let Some(f) = wa.flags.get_mut(self.root) {
            *f |= wflags::REVERSE;
        }
    }

    /// Fades in and out instead of sliding (target flag 0x10).
    pub fn fades(&self) -> bool {
        self.flags8 & 0x10 != 0
    }

    /// fn_80062A00's fade branch: `on` (faded out, nocol) is st & 0x2F with
    /// target flag 0x20 (triggered or held: the tower's crystal barriers
    /// vanish once opened), else !(st & 0xF) (shown while held).
    fn fade_shown(&self) -> bool {
        if self.flags8 & 0x20 != 0 {
            !(self.active || self.held)
        } else {
            self.active || self.held
        }
    }

    /// Momentary targets forget their state every frame unless a trigger holds them.
    fn momentary(&self) -> bool {
        self.flags8 & 0x47 == 0
    }

    /// Apply a trigger touched (or not) this frame. Returns true if it changed the target.
    pub fn trigger(&mut self, mode: TriggerMode, pressed: bool, idle: &mut f32) -> bool {
        let was = self.active;
        if pressed {
            self.held = true;
        }
        match mode {
            TriggerMode::Release => {
                if pressed {
                    self.active = false;
                }
            }
            TriggerMode::Latch | TriggerMode::Set => {
                if pressed {
                    self.active = true;
                }
            }
            TriggerMode::Toggle => {
                if pressed {
                    if self.moving {
                        *idle = TOGGLE_IDLE;
                    } else if *idle <= 0.0 {
                        self.active = !self.active;
                        *idle = TOGGLE_IDLE;
                    }
                }
            }
        }
        self.active != was
    }

    /// Slide toward the active/inactive height and move the collision with it.
    /// Returns (started, stopped) moving this frame.
    /// `ridden`: a hero stands on the target (did_generate >= 2), which holds
    /// targets without flag 8 still.
    pub fn update(
        &mut self,
        collision: &mut Collision,
        wa: &mut WorldAnims,
        ridden: bool,
        dt: f32,
    ) -> (bool, bool) {
        let was_moving = self.moving;
        if self.animated && !self.fades() {
            self.update_anim(wa, ridden);
            self.held = false;
            return (self.moving && !was_moving, was_moving && !self.moving);
        }
        if !self.fades() && self.flags8 & 8 == 0 && ridden {
            self.moving = false;
            self.held = false;
            return (false, was_moving);
        }
        if self.fades() {
            self.shown = self.fade_shown();
            for &i in &self.owned {
                collision.enabled[i] = self.shown;
            }
            self.moving = false;
        } else {
            let target = if self.active { self.on } else { self.off };
            let delta = target - self.offset;
            let step = SLIDE_SPEED * dt;
            self.moving = delta.abs() > 0.001;
            if self.moving {
                self.offset += delta.clamp(-step, step);
                if self.node {
                    wa.slide(self.root, self.offset);
                }
                for &(i, rest) in &self.tris {
                    collision.tris[i].v = rest.map(|v| v + Vec3::Y * self.offset);
                }
            } else if self.flags8 & 0x20 != 0 {
                // Ping-pong targets turn around at each end.
                self.active = !self.active;
            }
        }
        if self.momentary() {
            self.active = false;
        }
        self.held = false;
        if let Some(f) = wa.flags.get_mut(self.root) {
            if self.moving {
                *f |= wflags::ANIMATING;
            } else {
                *f &= !wflags::ANIMATING;
            }
        }
        (self.moving && !was_moving, was_moving && !self.moving)
    }

    /// fn_80062A00's 0x2000000 branch: point the target's animation forward
    /// (0x200000) while triggered and backward (0x100000) otherwise; a
    /// ping-pong target (0x20) loops while held and stops (both) once it
    /// comes round. It acts until the animation reaches that end.
    fn update_anim(&mut self, wa: &mut WorldAnims, ridden: bool) {
        let Some(f) = wa.flags.get_mut(self.root) else {
            return;
        };
        if self.flags8 & 8 == 0 && ridden {
            *f |= wflags::REVERSE | wflags::FORWARD;
            self.moving = false;
            return;
        }
        let mut act = true;
        if self.flags8 & 0x20 != 0 {
            if self.held {
                *f &= !(wflags::REVERSE | wflags::FORWARD);
            } else if *f & wflags::AT_END != 0 {
                *f |= wflags::REVERSE | wflags::FORWARD;
                act = false;
            }
        } else {
            if self.active {
                *f = (*f | wflags::FORWARD) & !wflags::REVERSE;
            } else {
                *f = (*f | wflags::REVERSE) & !wflags::FORWARD;
            }
            if (*f & wflags::REVERSE != 0 && *f & wflags::AT_START != 0)
                || (*f & wflags::FORWARD != 0 && *f & wflags::AT_END != 0)
            {
                act = false;
            }
        }
        if act {
            *f |= wflags::ACTING;
        } else {
            *f &= !wflags::ACTING;
        }
        self.moving = act;
        if self.momentary() {
            self.active = false;
        }
    }

    /// Put the collision at the starting offset.
    pub fn place(&self, collision: &mut Collision) {
        for &(i, rest) in &self.tris {
            collision.tris[i].v = rest.map(|v| v + Vec3::Y * self.offset);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Game, Kind};
    use crate::input::PlayerInput;

    fn data() -> Option<crate::fs::GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        crate::fs::GameFs::open(&root).ok()
    }

    fn tick(g: &mut Game, n: usize) {
        let idle = vec![PlayerInput::default(); g.players.len()];
        for _ in 0..n {
            g.update(1.0 / 60.0, &idle);
        }
    }

    /// The first trigger matching `f` (flags, id): its entity index and position.
    fn find_trigger(g: &Game, f: impl Fn(u16, u8) -> bool) -> (usize, Vec3) {
        let i = g
            .entities
            .iter()
            .position(|e| matches!(e.kind, Kind::Trigger { flags, id, .. } if f(flags, id)))
            .expect("trigger");
        (i, g.entities[i].pos)
    }

    /// A critter statue not yet woken.
    fn asleep(g: &Game, i: usize) -> bool {
        matches!(&g.entities[i].kind, Kind::Critter(c) if c.statue.is_some_and(|s| !s.woken))
    }

    #[test]
    fn daction_follows_mode_and_target() {
        use TriggerMode::*;
        // Held: always set off.
        for m in [Release, Latch, Toggle, Set] {
            assert!(daction(m, true, Some(false), false));
        }
        // Released: a release lever shows on while its target is down.
        assert!(daction(Release, false, Some(false), false));
        assert!(!daction(Release, false, Some(true), false));
        assert!(daction(Latch, false, Some(true), false));
        assert!(!daction(Toggle, false, Some(true), true));
        // Without a target a pad stays set off once it has been (a toggle not).
        assert!(daction(Set, false, None, true));
        assert!(!daction(Toggle, false, None, true));
    }

    #[test]
    fn trigger_camera_times_out_after_its_target_stops() {
        let mut c = TriggerCamera::new(Vec3::ZERO, Vec3::Z, 20, 0x1E, 0);
        assert_eq!(c.main, 120);
        // 30 ticks before the view switches, then 120 more.
        for _ in 0..30 {
            assert!(!c.showing());
            assert!(c.tick(1, false));
        }
        assert!(c.showing());
        for _ in 0..119 {
            assert!(c.tick(1, false));
        }
        // Out of time, but the target still moves.
        assert!(c.tick(1, true));
        assert!(!c.tick(1, false));
        assert_eq!(TriggerCamera::new(Vec3::ZERO, Vec3::Z, 0, 0, 0).main, 40);
    }

    #[test]
    fn subtype_bits_keep_the_high_flags() {
        assert_eq!(trigger_bits(25, 0x2000), 0x2804);
        assert_eq!(trigger_bits(24, 0x1002), 0x100A);
        assert_eq!(trigger_flags(24, 0x1002).0, TriggerMode::Latch);
    }

    /// levelA1's levers start their locator cameras: the controls go at
    /// once, the view switches after 30 ticks (letterboxed) and comes back
    /// once the target has stopped.
    #[test]
    fn lever_runs_its_trigger_camera() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let (_, pos) = find_trigger(&g, |_, id| id == 1);
        let (eye, _, dur) = trigger_camera(&g.level.worlds, 1).expect("camera 1");
        assert_eq!(dur, 20);
        g.players[0].pos = pos + Vec3::new(0.0, 0.5, 1.2);
        tick(&mut g, 2);
        let c = g.trigger_cam.expect("camera started");
        assert!(!c.showing());
        // Pushing the stick does nothing while it runs.
        let start = g.players[0].pos;
        let walk = PlayerInput {
            stick: glam::Vec2::new(0.0, 1.0),
            ..Default::default()
        };
        for _ in 0..40 {
            g.update(1.0 / 60.0, &[walk]);
        }
        let moved = g.players[0].pos - start;
        assert!(moved.x.hypot(moved.z) < 0.5, "controls held");
        assert_eq!(g.camera_view().0, eye);
        assert!(g.trigger_camera_view().is_some());
        tick(&mut g, 600);
        assert!(g.trigger_cam.is_none(), "camera ended");
        assert_ne!(g.camera_view().0, eye);
    }

    /// Flag 0x400 (levelA5's lift): one hero of two is told to bring the
    /// other (message 127) and nothing moves; both together set it off.
    #[test]
    fn all_heroes_pad_waits_for_everyone() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA5", &[("WAR", "RED"), ("VAL", "BLU")]).unwrap();
        let (i, pos) = find_trigger(&g, |f, _| f & tflags::ALL_HEROES != 0);
        let Kind::Trigger { mover: Some(m), .. } = g.entities[i].kind else {
            panic!("pad has a target");
        };
        g.players[0].pos = pos + Vec3::Y * 0.2;
        g.players[1].pos = pos + Vec3::new(30.0, 0.2, 0.0);
        g.msgs.shown = None;
        tick(&mut g, 10);
        assert!(!g.movers[m].active, "one hero is not enough");
        assert_eq!(g.msgs.shown.as_ref().map(|s| s.index), Some(127));
        g.players[1].pos = pos + Vec3::new(0.5, 0.2, 0.0);
        tick(&mut g, 2);
        assert!(g.movers[m].active, "both heroes set it off");
    }

    /// Item update case 5: a latching door switch (levelA1) posts the traps
    /// hint, message 5.
    #[test]
    fn door_switch_posts_the_traps_hint() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let mut seen = false;
        for _ in 0..900 {
            tick(&mut g, 1);
            seen |= g.msgs.shown.as_ref().is_some_and(|s| s.index == 5);
        }
        assert!(seen);
    }

    /// Flag 0x1000 (levelB5): setting the lever off shakes the camera.
    #[test]
    fn shake_flag_shakes_the_camera() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelB5", &[("WAR", "RED")]).unwrap();
        let (_, pos) = find_trigger(&g, |f, _| f & tflags::SHAKE != 0);
        assert!(!g.shake.shaking);
        g.players[0].pos = pos + Vec3::new(0.0, 0.5, 1.2);
        tick(&mut g, 2);
        assert!(g.shake.shaking);
    }

    /// levelC2's ambush pads sit on golem statues: the golem waits (no
    /// target) until the pad's ambush finds it; a far statue sleeps on.
    #[test]
    fn ambush_wakes_the_nearest_statue() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelC2", &[("WAR", "RED")]).unwrap();
        let (_, pos) = find_trigger(&g, |f, _| f & tflags::AMBUSH != 0);
        let statue_at = |g: &Game, near: bool| {
            g.entities
                .iter()
                .position(|e| {
                    let d = glam::Vec2::new(e.pos.x - pos.x, e.pos.z - pos.z).length();
                    matches!(&e.kind, Kind::Critter(c) if c.statue.is_some()) && (d < 2.0) == near
                })
                .expect("statue")
        };
        let golem = statue_at(&g, true);
        let far = statue_at(&g, false);
        tick(&mut g, 30);
        assert!(asleep(&g, golem), "waits while nobody comes");
        g.ambush(pos);
        assert!(!asleep(&g, golem), "the ambush woke it");
        assert!(asleep(&g, far));
    }

    /// Bumping a statue wakes it (fn_8005D730 case 4); passing by doesn't.
    #[test]
    fn bumping_a_statue_wakes_it() {
        let Some(fs) = data() else { return };
        let mut g = Game::new(&fs, "levelA1", &[("WAR", "RED")]).unwrap();
        let (i, s) = g
            .entities
            .iter()
            .enumerate()
            .find_map(|(i, e)| match &e.kind {
                Kind::Critter(c) => c.statue.map(|s| (i, s)),
                _ => None,
            })
            .expect("a golem statue");
        assert!(s.touch);
        let (at, id) = (g.entities[i].pos, g.entities[i].id);
        let find = |g: &Game| g.entities.iter().position(|e| e.id == id).unwrap();
        g.players[0].pos = at + Vec3::new(s.radius + 3.0, 0.0, 0.0);
        tick(&mut g, 5);
        assert!(asleep(&g, find(&g)));
        g.players[0].pos = at + Vec3::new(s.radius * 0.5, 0.0, 0.0);
        tick(&mut g, 1);
        assert!(!asleep(&g, find(&g)));
        // In view it rises (GOL_STATUE's ACTIVE), then the golem takes over.
        let prop = s.prop.expect("statue model");
        let mut rose = false;
        for _ in 0..600 {
            tick(&mut g, 1);
            rose |= g.entities.iter().any(|e| {
                e.id == prop
                    && e.model.and_then(|m| g.models[m].sequence("ACTIVE")) == Some(e.animator.seq)
            });
        }
        assert!(rose, "played ACTIVE");
        let golem = &g.entities[find(&g)];
        assert!(
            matches!(&golem.kind, Kind::Critter(c) if c.statue.is_none() && c.alpha == 1.0),
            "the golem is out"
        );
        assert!(
            g.entities.iter().all(|e| e.id != prop),
            "the statue is gone"
        );
    }
}
