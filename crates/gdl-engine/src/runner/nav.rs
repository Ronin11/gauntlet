//! The level runner's walkable graph: floor samples on a grid, joined where a
//! hero can actually walk between them. Edges are checked by stepping the
//! hero's own movement rules (PlayerCollideFloor's ledge limit, the one-sided
//! wall resolution, stairs, solid items) from one node to the next, so what
//! the graph says is walkable is what the game lets a hero walk. Edges are
//! worked out lazily and cached; collision changes (movers, doors, broken
//! walls, items) invalidate the cells they touch.

use crate::collision::Collision;
use crate::game::{Entity, Game, Shape, PLAYER_COL_HEIGHT, PLAYER_RADIUS, STAIR_RISE};
use glam::{Vec2, Vec3};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Maps and sets with a fixed hasher: the runner's choices (and so its
/// runs) come out the same every time.
pub type Fixed = std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>;
pub type HashMap<K, V> = std::collections::HashMap<K, V, Fixed>;
pub type HashSet<K> = std::collections::HashSet<K, Fixed>;

/// Grid spacing of the floor samples.
pub const CELL: f32 = 2.0;
/// The walking simulation's step: a hero runs ~9.5 units a second, 0.16 a tick.
const STEP: f32 = 0.16;
/// Most floors stacked in one cell.
const MAX_LAYERS: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct Node {
    pub pos: Vec3,
    pub cell: (i32, i32),
    pub alive: bool,
    /// The floor hurts (lava, fire): walked only when nothing else goes.
    pub hurts: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub to: u32,
    pub cost: f32,
    /// A transporter trip (pad index), not a walk.
    pub pad: Option<usize>,
}

/// What a goal is: somewhere within `r` (xz) and `dy` (height) of `pos`.
#[derive(Clone, Copy, Debug)]
pub struct Goal {
    pub pos: Vec3,
    pub r: f32,
    pub dy: f32,
}

impl Goal {
    pub fn new(pos: Vec3, r: f32, dy: f32) -> Goal {
        Goal { pos, r, dy }
    }
    pub fn holds(&self, p: Vec3) -> bool {
        xz(p - self.pos).length() <= self.r && (p.y - self.pos.y).abs() <= self.dy
    }
}

pub fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

/// A walking hero: where, falling speed, and on the floor or not.
#[derive(Clone, Copy, Debug)]
pub struct Hero {
    pub pos: Vec3,
    pub vy: f32,
    pub on_ground: bool,
}

/// The world as the walking simulation sees it.
pub struct WalkWorld<'a> {
    pub col: &'a Collision,
    pub ents: &'a [Entity],
    /// Solid entities to walk through (breakables the runner will smash).
    pub ignore: &'a HashSet<u64>,
    pub min_y: f32,
    /// Where a step was refused for want of floor although floor goes on
    /// just beyond (a seam between floor objects narrower than a step).
    pub cracks: std::cell::RefCell<Vec<Vec3>>,
}

