//! Moving world machinery: world-object keyframe animations (gauntworld.c
//! `DoWorldAnimation`, world.c `DoWorldAnimSub`), rotator items (items.c
//! case 0xC, `YawMat3` on the target's node), and what rides on them.
//!
//! Every world object has a display node; an animated object's node matrix
//! is rebuilt from its keyframe track each frame, and its children (whose
//! positions stay parent-relative, NewWorldObject) follow. Collision
//! triangles of objects flagged 0x1000 / 0x1000000 are tested in the node's
//! frame (WorldObjCollide), so they move with it; heroes, monsters and
//! critters standing on a 0x1000 object are parented to its node
//! (PlayerCheckFloor / enemy / critter `MBNodeSetParent`) and are carried.

use crate::math::Det as _;
use crate::character::{pyr_matrix, ryp_matrix};
use crate::collision::Collision;
use crate::level::Level;
use gdl_formats::anim::Track;
use glam::{Mat4, Vec3};

/// WorldObj.flags bits used here.
pub mod wflags {
    /// Collision only follows the node's translation (with 0x1000).
    pub const TRANSLATE_ONLY: u32 = 0x1;
    /// The object has its own node matrix (collision in its frame; riders parent to it).
    pub const NODE: u32 = 0x1000;
    /// A child of a dynamic object: collision in its world matrix.
    pub const DYN_CHILD: u32 = 0x0100_0000;
    /// Has a world animation, or is the child of one (InitWorldInfo / NewWorldObject).
    pub const HAS_ANIM: u32 = 0x0200_0000;
    /// The animation is running (DoWorldAnimSub).
    pub const ANIMATING: u32 = 0x0800_0000;
    /// Play backwards, one shot (a trigger target at rest).
    pub const REVERSE: u32 = 0x0010_0000;
    /// Play forwards, one shot (a set-off trigger target). Both: stopped.
    pub const FORWARD: u32 = 0x0020_0000;
    /// Reached the first / last frame this step.
    pub const AT_START: u32 = 0x0040_0000;
    pub const AT_END: u32 = 0x0080_0000;
    /// Hidden by WorldObjectExplode until it comes round again.
    pub const EXPLODED: u32 = 0x1000_0000;
    /// A trigger target is acting (fn_80062A00).
    pub const ACTING: u32 = 0x2000_0000;
}
use wflags::*;

/// One `struct worldanim` and its run state.
#[derive(Clone)]
struct Anim {
    obj: usize,
    nframes: i16,
    /// Direction / mode bits: 1 on, 2 reverse, 0x100 loop, 0x200 ping-pong.
    state: u16,
    curframe: f32,
    track: Option<Track>,
}

/// A rotator item (items.c rotdata): turns its target's node about Y.
#[derive(Clone)]
struct Rotator {
    target: usize,
    /// Radians per 60 Hz tick.
    speed: f32,
    /// Subtype 2: the angle it stops at, and how far it has turned.
    delta: f32,
    curang: f32,
    sub: i32,
    /// Subtype 2: set off by a hero (item active & 1), done (active & 0x1000).
    on: bool,
    done: bool,
    /// Its turning sound is going.
    playing: bool,
    /// The rotator item's position, for touching it.
    pos: Vec3,
    radius: f32,
}

#[derive(Clone)]
pub struct WorldAnims {
    /// Live WorldObj.flags.
    pub flags: Vec<u32>,
    parent: Vec<Option<usize>>,
    /// Position relative to the parent (WorldObj.pos for dynamic objects).
    pos: Vec<Vec3>,
    /// Absolute modelled position: where the mesh and collision were built.
    rest: Vec<Vec3>,
    /// Node matrices: translation, rotation and scale relative to the parent.
    node: Vec<Mat4>,
    /// World matrices this frame and last.
    pub world: Vec<Mat4>,
    pub prev: Vec<Mat4>,
    anims: Vec<Anim>,
    rotators: Vec<Rotator>,
    /// Objects that can move (animated or rotated, their descendants, and the
    /// ancestors an explosion hides), parents before children. The position
    /// in this list is the object's part in the scene's dynamic mesh.
    pub dynamic: Vec<usize>,
    /// Collision triangles that follow their owner: (triangle, owner, rest
    /// vertices, rest normal).
    tris: Vec<(usize, usize, [Vec3; 3], Vec3)>,
    /// Hidden nodes (MBTreeSetFlags 2).
    pub hidden: Vec<bool>,
    /// WorldObjectExplode positions this frame, for the game to set off.
    pub explosions: Vec<Vec3>,
    /// Each hero's floor object at the start of the frame (floor_name2).
    pub floors: Vec<Option<i32>>,
}

impl WorldAnims {
    /// InitWorldInfo's world animations (state 0x101, flag 0x2000000 on each
    /// animated object, inherited by its children) and the rotators placed
    /// in the level.
    pub fn new(level: &Level, collision: &mut Collision) -> WorldAnims {
        let w = &level.worlds;
        let n = w.world_objects.len();
        let mut parent = vec![None; n];
        for (i, o) in w.world_objects.iter().enumerate() {
            let mut c = o.child_index;
            let mut guard = 0;
            while c >= 0 && (c as usize) < n && guard < n {
                if parent[c as usize].is_none() && c as usize != i {
                    parent[c as usize] = Some(i);
                }
                c = w.world_objects[c as usize].next_index;
                guard += 1;
            }
        }
        let mut flags: Vec<u32> = w.world_objects.iter().map(|o| o.flags).collect();
        let anims: Vec<Anim> = w
            .world_anims
            .iter()
            .map(|a| Anim {
                obj: a.obj,
                nframes: a.nframes,
                state: 0x101,
                curframe: a.curframe,
                track: a.track.clone(),
            })
            .collect();
        for a in &anims {
            flags[a.obj] |= HAS_ANIM;
        }
        let pos: Vec<Vec3> = w.world_objects.iter().map(|o| Vec3::from(o.pos)).collect();
        let rest: Vec<Vec3> = level.world_pos.iter().map(|p| Vec3::from(*p)).collect();
        let mut rotators = Vec::new();
        for inst in &w.item_instances {
            let Some(info) = w.item_infos.get(inst.item_index.max(0) as usize) else {
                continue;
            };
            if info.item_type != gdl_formats::worlds::ItemType::Rotator {
                continue;
            }
            let p = &inst.params;
            let target = i32::from_le_bytes(p[0..4].try_into().unwrap());
            if target < 0 || target as usize >= n {
                continue;
            }
            rotators.push(Rotator {
                target: target as usize,
                speed: f32::from_le_bytes(p[4..8].try_into().unwrap()),
                delta: f32::from_le_bytes(p[8..12].try_into().unwrap()),
                curang: 0.0,
                sub: info.subtype,
                on: false,
                done: false,
                playing: false,
                pos: Vec3::from(inst.pos),
                radius: info.radius.max(0.5),
            });
        }
        // NewWorldObject: children of animated objects are animated too.
        let mut order = Vec::with_capacity(n);
        let mut seen = vec![false; n];
        fn visit(i: usize, parent: &[Option<usize>], seen: &mut [bool], order: &mut Vec<usize>) {
            if seen[i] {
                return;
            }
            seen[i] = true;
            if let Some(p) = parent[i] {
                visit(p, parent, seen, order);
            }
            order.push(i);
        }
        for i in 0..n {
            visit(i, &parent, &mut seen, &mut order);
        }
        for &i in &order {
            if let Some(p) = parent[i] {
                if flags[p] & HAS_ANIM != 0 {
                    flags[i] |= HAS_ANIM;
                }
            }
        }
        // What can move: the animated, the rotated and their subtrees, and
        // the ancestors of anything that explodes (it hides them).
        let mut moving = vec![false; n];
        for a in &anims {
            moving[a.obj] = true;
            if flags[a.obj] & 0xF_0000 == 0x5_0000 {
                let mut p = parent[a.obj];
                while let Some(q) = p {
                    moving[q] = true;
                    p = parent[q];
                }
            }
        }
        for r in &rotators {
            moving[r.target] = true;
        }
        for &i in &order {
            if let Some(p) = parent[i] {
                if moving[p] {
                    moving[i] = true;
                }
            }
        }
        let dynamic: Vec<usize> = order.iter().copied().filter(|&i| moving[i]).collect();
        // Their collision: tested in the node frame for 0x1000 / 0x1000000.
        let mut tris = Vec::new();
        let mut groups = Vec::new();
        for &o in &dynamic {
            if flags[o] & (NODE | DYN_CHILD) == 0 {
                continue;
            }
            let mut g = Vec::new();
            for (ti, t) in collision.tris.iter().enumerate() {
                if t.owner == o as i32 {
                    tris.push((ti, o, t.v, t.n));
                    g.push(ti);
                }
            }
            groups.push(g);
        }
        collision.make_dynamic(groups);
        let node: Vec<Mat4> = pos.iter().map(|p| Mat4::from_translation(*p)).collect();
        let world: Vec<Mat4> = rest.iter().map(|p| Mat4::from_translation(*p)).collect();
        WorldAnims {
            flags,
            parent,
            pos,
            rest,
            node,
            prev: world.clone(),
            world,
            anims,
            rotators,
            dynamic,
            tris,
            hidden: vec![false; n],
            explosions: Vec::new(),
            floors: Vec::new(),
        }
    }

    /// No world objects at all (tests and empty levels).
    pub fn empty() -> WorldAnims {
        WorldAnims {
            flags: Vec::new(),
            parent: Vec::new(),
            pos: Vec::new(),
            rest: Vec::new(),
            node: Vec::new(),
            world: Vec::new(),
            prev: Vec::new(),
            anims: Vec::new(),
            rotators: Vec::new(),
            dynamic: Vec::new(),
            tris: Vec::new(),
            hidden: Vec::new(),
            explosions: Vec::new(),
            floors: Vec::new(),
        }
    }