impl WalkWorld<'_> {
    /// Game::resolve_solids, without the ignored entities.
    fn solids(&self, mut pos: Vec3) -> Vec3 {
        let radius = PLAYER_RADIUS;
        for e in self.ents {
            let Some(shape) = e.solid else { continue };
            if !e.alive || self.ignore.contains(&e.id) || (e.pos.y - pos.y).abs() > 4.0 {
                continue;
            }
            let d = Vec2::new(pos.x - e.pos.x, pos.z - e.pos.z);
            match shape {
                Shape::Cylinder { radius: r } => {
                    let len = d.length();
                    let min = r + radius;
                    if len < min {
                        let n = if len > 1e-4 { d / len } else { Vec2::X };
                        pos.x = e.pos.x + n.x * min;
                        pos.z = e.pos.z + n.y * min;
                    }
                }
                Shape::Box { half } => {
                    let (s, c) = e.yaw.sin_cos();
                    let local = Vec2::new(d.x * c - d.y * s, d.x * s + d.y * c);
                    let clamped = local.clamp(-half, half);
                    let diff = local - clamped;
                    let len = diff.length();
                    let push_local = if len > 1e-4 {
                        if len >= radius {
                            continue;
                        }
                        diff / len * (radius - len)
                    } else {
                        let px = half.x + radius - local.x.abs();
                        let pz = half.y + radius - local.y.abs();
                        if px < pz {
                            Vec2::new(px * local.x.signum(), 0.0)
                        } else {
                            Vec2::new(0.0, pz * local.y.signum())
                        }
                    };
                    let push = Vec2::new(
                        push_local.x * c + push_local.y * s,
                        -push_local.x * s + push_local.y * c,
                    );
                    pos.x += push.x;
                    pos.z += push.y;
                }
            }
        }
        pos
    }

    /// One tick of a hero's walk from `h` along `dir` for `s` units, as
    /// Game::update moves heroes: no step off a ledge (unless airborne),
    /// walls, stairs, solid items, then gravity and the floor
    /// (snap_to_ground). None when the hero would leave the world.
    pub fn step(&self, h: Hero, dir: Vec2, s: f32) -> Option<Hero> {
        const DT: f32 = 1.0 / 60.0;
        let col = self.col;
        let p = h.pos;
        let fwd = Vec3::new(dir.x, 0.0, dir.y);
        let target = p + fwd * s;
        let probe = if h.on_ground {
            Game::player_step(col, p, target - p)
        } else {
            None
        };
        if probe.is_none() && h.on_ground && s > 0.0 {
            let beyond = |d: f32| {
                Game::player_step(col, p, fwd * d).is_some_and(|(m, _)| xz(m).dot(dir) > 0.0)
            };
            if beyond(s + 0.25) || beyond(s + 0.5) {
                self.cracks.borrow_mut().push(target);
            }
        }
        let q = match probe {
            Some((d, _)) => p + d,
            None if !h.on_ground => target,
            None => p,
        };
        let (mut pos, wall_hit) = Game::player_walls(col, q);
        if h.on_ground {
            if let Some((y, t)) = col.ground_tri(pos, pos.y + STAIR_RISE) {
                if col.stairs[t] && y > pos.y {
                    pos.y = y;
                }
            }
        }
        let walled = pos;
        pos = self.solids(pos);
        let mut vy = h.vy;
        let g = Game::player_snap(col, &mut pos, &mut vy, DT);
        let on_ground = g.is_some();
        if std::env::var_os("GDL_NAV_DEBUG").is_some() {
            println!(
                "    step {p:?} -> target {target:?} probe {probe:?} walls {walled:?} solids {pos:?} ground {g:?} on {on_ground} wall {wall_hit:?}",
            );
        }
        (pos.y > self.min_y).then_some(Hero { pos, vy, on_ground })
    }

    /// Walk from `from` toward `to`: where the hero ends up (landed).
    pub fn walk(&self, from: Vec3, to: Vec3) -> Vec3 {
        let mut h = Hero {
            pos: from,
            vy: 0.0,
            on_ground: true,
        };
        let dist = xz(to - from).length();
        let max = (dist / STEP * 2.0) as usize + 4;
        for _ in 0..max {
            let d = xz(to - h.pos);
            let len = d.length();
            if len < 0.05 {
                break;
            }
            match self.step(h, d / len, STEP.min(len)) {
                Some(q) => {
                    let still =
                        xz(q.pos - h.pos).length() < 0.01 && (q.pos.y - h.pos.y).abs() < 0.01;
                    h = q;
                    if still && h.on_ground {
                        break;
                    }
                }
                None => return h.pos - Vec3::Y * 1000.0,
            }
        }
        // Land.
        for _ in 0..120 {
            if h.on_ground {
                break;
            }
            match self.step(h, Vec2::ZERO, 0.0) {
                Some(q) => h = q,
                None => return h.pos - Vec3::Y * 1000.0,
            }
        }
        h.pos
    }

    /// Whether a hero walks from `from` to `to`.
    pub fn walks(&self, from: Vec3, to: Vec3) -> bool {
        let end = self.walk(from, to);
        xz(end - to).length() < 0.5 && (end.y - to.y).abs() < 1.0
    }
}