    /// A debug save state's view of the world machinery (savestate.rs):
    /// each animation's object, state and frame, and each rotator's target,
    /// turn so far, set-off / done and node matrix.
    pub fn save_state(&self) -> (Vec<(usize, u16, f32)>, Vec<(usize, f32, bool, bool, Mat4)>) {
        let anims = self
            .anims
            .iter()
            .map(|a| (a.obj, a.state, a.curframe))
            .collect();
        let rots = self
            .rotators
            .iter()
            .map(|r| (r.target, r.curang, r.on, r.done, self.node[r.target]))
            .collect();
        (anims, rots)
    }

    /// Put an animation (by its object) back at a saved state and frame.
    pub fn restore_anim(&mut self, obj: usize, state: u16, curframe: f32) {
        if let Some(a) = self.anims.iter_mut().find(|a| a.obj == obj) {
            a.state = state;
            a.curframe = curframe.clamp(0.0, (a.nframes.max(1) - 1) as f32);
        }
    }

    /// Put a rotator (by its target) back at a saved turn and node matrix.
    pub fn restore_rotator(
        &mut self,
        target: usize,
        curang: f32,
        on: bool,
        done: bool,
        node: Mat4,
    ) {
        if let Some(r) = self.rotators.iter_mut().find(|r| r.target == target) {
            r.curang = curang;
            r.on = on;
            r.done = done;
            if let Some(n) = self.node.get_mut(target) {
                *n = node;
            }
        }
    }

    /// Whether world object `o` has an animation (FindWobjWanim).
    pub fn has_anim(&self, o: usize) -> bool {
        self.anims.iter().any(|a| a.obj == o)
    }

    /// ActivateSpecialTrigger with `flag`, items.c SetItem's tower case:
    /// jump a trigger target's animation to its last frame, pointed forward
    /// so it holds there.
    pub fn jump_to_end(&mut self, o: usize) {
        if let Some(a) = self.anims.iter_mut().find(|a| a.obj == o) {
            a.curframe = (a.nframes - 1) as f32;
        }
        if let Some(f) = self.flags.get_mut(o) {
            *f = (*f | FORWARD | AT_END) & !REVERSE;
        }
    }

    /// fn_80062A00's slide: a trigger target's node sits `y` above its place.
    pub fn slide(&mut self, o: usize, y: f32) {
        if let (Some(n), Some(p)) = (self.node.get_mut(o), self.pos.get(o)) {
            n.w_axis.y = p.y + y;
        }
    }

    /// WorldObjGetAllFlags & ANIMATING: the object or one of its ancestors is
    /// animating (or, for trigger sliders, moving).
    pub fn animating(&self, o: usize) -> bool {
        let mut i = Some(o);
        while let Some(j) = i {
            if self.flags.get(j).is_some_and(|f| f & ANIMATING != 0) {
                return true;
            }
            i = self.parent.get(j).copied().flatten();
        }
        false
    }

    /// The delta from an object's modelled placement to where it is now: the
    /// transform of its mesh part and collision.
    pub fn delta(&self, o: usize) -> Mat4 {
        self.world[o] * Mat4::from_translation(-self.rest[o])
    }

    /// For the level runner (read only): the subtype-2 rotators a touch
    /// still sets off: (place, touch radius, target object).
    pub fn untouched_rotators(&self) -> Vec<(Vec3, f32, usize)> {
        self.rotators
            .iter()
            .filter(|r| r.sub == 2 && !r.on)
            .map(|r| (r.pos, r.radius, r.target))
            .collect()
    }

    /// RotatorTouched (fn_8005D730 case 12): a hero touching a subtype-2
    /// rotator sets it off.
    pub fn touch_rotators(&mut self, at: Vec3, radius: f32) {
        for r in &mut self.rotators {
            if r.sub == 2 && !r.on {
                let d = glam::Vec2::new(at.x - r.pos.x, at.z - r.pos.z).length();
                if d < r.radius + radius && (at.y - r.pos.y).abs() < 3.0 {
                    r.on = true;
                }
            }
        }
    }

    /// One frame: DoWorldAnimation's tracks, then the rotators, then the
    /// world matrices and the collision that follows them. Returns the
    /// rotators' sound events (0 turning, 2 stopped) and where.
    pub fn update(&mut self, collision: &mut Collision, dt: f32) -> Vec<(i32, Vec3)> {
        self.explosions.clear();
        let mut sounds = Vec::new();
        for ai in 0..self.anims.len() {
            self.anim_sub(ai, dt);
        }
        // items.c case 0xC: subtype 0 turns for ever, subtype 2 once set off
        // until it has turned `delta`.
        let ticks = dt * 60.0;
        for ri in 0..self.rotators.len() {
            let r = &mut self.rotators[ri];
            let t = r.target;
            let ang = r.speed * ticks;
            let turn = match r.sub {
                0 => true,
                2 if r.on && !r.done => {
                    let at = self.world[t].w_axis.truncate();
                    if (r.speed >= 0.0 && r.curang >= r.delta)
                        || (r.speed < 0.0 && r.curang <= -r.delta)
                    {
                        sounds.push((2, at));
                        r.done = true;
                        false
                    } else {
                        if !r.playing {
                            sounds.push((0, at));
                            r.playing = true;
                        }
                        r.curang += ang;
                        true
                    }
                }
                _ => false,
            };
            if turn {
                self.flags[t] &= !TRANSLATE_ONLY;
                if ang.abs() >= 1e-6 {
                    self.node[t] *= Mat4::from_rotation_y(ang);
                }
            }
        }
        self.prev.clone_from(&self.world);
        for k in 0..self.dynamic.len() {
            let o = self.dynamic[k];
            let pw = match self.parent[o] {
                Some(p) => self.world[p],
                None => Mat4::IDENTITY,
            };
            self.world[o] = pw * self.node[o];
        }
        for (ti, o, rest, n) in &self.tris {
            let d = self.world[*o] * Mat4::from_translation(-self.rest[*o]);
            let f = self.flags[*o];
            let d = if f & DYN_CHILD == 0 && f & TRANSLATE_ONLY != 0 {
                Mat4::from_translation(d.w_axis.truncate())
            } else {
                d
            };
            let Some(t) = collision.tris.get_mut(*ti) else {
                continue;
            };
            t.v = rest.map(|v| d.transform_point3(v));
            t.n = d.transform_vector3(*n).normalize_or_zero();
        }
        collision.refit_dynamic();
        sounds
    }

    /// DoWorldAnimSub: set the direction from the object's flags, pose the
    /// node at the current frame, then step the frame.
    fn anim_sub(&mut self, ai: usize, dt: f32) {
        let a = &mut self.anims[ai];
        let o = a.obj;
        let f = self.flags[o];
        let rev = f & REVERSE != 0;
        let fwd = f & FORWARD != 0;
        if rev && fwd {
            a.state &= !1;
        } else if rev {
            a.state = (a.state | 1 | 2) & !0x300;
        } else if fwd {
            a.state = (a.state | 1) & !0x300 & !2;
        } else {
            a.state = (a.state | 1 | 0x100) & !2;
        }
        if a.state & 1 == 0 {
            self.flags[o] &= !ANIMATING;
            return;
        }
        match &a.track {
            // No keyframes: the template pose.
            None => self.node[o] = Mat4::IDENTITY,
            Some(t) => {
                let mode = t.flags;
                let p = t.sample(a.curframe);
                let (s, r, tr) = self.node[o].to_scale_rotation_translation();
                let mut rot = Mat4::from_quat(r);
                let mut tr = tr;
                let mut scale = s;
                if mode & 7 != 0 {
                    rot = if mode & 0x8000 != 0 {
                        pyr_matrix(p.pyr)
                    } else {
                        ryp_matrix(p.pyr)
                    };
                }
                if mode & 0x70 != 0 {
                    tr = self.pos[o] + Vec3::from(p.xyz);
                }
                if mode & 0x700 != 0 {
                    scale = Vec3::from(p.scale);
                }
                self.node[o] = Mat4::from_translation(tr) * rot * Mat4::from_scale(scale);
            }
        }
        if dt <= 0.0 {
            return;
        }
        let mut f = self.flags[o];
        f &= !(AT_START | AT_END);
        f |= ANIMATING;
        let last = (a.nframes - 1) as f32;
        let mut explode = false;
        let mut unhide = false;
        if a.state & 2 != 0 {
            a.curframe -= 30.0 * dt;
            if a.curframe < 0.0 {
                if a.state & 0x200 != 0 {
                    a.curframe = 0.0;
                    a.state ^= 2;
                } else if a.state & 0x100 != 0 {
                    a.curframe = last;
                    explode = f & 0x100F_0000 == 0x5_0000;
                } else {
                    a.curframe = 0.0;
                    f &= !ANIMATING;
                }
                f |= AT_START;
            } else if (a.curframe as i16) >= a.nframes - 2 && f & EXPLODED != 0 {
                f &= !EXPLODED;
                unhide = true;
            }
        } else {
            a.curframe += 30.0 * dt;
            if (a.curframe as i16) >= a.nframes - 1 {
                if a.state & 0x200 != 0 {
                    a.curframe = last;
                    a.state ^= 2;
                } else if a.state & 0x100 != 0 {
                    a.curframe = 0.0;
                    explode = f & 0x100F_0000 == 0x5_0000;
                } else {
                    a.curframe = last;
                    f &= !ANIMATING;
                }
                f |= AT_END;
            } else if (a.curframe as i16) <= 1 && f & EXPLODED != 0 {
                f &= !EXPLODED;
                unhide = true;
            }
        }
        self.flags[o] = f;
        if unhide {
            self.set_hidden(o, false);
        }
        if explode {
            // WorldObjectExplode: the effect where it is, then it and its
            // ancestors' trees hide until each comes round again.
            let at = (self.parent[o].map_or(Mat4::IDENTITY, |p| self.world[p]) * self.node[o])
                .w_axis
                .truncate();
            self.explosions.push(at);
            let mut i = Some(o);
            while let Some(j) = i {
                self.set_hidden(j, true);
                self.flags[j] |= EXPLODED;
                i = self.parent[j];
            }
        }
    }