pub struct Nav {
    pub min: Vec2,
    pub nx: i32,
    pub nz: i32,
    pub nodes: Vec<Node>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    edges: Vec<Option<Vec<Edge>>>,
    /// Edges found not to work in play (stuck there), until a game time.
    pub blocked: HashMap<(u32, u32), f32>,
    /// Transporter pads: (pad index, node on it, node on its partner).
    pads: Vec<(usize, u32, u32)>,
    /// Collision as of the last refresh, to find what changed.
    last_verts: Vec<[Vec3; 3]>,
    last_enabled: Vec<bool>,
    last_solids: HashMap<u64, Vec3>,
    pub ignore: HashSet<u64>,
    min_y: f32,
    /// Floor seams that stopped an edge walk (see WalkWorld::cracks).
    pub cracks: Vec<Vec3>,
    /// Refresh bookkeeping: looks taken, and how many looks in a row each
    /// triangle has moved.
    checks: u64,
    streak: Vec<u8>,
    /// Damage tiles: (position, radius).
    hazards: Vec<(Vec3, f32)>,
    /// Edge walks done (for the stats).
    pub walks: u64,
}

impl Nav {
    pub fn new(g: &Game) -> Nav {
        let w = &g.level.worlds;
        let (mut lo, mut hi) = (
            Vec2::new(w.min_bounds[0], w.min_bounds[2]),
            Vec2::new(w.max_bounds[0], w.max_bounds[2]),
        );
        // Bounds from the collision itself when the header's are odd.
        if !(hi.x > lo.x && hi.y > lo.y) || (hi - lo).max_element() > 5000.0 {
            lo = Vec2::splat(f32::MAX);
            hi = Vec2::splat(f32::MIN);
            for t in &g.collision.tris {
                for v in t.v {
                    lo = lo.min(xz(v));
                    hi = hi.max(xz(v));
                }
            }
        }
        let nx = ((hi.x - lo.x) / CELL).ceil() as i32 + 1;
        let nz = ((hi.y - lo.y) / CELL).ceil() as i32 + 1;
        let mut nav = Nav {
            min: lo,
            nx,
            nz,
            nodes: Vec::new(),
            cells: HashMap::default(),
            edges: Vec::new(),
            blocked: HashMap::default(),
            pads: Vec::new(),
            last_verts: g.collision.tris.iter().map(|t| t.v).collect(),
            last_enabled: g.collision.enabled.clone(),
            last_solids: solid_set(g),
            ignore: HashSet::default(),
            min_y: w.min_bounds[1],
            cracks: Vec::new(),
            checks: 0,
            streak: Vec::new(),
            hazards: g
                .entities
                .iter()
                .filter_map(|e| match e.kind {
                    crate::game::Kind::Hazard { radius, .. } => Some((e.pos, radius)),
                    // A secret icon would end the level (into its secret level).
                    crate::game::Kind::Secret { radius, .. } => Some((e.pos, radius + 1.0)),
                    _ => None,
                })
                .collect(),
            walks: 0,
        };
        for iz in 0..nz {
            for ix in 0..nx {
                nav.build_cell(g, (ix, iz));
            }
        }
        nav.find_pads(g);
        nav
    }

    pub fn cell_of(&self, p: Vec3) -> (i32, i32) {
        (
            ((p.x - self.min.x) / CELL).floor() as i32,
            ((p.z - self.min.y) / CELL).floor() as i32,
        )
    }

    fn centre(&self, c: (i32, i32)) -> Vec2 {
        self.min + Vec2::new(c.0 as f32 + 0.5, c.1 as f32 + 0.5) * CELL
    }

    fn world<'a>(&'a self, g: &'a Game) -> WalkWorld<'a> {
        WalkWorld {
            col: &g.collision,
            ents: &g.entities,
            ignore: &self.ignore,
            min_y: self.min_y,
            cracks: Default::default(),
        }
    }