    /// MBTreeSetFlags / MBTreeClearFlags 2 over a node's subtree.
    fn set_hidden(&mut self, o: usize, on: bool) {
        self.hidden[o] = on;
        for k in 0..self.dynamic.len() {
            let c = self.dynamic[k];
            let mut p = self.parent[c];
            while let Some(q) = p {
                if q == o {
                    self.hidden[c] = on;
                    break;
                }
                p = self.parent[q];
            }
        }
    }

    /// A rider standing on `owner` (PlayerCheckFloor / MBNodeSetParent onto a
    /// 0x1000 object): where this frame's motion of the node carries a point,
    /// and how far it turns. None when the floor isn't a node or didn't move.
    pub fn carry(&self, owner: i32, at: Vec3) -> Option<(Vec3, f32)> {
        let o = usize::try_from(owner).ok()?;
        if self.flags.get(o)? & NODE == 0 || !self.dynamic.contains(&o) {
            return None;
        }
        let m = self.world[o] * self.prev[o].inverse();
        if m.abs_diff_eq(Mat4::IDENTITY, 1e-6) {
            return None;
        }
        let to = m.transform_point3(at);
        let f = m.transform_vector3(Vec3::Z);
        Some((to, f.x.det_atan2(f.z)))
    }
}

/// Sound ids by realm (sMusicTrackHi), sounds_evt.c: lbl_80123B1C / B54
/// (fn_8009D7E4, a rotator turning / stopping) and lbl_801240A4
/// (AudioWorldExplosion); each 14-entry table runs into the next.
const ROTATE_SFX: [i32; 16] = [
    -1, 0x260002, -1, -1, -1, -1, -1, -1, -1, 0x2B004C, -1, -1, -1, 0, -1, 0x260003,
];
const ROTATE_STOP_SFX: [i32; 16] = [
    -1, 0x260003, -1, -1, -1, -1, -1, -1, -1, 0x2B004D, -1, -1, -1, 0, -1, -1,
];
const EXPLOSION_SFX: [i32; 16] = [
    -1, -1, -1, -1, -1, -1, -1, -1, -1, 0x2B0047, -1, -1, -1, 0, -1, -1,
];

impl crate::game::Game {
    /// RegisterItemWobj: a trigger's target joins the movers. An animated
    /// target is steered through its animation; one that also turns slides
    /// its node.
    pub(crate) fn add_mover(&mut self, mut m: crate::triggers::Mover) -> usize {
        let t = m.root;
        if self
            .world_anim
            .flags
            .get(t)
            .is_some_and(|f| f & HAS_ANIM != 0)
        {
            m.animate(&mut self.world_anim);
        } else if self.world_anim.dynamic.contains(&t) {
            m.on_node(&mut self.world_anim);
        }
        m.place(&mut self.collision);
        self.movers.push(m);
        self.movers.len() - 1
    }

    /// AddItemSub (items.c), once every item is set and fn_80062A00 has put
    /// the trigger targets at their starting heights: an item without
    /// collision of its own (iteminfo colflags bit 0 clear) is put 0.1
    /// (sItemFloorYOffset) above the floor FloorPos finds under it (radius
    /// 1, from 4 above to 10 below), and is parented to that floor's node if
    /// the floor object moves (flag 0x1000): a pad on a lift that starts
    /// lowered sits on the lowered deck and rides with it, and the tower's
    /// Citadel portal rides the lift the 13th runestone raises. (The
    /// original does this for every item; triggers and exits are put down
    /// here.)
    pub(crate) fn put_items_on_floor(&mut self, ids: &[u64]) {
        // The world posed at its animations' first frames, as it is when
        // the original adds its items: a lift cabin whose animation starts
        // it away from where it was modelled has its pad put down in the
        // cabin, not on the water where the cabin was modelled.
        self.world_anim.update(&mut self.collision, 0.0);
        for e in self.entities.iter_mut().filter(|e| ids.contains(&e.id)) {
            let hit = self.collision.floor_collide(e.pos, 1.0, 4.0, -10.0, true);
            e.pos.y = hit.map_or(e.pos.y, |h| h.at.y) + 0.1;
            e.ride = hit
                .filter(|h| self.collision.obj_flags[h.tri] & NODE != 0)
                .map(|h| {
                    let off = Self::sliding(&self.movers, h.owner).map_or(0.0, |m| m.offset);
                    let world = usize::try_from(h.owner)
                        .ok()
                        .and_then(|o| self.world_anim.world.get(o))
                        .copied()
                        .unwrap_or(Mat4::IDENTITY);
                    crate::game::Ride {
                        obj: h.owner,
                        local: world.inverse().transform_point3(e.pos),
                        y: e.pos.y,
                        off,
                    }
                });
        }
    }

    /// The mover that slides world object `owner` (with its subtree).
    fn sliding(movers: &[crate::triggers::Mover], owner: i32) -> Option<&crate::triggers::Mover> {
        let o = usize::try_from(owner).ok()?;
        movers
            .iter()
            .find(|m| !m.animated && !m.fades() && m.objects.contains(&o))
    }

    /// Items parented to a slid floor (see put_items_on_floor) go up and
    /// down with it.
    pub(crate) fn slide_riding_items(&mut self) {
        for e in &mut self.entities {
            if let Some(r) = e.ride {
                if let Some(m) = Self::sliding(&self.movers, r.obj) {
                    e.pos.y = r.y + m.offset - r.off;
                }
            }
        }
    }

    /// Items parented to a floor whose node moves (an animated lift, a
    /// rotator's deck) sit where its node now puts them, turned as it
    /// turned this frame.
    pub(crate) fn pose_riding_items(&mut self) {
        for e in &mut self.entities {
            let Some(r) = e.ride else {
                continue;
            };
            if Self::sliding(&self.movers, r.obj).is_some() {
                continue;
            }
            let Some(world) = usize::try_from(r.obj)
                .ok()
                .and_then(|o| self.world_anim.world.get(o))
            else {
                continue;
            };
            if let Some((_, turn)) = self.world_anim.carry(r.obj, e.pos) {
                e.yaw += turn;
            }
            e.pos = world.transform_point3(r.local);
        }
    }