    /// The floors of a cell, each moved to a spot where a hero fits.
    fn build_cell(&mut self, g: &Game, c: (i32, i32)) {
        let col = &g.collision;
        let ctr = self.centre(c);
        let top0 = g.level.worlds.max_bounds[1].max(self.min_y + 1.0) + 20.0;
        let mut ids: Vec<u32> = Vec::new();
        // The centre first, then spots around it: a narrow ledge may miss
        // the centre, or have no room for a hero there.
        let k = CELL * 0.3;
        const OFFS: [(f32, f32); 9] = [
            (0.0, 0.0),
            (1.0, 0.0),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (1.0, 1.0),
            (-1.0, 1.0),
            (1.0, -1.0),
            (-1.0, -1.0),
        ];
        for (ox, oz) in OFFS {
            let q = ctr + Vec2::new(ox, oz) * k;
            let mut top = top0;
            for _ in 0..MAX_LAYERS {
                let at = Vec3::new(q.x, top, q.y);
                let Some(y) = col.ground_height(at, top) else {
                    break;
                };
                top = y - 1.5;
                if y <= self.min_y {
                    break;
                }
                if ids
                    .iter()
                    .any(|&i| (self.nodes[i as usize].pos.y - y).abs() < 1.0)
                {
                    continue;
                }
                let p0 = Vec3::new(q.x, y, q.y);
                let (p1, _) = Game::player_walls(col, p0);
                let (p2, _) = Game::player_walls(col, p1);
                if xz(p2 - p1).length() > 0.05 || xz(p2 - ctr3(ctr, y)).length() > CELL * 0.75 {
                    continue;
                }
                let Some((y2, owner)) = col
                    .player_floor(p2, PLAYER_RADIUS, PLAYER_COL_HEIGHT, true)
                    .map(|h| (h.at.y, h.owner))
                else {
                    continue;
                };
                if (y2 - y).abs() > 1.5 {
                    continue;
                }
                let pos = Vec3::new(p2.x, y2, p2.z);
                // No standing near a secret icon (touching it ends the level).
                if g.entities.iter().any(|e| matches!(e.kind, crate::game::Kind::Secret { radius, used: false, .. } if xz(e.pos - pos).length() < radius + PLAYER_RADIUS + 0.5 && (e.pos.y - pos.y).abs() < 4.0)) {
                    continue;
                }
                if ids
                    .iter()
                    .any(|&i| (self.nodes[i as usize].pos.y - y2).abs() < 1.0)
                {
                    continue;
                }
                let id = self.nodes.len() as u32;
                self.nodes.push(Node {
                    pos,
                    cell: c,
                    alive: true,
                    hurts: g.hurting_surface(owner).is_some()
                        || self.hazards.iter().any(|&(hp, r)| {
                            xz(hp - pos).length() < r + PLAYER_RADIUS && (hp.y - pos.y).abs() < 4.0
                        }),
                });
                self.edges.push(None);
                ids.push(id);
            }
        }
        if ids.is_empty() {
            self.cells.remove(&c);
        } else {
            self.cells.insert(c, ids);
        }
    }

    fn find_pads(&mut self, g: &Game) {
        self.pads.clear();
        let pads = &g.transport.pads;
        for (i, p) in pads.iter().enumerate() {
            let Some(d) = p.dest else { continue };
            let (Some(a), Some(b)) = (
                self.node_near(p.pos, p.radius + PLAYER_RADIUS - 0.2, 2.0),
                self.node_near(pads[d].pos, CELL * 1.5, 2.5),
            ) else {
                continue;
            };
            self.pads.push((i, a, b));
        }
    }