    /// The world's moving parts for one frame, before the heroes move:
    /// PlayerCheckMovingFloor notes what everyone stands on, DoWorldAnimation
    /// and the rotators move the world, and the riders go with their floors.
    pub(crate) fn update_world_motion(&mut self, dt: f32) {
        use crate::game::{Kind, PLAYER_COL_HEIGHT, PLAYER_RADIUS};
        let floor_of = |c: &Collision, at: Vec3| {
            c.ground_hit(at, at.y + 0.3)
                .filter(|(y, _)| at.y - y < 0.3)
                .map(|(_, o)| o)
        };
        let floors: Vec<Option<i32>> = self
            .players
            .iter()
            .map(|p| {
                // PlayerCheckMovingFloor: the floor found with the hero's
                // radius (PlayerCollideFloor's).
                (p.state.active() && p.on_ground)
                    .then(|| {
                        self.collision
                            .player_floor(p.pos, PLAYER_RADIUS, PLAYER_COL_HEIGHT, true)
                            .filter(|h| (p.pos.y - h.at.y).abs() < 0.3)
                            .map(|h| h.owner)
                    })
                    .flatten()
            })
            .collect();
        let riders: Vec<(usize, i32)> = self
            .entities
            .iter()
            .enumerate()
            .filter(|(_, e)| e.alive && matches!(e.kind, Kind::Enemy { .. } | Kind::Critter(_)))
            .filter_map(|(i, e)| Some((i, floor_of(&self.collision, e.pos)?)))
            .collect();
        for p in self.players.iter().filter(|p| p.state.active()) {
            self.world_anim.touch_rotators(p.pos, PLAYER_RADIUS);
        }
        let sounds = self.world_anim.update(&mut self.collision, dt);
        self.world_anim.floors = floors.clone();
        for (pi, f) in floors.iter().enumerate() {
            if let Some((to, turn)) = f.and_then(|o| self.world_anim.carry(o, self.players[pi].pos))
            {
                let p = &mut self.players[pi];
                p.pos = to;
                p.yaw += turn;
            }
        }
        for (ei, o) in riders {
            if let Some((to, turn)) = self.world_anim.carry(o, self.entities[ei].pos) {
                let e = &mut self.entities[ei];
                e.pos = to;
                e.yaw += turn;
            }
        }
        // Items parented to an animated floor (put_items_on_floor).
        self.pose_riding_items();
        for (kind, _) in sounds {
            let id = if kind == 0 {
                self.realm_sound(&ROTATE_SFX)
            } else {
                self.realm_sound(&ROTATE_STOP_SFX)
            };
            self.sfx_id(id);
        }
        // WorldExplosion: its expanding blast (effect.rs), and its sound.
        for at in std::mem::take(&mut self.world_anim.explosions) {
            let id = self.realm_sound(&EXPLOSION_SFX);
            self.sfx_id(id);
            self.world_explosion(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gdl_formats::anim::Track;

    /// An object at (10, 0, 0) with one floor triangle and a 60-frame track
    /// sliding it 6 units along +x.
    fn slider() -> (WorldAnims, Collision) {
        let tri = crate::collision::Tri {
            v: [
                Vec3::new(8.0, 0.0, -2.0),
                Vec3::new(12.0, 0.0, -2.0),
                Vec3::new(10.0, 0.0, 3.0),
            ],
            n: Vec3::Y,
            owner: 0,
        };
        let mut col = Collision::new(vec![tri]);
        col.make_dynamic(vec![vec![0]]);
        let track = Track {
            flags: 0x10,
            keys: vec![
                (0, [0.0; 9]),
                (59, {
                    let mut v = [0.0; 9];
                    v[3] = 6.0;
                    v
                }),
            ],
        };
        let wa = WorldAnims {
            flags: vec![NODE | HAS_ANIM],
            parent: vec![None],
            pos: vec![Vec3::new(10.0, 0.0, 0.0)],
            rest: vec![Vec3::new(10.0, 0.0, 0.0)],
            node: vec![Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))],
            world: vec![Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))],
            prev: vec![Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))],
            anims: vec![Anim {
                obj: 0,
                nframes: 60,
                state: 0x101,
                curframe: 0.0,
                track: Some(track),
            }],
            rotators: Vec::new(),
            dynamic: vec![0],
            tris: vec![(0, 0, col.tris[0].v, Vec3::Y)],
            hidden: vec![false],
            explosions: Vec::new(),
            floors: Vec::new(),
        };
        (wa, col)
    }

    #[test]
    fn animated_object_moves_its_collision() {
        let (mut wa, mut col) = slider();
        assert!(col.ground_height(Vec3::new(10.0, 1.0, 0.0), 2.0).is_some());
        // One second: 30 frames, halfway along.
        for _ in 0..30 {
            wa.update(&mut col, 1.0 / 30.0);
        }
        assert!(wa.flags[0] & ANIMATING != 0, "animating while it plays");
        let x = col.tris[0].v[2].x;
        assert!((x - 13.0).abs() < 0.2, "the triangle slid to {x}");
        // Found at its new place (off the static grid), gone from the old.
        assert!(col.ground_height(Vec3::new(13.5, 1.0, 0.0), 2.0).is_some());
        assert!(col.ground_height(Vec3::new(8.5, 1.0, -1.5), 2.0).is_none());
    }

    #[test]
    fn loops_and_one_shots() {
        let (mut wa, mut col) = slider();
        // Looping (no direction flags): wraps to frame 0 at the end.
        for _ in 0..61 {
            wa.update(&mut col, 1.0 / 30.0);
        }
        assert!(wa.anims[0].curframe < 3.0, "{}", wa.anims[0].curframe);
        // A trigger target set off (0x200000) plays once and stops at the end.
        wa.flags[0] |= FORWARD;
        for _ in 0..90 {
            wa.update(&mut col, 1.0 / 30.0);
        }
        assert_eq!(wa.anims[0].curframe, 59.0);
        assert!(wa.flags[0] & AT_END != 0);
        assert!(wa.flags[0] & ANIMATING == 0, "stopped");
        // Back at rest (0x100000) it plays backwards to frame 0.
        wa.flags[0] = (wa.flags[0] & !FORWARD) | REVERSE;
        for _ in 0..90 {
            wa.update(&mut col, 1.0 / 30.0);
        }
        assert_eq!(wa.anims[0].curframe, 0.0);
        assert!(wa.flags[0] & AT_START != 0);
        // Both: switched off.
        wa.flags[0] |= FORWARD;
        wa.update(&mut col, 1.0 / 30.0);
        assert!(wa.flags[0] & ANIMATING == 0);
    }

    #[test]
    fn a_rider_is_carried() {
        let (mut wa, mut col) = slider();
        let mut at = Vec3::new(10.0, 0.0, 0.0);
        for _ in 0..15 {
            let owner = col.ground_hit(at, at.y + 0.1).map(|(_, o)| o).unwrap();
            wa.update(&mut col, 1.0 / 30.0);
            if let Some((to, _)) = wa.carry(owner, at) {
                at = to;
            }
        }
        assert!((at.x - 11.5).abs() < 0.15, "carried to {at}");
    }

    fn data() -> Option<crate::fs::GameFs> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data");
        crate::fs::GameFs::open(&root).ok()
    }

    /// levelA2's turning hogs (yaw tracks) carry their collision round.
    #[test]
    fn level_animation_moves_collision() {
        let Some(fs) = data() else { return };
        let mut g = crate::game::Game::new(&fs, "levelA2", &[("WAR", "RED")]).unwrap();
        let o = g
            .level
            .worlds
            .world_objects
            .iter()
            .position(|w| w.name == "A2HOG04")
            .unwrap();
        assert!(g.world_anim.has_anim(o));
        assert!(g.world_anim.dynamic.contains(&o));
        let tri = g
            .collision
            .tris
            .iter()
            .position(|t| t.owner == o as i32)
            .unwrap();
        let v0 = g.collision.tris[tri].v;
        for _ in 0..60 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        let v1 = g.collision.tris[tri].v;
        let moved = (0..3).map(|k| (v1[k] - v0[k]).length()).fold(0.0, f32::max);
        assert!(moved > 0.1, "hog collision moved {moved}");
        assert!(g.world_anim.flags[o] & ANIMATING != 0);
        assert!(g.world_anim.animating(o));
    }

    /// levelI2's turning wheels (rotator items, subtype 0) turn their
    /// collision about their node.
    #[test]
    fn rotator_turns_its_target() {
        let Some(fs) = data() else { return };
        let mut g = crate::game::Game::new(&fs, "levelI2", &[("WAR", "RED")]).unwrap();
        let o = g
            .level
            .worlds
            .world_objects
            .iter()
            .position(|w| w.name == "I2XSELEV1")
            .unwrap();
        assert!(g.world_anim.dynamic.contains(&o));
        let tri = g
            .collision
            .tris
            .iter()
            .position(|t| t.owner == o as i32)
            .unwrap();
        let v0 = g.collision.tris[tri].v;
        for _ in 0..60 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        // 0.5 degrees a tick: 30 degrees in a second, about the node.
        let d = g.world_anim.delta(o);
        let f = d.transform_vector3(Vec3::Z);
        let turned = f.x.det_atan2(f.z).abs().to_degrees();
        assert!((turned - 30.0).abs() < 1.0, "turned {turned}");
        // Each vertex keeps its distance from the node (which its elevator
        // trigger may also have slid; the world is posed from load).
        let r0 = (v0[0] - g.world_anim.world[o].w_axis.truncate()).length();
        let v1 = g.collision.tris[tri].v;
        let r1 = (v1[0] - g.world_anim.world[o].w_axis.truncate()).length();
        assert!((r1 - r0).abs() < 1e-3, "{r0} {r1}");
        assert!((v1[0] - v0[0]).length() > 0.01, "collision turned");
    }

    /// A hero standing on a moving floor goes with it (PlayerCheckFloor).
    #[test]
    fn hero_rides_a_moving_floor() {
        let Some(fs) = data() else { return };
        let mut g = crate::game::Game::new(&fs, "levelG2", &[("WAR", "RED")]).unwrap();
        // G2's rail platform: an x/z/yaw track, its deck a floor.
        let o = g
            .level
            .worlds
            .world_objects
            .iter()
            .position(|w| w.name == "G2RAIL1")
            .unwrap();
        assert!(g.world_anim.flags[o] & NODE != 0);
        // Let it get going, then stand the hero on it: on the largest deck
        // triangle clear of the cart's walls (its handle bars would push a
        // hero standing under them, and the invisible G2BLOCKER1 box around
        // the cart faces outward, so it only keeps heroes from walking in).
        for _ in 0..10 {
            g.update(1.0 / 60.0, &[crate::input::PlayerInput::default()]);
        }
        let mid = |t: &crate::collision::Tri| (t.v[0] + t.v[1] + t.v[2]) / 3.0;
        let tri = g
            .collision
            .tris
            .iter()
            .enumerate()
            .filter(|(_, t)| t.owner == o as i32 && t.n.y > 0.95)
            .filter(|(_, t)| {
                let (_, wall) = g.collision.resolve_walls_hit(
                    mid(t),
                    crate::game::PLAYER_RADIUS,
                    crate::game::PLAYER_HEIGHT,
                    crate::game::STEP_HEIGHT,
                );
                wall.is_none()
            })
            .max_by(|(_, a), (_, b)| {
                let area =
                    |t: &crate::collision::Tri| (t.v[1] - t.v[0]).cross(t.v[2] - t.v[0]).length();
                area(a).total_cmp(&area(b))
            })
            .map(|(i, _)| i)
            .expect("the deck");
        let centre = |g: &crate::game::Game| mid(&g.collision.tris[tri]);
        let c0 = centre(&g);
        g.players[0].pos = c0;
        g.players[0].on_ground = true;
        g.players[0].vy = 0.0;
        let input = [crate::input::PlayerInput::default()];
        for _ in 0..5 {
            g.update(1.0 / 60.0, &input);
        }
        // Where the hero stands on the deck, in the deck's own frame.
        let local = |g: &crate::game::Game| {
            g.world_anim.world[o]
                .inverse()
                .transform_point3(g.players[0].pos)
        };
        let l0 = local(&g);
        for _ in 0..30 {
            g.update(1.0 / 60.0, &input);
        }
        let c1 = centre(&g);
        assert!(
            (c1 - c0).length() > 1.0,
            "the rail moved {}",
            (c1 - c0).length()
        );
        assert_eq!(g.world_anim.floors[0], Some(o as i32), "still on the deck");
        let slip = (local(&g) - l0).length();
        assert!(slip < 0.05, "the hero slipped {slip} on the deck");
    }
}