    /// The live node closest to `p` (xz within `r`, height within `dy`).
    pub fn node_near(&self, p: Vec3, r: f32, dy: f32) -> Option<u32> {
        let c = self.cell_of(p);
        let k = (r / CELL).ceil() as i32 + 1;
        let mut best: Option<(f32, u32)> = None;
        for dz in -k..=k {
            for dx in -k..=k {
                let Some(ids) = self.cells.get(&(c.0 + dx, c.1 + dz)) else {
                    continue;
                };
                for &i in ids {
                    let n = &self.nodes[i as usize];
                    let d = xz(n.pos - p).length();
                    if n.alive
                        && d <= r
                        && (n.pos.y - p.y).abs() <= dy
                        && best.is_none_or(|b| d < b.0)
                    {
                        best = Some((d, i));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// The node a hero standing at `p` starts a path from: the nearest one
    /// it can walk straight to.
    pub fn start_node(&mut self, g: &Game, p: Vec3) -> Option<u32> {
        let c = self.cell_of(p);
        let mut cands: Vec<(f32, u32)> = Vec::new();
        for dz in -2..=2 {
            for dx in -2..=2 {
                if let Some(ids) = self.cells.get(&(c.0 + dx, c.1 + dz)) {
                    for &i in ids {
                        let n = &self.nodes[i as usize];
                        if n.alive && (n.pos.y - p.y).abs() < 3.5 {
                            cands.push((xz(n.pos - p).length() + (n.pos.y - p.y).abs(), i));
                        }
                    }
                }
            }
        }
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        for &(_, i) in cands.iter().take(8) {
            self.walks += 1;
            if self.world(g).walks(p, self.nodes[i as usize].pos) {
                return Some(i);
            }
        }
        cands.first().map(|c| c.1)
    }

    /// A node's edges, worked out on first use.
    pub fn edges(&mut self, g: &Game, n: u32) -> Vec<Edge> {
        if let Some(e) = &self.edges[n as usize] {
            return e.clone();
        }
        let node = self.nodes[n as usize];
        let mut out = Vec::new();
        if node.alive {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dz == 0 {
                        continue;
                    }
                    let Some(ids) = self.cells.get(&(node.cell.0 + dx, node.cell.1 + dz)) else {
                        continue;
                    };
                    for &m in ids.clone().iter() {
                        let mn = self.nodes[m as usize];
                        let dy = mn.pos.y - node.pos.y;
                        // Up a step or stairs; down a step, or a fall off an edge.
                        if !mn.alive || dy > STAIR_RISE + 1.5 || dy < -30.0 {
                            continue;
                        }
                        self.walks += 1;
                        let w = self.world(g);
                        let ok = w.walks(node.pos, mn.pos);
                        let cr = w.cracks.take();
                        if !ok {
                            if let Some(c) = cr.last() {
                                self.cracks.push(*c);
                            }
                        }
                        if ok {
                            let d = (mn.pos - node.pos).length();
                            let mut cost = if mn.hurts { d * 12.0 + 20.0 } else { d };
                            // A drop off an edge is one way: only when nothing else goes.
                            if dy < -(STAIR_RISE + 1.0) {
                                cost += 60.0;
                            }
                            out.push(Edge {
                                to: m,
                                cost,
                                pad: None,
                            });
                        }
                    }
                }
            }
        }
        self.edges[n as usize] = Some(out.clone());
        out
    }

    fn all_edges(&mut self, g: &Game, n: u32) -> Vec<Edge> {
        let mut e = self.edges(g, n);
        for &(pad, a, b) in &self.pads {
            if a == n {
                e.push(Edge {
                    to: b,
                    cost: 8.0,
                    pad: Some(pad),
                });
            }
        }
        e
    }

    /// Recheck what changed in the world since the last look; drop the
    /// nodes and edges it touches. Returns whether anything changed.
    pub fn refresh(&mut self, g: &Game) -> bool {
        let mut boxes: Vec<(Vec2, Vec2)> = Vec::new();
        let col = &g.collision;
        if col.tris.len() == self.last_verts.len() {
            // Per moving object; objects that never stop (spinners,
            // conveyors) only every 20th look.
            self.checks += 1;
            if self.streak.len() != col.tris.len() {
                self.streak = vec![0; col.tris.len()];
            }
            let mut per: HashMap<i32, (Vec2, Vec2, bool)> = HashMap::default();
            let mut changed: Vec<usize> = Vec::new();
            for (i, t) in col.tris.iter().enumerate() {
                let was = &self.last_verts[i];
                if t.v != *was || col.enabled[i] != self.last_enabled[i] {
                    self.streak[i] = self.streak[i].saturating_add(1);
                    let e = per.entry(t.owner).or_insert((
                        Vec2::splat(f32::MAX),
                        Vec2::splat(f32::MIN),
                        false,
                    ));
                    for v in t.v.iter().chain(was.iter()) {
                        e.0 = e.0.min(xz(*v));
                        e.1 = e.1.max(xz(*v));
                    }
                    e.2 |= self.streak[i] >= 4;
                    changed.push(i);
                } else {
                    self.streak[i] = 0;
                }
            }
            let due = self.checks % 120 == 0;
            // ...or when a hero is close by (riding it, say).
            let near = |lo: Vec2, hi: Vec2| {
                g.players.iter().any(|p| {
                    let q = xz(p.pos).clamp(lo, hi);
                    (q - xz(p.pos)).length() < 16.0
                })
            };
            let take: HashMap<i32, bool> = per
                .iter()
                .map(|(o, e)| (*o, !e.2 || due || near(e.0, e.1)))
                .collect();
            for i in changed {
                if take.get(&col.tris[i].owner).copied().unwrap_or(true) {
                    self.last_verts[i] = col.tris[i].v;
                    self.last_enabled[i] = col.enabled[i];
                }
            }
            for (o, (lo, hi, volatile)) in per {
                if !take[&o] {
                    continue;
                }
                if volatile && !due {
                    // Only around the heroes: a big object that never stops
                    // (water, a spinning room) would rebuild half the level.
                    for p in &g.players {
                        let c = xz(p.pos);
                        let (l, h) = (lo.max(c - Vec2::splat(14.0)), hi.min(c + Vec2::splat(14.0)));
                        if l.x <= h.x && l.y <= h.y {
                            boxes.push((l, h));
                        }
                    }
                } else {
                    boxes.push((lo, hi));
                }
            }
        }
        let now = solid_set(g);
        for (id, p) in &now {
            if self
                .last_solids
                .get(id)
                .is_none_or(|q| q.distance(*p) > 0.3)
            {
                boxes.push((xz(*p) - Vec2::splat(6.0), xz(*p) + Vec2::splat(6.0)));
                if let Some(q) = self.last_solids.get(id) {
                    boxes.push((xz(*q) - Vec2::splat(6.0), xz(*q) + Vec2::splat(6.0)));
                }
            }
        }
        for (id, p) in &self.last_solids {
            if !now.contains_key(id) {
                boxes.push((xz(*p) - Vec2::splat(6.0), xz(*p) + Vec2::splat(6.0)));
            }
        }
        self.last_solids = now;
        if boxes.is_empty() {
            return false;
        }
        for (lo, hi) in boxes {
            self.invalidate(g, lo, hi);
        }
        self.find_pads(g);
        true
    }

    /// Rebuild the cells in a box and forget the edges around it.
    pub fn invalidate(&mut self, g: &Game, lo: Vec2, hi: Vec2) {
        let c0 = self.cell_of(Vec3::new(lo.x - PLAYER_RADIUS, 0.0, lo.y - PLAYER_RADIUS));
        let c1 = self.cell_of(Vec3::new(hi.x + PLAYER_RADIUS, 0.0, hi.y + PLAYER_RADIUS));
        let (x0, x1) = (c0.0.max(0), c1.0.min(self.nx - 1));
        let (z0, z1) = (c0.1.max(0), c1.1.min(self.nz - 1));
        for iz in z0..=z1 {
            for ix in x0..=x1 {
                if let Some(ids) = self.cells.get(&(ix, iz)) {
                    for &i in ids {
                        self.nodes[i as usize].alive = false;
                        self.edges[i as usize] = Some(Vec::new());
                    }
                }
                self.build_cell(g, (ix, iz));
            }
        }
        for iz in (z0 - 1)..=(z1 + 1) {
            for ix in (x0 - 1)..=(x1 + 1) {
                if let Some(ids) = self.cells.get(&(ix, iz)) {
                    for &i in ids {
                        self.edges[i as usize] = None;
                    }
                }
            }
        }
    }

    /// Forget every edge (when the ignored set changes).
    pub fn reset_edges(&mut self) {
        for e in &mut self.edges {
            *e = None;
        }
    }

    /// A* from `start` to any node meeting a goal; `time` drops blocks that
    /// have run out. The path (node ids, start first) and its cost.
    pub fn path(
        &mut self,
        g: &Game,
        start: u32,
        goals: &[Goal],
        time: f32,
        max_expand: usize,
    ) -> Option<(Vec<u32>, f32)> {
        if goals.is_empty() {
            return None;
        }
        self.blocked.retain(|_, until| *until > time);
        let h = |p: Vec3| {
            goals
                .iter()
                .map(|gl| (xz(p - gl.pos).length() - gl.r).max(0.0))
                .fold(f32::MAX, f32::min)
        };
        let mut open = BinaryHeap::new();
        let mut came: HashMap<u32, (u32, f32)> = HashMap::default();
        let mut closed: HashSet<u32> = HashSet::default();
        came.insert(start, (u32::MAX, 0.0));
        open.push(Open {
            f: h(self.nodes[start as usize].pos),
            g: 0.0,
            n: start,
        });
        let mut expanded = 0;
        while let Some(Open { g: gc, n, .. }) = open.pop() {
            if !closed.insert(n) {
                continue;
            }
            let pos = self.nodes[n as usize].pos;
            if goals.iter().any(|gl| gl.holds(pos)) {
                let mut path = vec![n];
                let mut c = n;
                while let Some(&(p, _)) = came.get(&c) {
                    if p == u32::MAX {
                        break;
                    }
                    path.push(p);
                    c = p;
                }
                path.reverse();
                return Some((path, gc));
            }
            expanded += 1;
            if expanded > max_expand {
                break;
            }
            for e in self.all_edges(g, n) {
                if closed.contains(&e.to)
                    || !self.nodes[e.to as usize].alive
                    || self.blocked.contains_key(&(n, e.to))
                {
                    continue;
                }
                let ng = gc + e.cost;
                if came.get(&e.to).is_none_or(|&(_, old)| ng < old) {
                    came.insert(e.to, (n, ng));
                    open.push(Open {
                        f: ng + h(self.nodes[e.to as usize].pos),
                        g: ng,
                        n: e.to,
                    });
                }
            }
        }
        None
    }

    /// Every node reachable from `start` (up to `max`).
    pub fn reachable(&mut self, g: &Game, start: u32, max: usize) -> HashSet<u32> {
        let mut seen = HashSet::default();
        let mut stack = vec![start];
        seen.insert(start);
        while let Some(n) = stack.pop() {
            if seen.len() > max {
                break;
            }
            for e in self.all_edges(g, n) {
                if self.nodes[e.to as usize].alive && seen.insert(e.to) {
                    stack.push(e.to);
                }
            }
        }
        seen
    }

    /// The edge from a to b is a transporter trip.
    pub fn pad_edge(&self, a: u32, b: u32) -> Option<usize> {
        self.pads
            .iter()
            .find(|&&(_, x, y)| x == a && y == b)
            .map(|p| p.0)
    }

    /// World objects whose collision keeps moving (moving platforms,
    /// spinners): owner, centre and xz extent.
    pub fn volatile(&self, g: &Game) -> Vec<(i32, Vec3, f32)> {
        let mut per: HashMap<i32, (Vec3, Vec3)> = HashMap::default();
        for (i, &s) in self.streak.iter().enumerate() {
            if s >= 4 {
                let t = &g.collision.tris[i];
                let e = per
                    .entry(t.owner)
                    .or_insert((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)));
                for v in t.v {
                    e.0 = e.0.min(v);
                    e.1 = e.1.max(v);
                }
            }
        }
        let mut v: Vec<(i32, Vec3, f32)> = per
            .into_iter()
            .map(|(o, (lo, hi))| (o, (lo + hi) * 0.5, xz(hi - lo).length() * 0.5))
            .collect();
        v.sort_by_key(|x| x.0);
        v
    }

    pub fn live_nodes(&self) -> usize {
        self.nodes.iter().filter(|n| n.alive).count()
    }
}

/// Solid entities by id and where they are.
fn solid_set(g: &Game) -> HashMap<u64, Vec3> {
    g.entities
        .iter()
        .filter(|e| e.alive && e.solid.is_some())
        .map(|e| (e.id, e.pos))
        .collect()
}

#[derive(PartialEq)]
struct Open {
    f: f32,
    g: f32,
    n: u32,
}
impl Eq for Open {}
impl Ord for Open {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f).then(self.n.cmp(&o.n))
    }
}
impl PartialOrd for Open {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

fn ctr3(c: Vec2, y: f32) -> Vec3 {
    Vec3::new(c.x, y, c.y)
}
