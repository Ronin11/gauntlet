//! World collision: ground height queries and cylinder-vs-wall resolution against
//! the level's collision triangles (which are stored in world space).

use crate::math::Det as _;
use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Tri {
    pub v: [Vec3; 3],
    pub n: Vec3,
    /// Owning world object (for switching moving/destroyable geometry on and off).
    pub owner: i32,
}

#[derive(Debug, Clone)]
pub struct Collision {
    pub tris: Vec<Tri>,
    pub enabled: Vec<bool>,
    /// The triangle's world object is stairs or a ramp (flags & 0x38): its
    /// risers don't stop a walking player (fn_80088714), who climbs them by
    /// the floor (PlayerCollideFloor).
    pub stairs: Vec<bool>,
    /// The flags of the triangle's world object (0 for none):
    /// PlayerCollideFloor reads bit 8 of the floor ahead.
    pub obj_flags: Vec<u32>,
    min: Vec2,
    cell: f32,
    nx: usize,
    nz: usize,
    grid: Vec<Vec<u32>>,
    /// Triangles of moving world objects, kept out of the grid
    /// (WorldDynCollide): per group, an XZ bounding circle and the triangles.
    dynamic: Vec<DynGroup>,
}

#[derive(Debug, Clone)]
struct DynGroup {
    centre: Vec2,
    radius: f32,
    tris: Vec<u32>,
}

/// Surfaces steeper than this (normal.y below it) are walls.
const FLOOR_NY: f32 = 0.5;

fn point_in_tri_xz(p: Vec2, t: &Tri) -> bool {
    let a = Vec2::new(t.v[0].x, t.v[0].z);
    let b = Vec2::new(t.v[1].x, t.v[1].z);
    let c = Vec2::new(t.v[2].x, t.v[2].z);
    let s = |p0: Vec2, p1: Vec2| (p1 - p0).perp_dot(p - p0);
    let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

fn closest_point_on_tri(p: Vec3, t: &Tri) -> Vec3 {
    // Ericson, Real-Time Collision Detection 5.1.5
    let (a, b, c) = (t.v[0], t.v[1], t.v[2]);
    let ab = b - a;
    let ac = c - a;
    let ap = p - a;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = 1.0 / (va + vb + vc);
    a + ab * (vb * denom) + ac * (vc * denom)
}

impl Collision {
    pub fn new(tris: Vec<Tri>) -> Self {
        let mut min = Vec2::splat(f32::MAX);
        let mut max = Vec2::splat(f32::MIN);
        for t in &tris {
            for v in t.v {
                min = min.min(Vec2::new(v.x, v.z));
                max = max.max(Vec2::new(v.x, v.z));
            }
        }
        if tris.is_empty() {
            min = Vec2::ZERO;
            max = Vec2::ONE;
        }
        let cell = 4.0;
        let nx = (((max.x - min.x) / cell).ceil() as usize).max(1);
        let nz = (((max.y - min.y) / cell).ceil() as usize).max(1);
        let mut grid = vec![Vec::new(); nx * nz];
        for (i, t) in tris.iter().enumerate() {
            let tmin =
                t.v.iter()
                    .fold(Vec2::splat(f32::MAX), |m, v| m.min(Vec2::new(v.x, v.z)));
            let tmax =
                t.v.iter()
                    .fold(Vec2::splat(f32::MIN), |m, v| m.max(Vec2::new(v.x, v.z)));
            let x0 =
                (((tmin.x - min.x) / cell).floor() as isize).clamp(0, nx as isize - 1) as usize;
            let x1 =
                (((tmax.x - min.x) / cell).floor() as isize).clamp(0, nx as isize - 1) as usize;
            let z0 =
                (((tmin.y - min.y) / cell).floor() as isize).clamp(0, nz as isize - 1) as usize;
            let z1 =
                (((tmax.y - min.y) / cell).floor() as isize).clamp(0, nz as isize - 1) as usize;
            for z in z0..=z1 {
                for x in x0..=x1 {
                    grid[z * nx + x].push(i as u32);
                }
            }
        }
        let enabled = vec![true; tris.len()];
        let stairs = vec![false; tris.len()];
        let obj_flags = vec![0; tris.len()];
        Collision {
            tris,
            enabled,
            stairs,
            obj_flags,
            min,
            cell,
            nx,
            nz,
            grid,
            dynamic: Vec::new(),
        }
    }

    /// Take these groups of triangles out of the static grid: they move
    /// (animated world objects). Call [`Collision::refit_dynamic`] after
    /// moving them.
    pub fn make_dynamic(&mut self, groups: Vec<Vec<usize>>) {
        let moving: std::collections::BTreeSet<u32> =
            groups.iter().flatten().map(|&i| i as u32).collect();
        if moving.is_empty() {
            return;
        }
        for cell in &mut self.grid {
            cell.retain(|i| !moving.contains(i));
        }
        self.dynamic.extend(
            groups
                .into_iter()
                .filter(|g| !g.is_empty())
                .map(|g| DynGroup {
                    centre: Vec2::ZERO,
                    radius: 0.0,
                    tris: g.into_iter().map(|i| i as u32).collect(),
                }),
        );
        self.refit_dynamic();
    }

    /// Recompute the moving groups' bounding circles.
    pub fn refit_dynamic(&mut self) {
        for g in &mut self.dynamic {
            let mut lo = Vec2::splat(f32::MAX);
            let mut hi = Vec2::splat(f32::MIN);
            for &i in &g.tris {
                for v in self.tris[i as usize].v {
                    lo = lo.min(Vec2::new(v.x, v.z));
                    hi = hi.max(Vec2::new(v.x, v.z));
                }
            }
            g.centre = (lo + hi) * 0.5;
            g.radius = (hi - lo).length() * 0.5;
        }
    }

    fn cells_around(&self, p: Vec2, r: f32) -> impl Iterator<Item = u32> + '_ {
        let x0 = (((p.x - r - self.min.x) / self.cell).floor() as isize)
            .clamp(0, self.nx as isize - 1) as usize;
        let x1 = (((p.x + r - self.min.x) / self.cell).floor() as isize)
            .clamp(0, self.nx as isize - 1) as usize;
        let z0 = (((p.y - r - self.min.y) / self.cell).floor() as isize)
            .clamp(0, self.nz as isize - 1) as usize;
        let z1 = (((p.y + r - self.min.y) / self.cell).floor() as isize)
            .clamp(0, self.nz as isize - 1) as usize;
        let mut seen = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                for &i in &self.grid[z * self.nx + x] {
                    if !seen.contains(&i) {
                        seen.push(i);
                    }
                }
            }
        }
        for g in &self.dynamic {
            if (g.centre - p).length() <= g.radius + r + 0.01 {
                seen.extend_from_slice(&g.tris);
            }
        }
        seen.into_iter().filter(|&i| self.enabled[i as usize])
    }

    /// Highest floor at (x, z) that is no higher than `max_y`.
    pub fn ground_height(&self, pos: Vec3, max_y: f32) -> Option<f32> {
        self.ground_hit(pos, max_y).map(|(y, _)| y)
    }

    /// The highest floor and its triangle's owner.
    pub fn ground_hit(&self, pos: Vec3, max_y: f32) -> Option<(f32, i32)> {
        self.ground_tri(pos, max_y)
            .map(|(y, i)| (y, self.tris[i].owner))
    }

    /// [`Collision::ground_hit`], giving the floor triangle's index (its
    /// normal is `tris[i].n`).
    pub fn ground_tri(&self, pos: Vec3, max_y: f32) -> Option<(f32, usize)> {
        let p = Vec2::new(pos.x, pos.z);
        let mut best: Option<(f32, usize)> = None;
        for i in self.cells_around(p, 0.0) {
            let t = &self.tris[i as usize];
            if t.n.y < FLOOR_NY || !point_in_tri_xz(p, t) {
                continue;
            }
            // Plane: n . (x - v0) = 0 -> y = v0.y - (n.x (x - v0.x) + n.z (z - v0.z)) / n.y
            let y = t.v[0].y - (t.n.x * (p.x - t.v[0].x) + t.n.z * (p.y - t.v[0].z)) / t.n.y;
            if y <= max_y && best.is_none_or(|b| y > b.0) {
                best = Some((y, i as usize));
            }
        }
        best
    }

    /// Push a vertical cylinder (feet at `pos.y`) out of walls. Returns the corrected position.
    pub fn resolve_walls(&self, pos: Vec3, radius: f32, height: f32, step: f32) -> Vec3 {
        self.resolve_walls_hit(pos, radius, height, step).0
    }

    /// resolve_walls, also giving the last wall touched: its owner and the
    /// contact point.
    pub fn resolve_walls_hit(
        &self,
        pos: Vec3,
        radius: f32,
        height: f32,
        step: f32,
    ) -> (Vec3, Option<(i32, Vec3)>) {
        self.resolve_walls_stairs(pos, radius, height, step, step)
    }

    /// [`Collision::resolve_walls_hit`] with stairs' walls (see
    /// [`Collision::stairs`]) only solid above `stair_step`.
    pub fn resolve_walls_stairs(
        &self,
        mut pos: Vec3,
        radius: f32,
        height: f32,
        step: f32,
        stair_step: f32,
    ) -> (Vec3, Option<(i32, Vec3)>) {
        let mut hit = None;
        for _ in 0..3 {
            let mut moved = false;
            let p2 = Vec2::new(pos.x, pos.z);
            let cands: Vec<u32> = self.cells_around(p2, radius + 0.5).collect();
            for i in cands {
                let t = &self.tris[i as usize];
                if t.n.y >= FLOOR_NY {
                    continue;
                }
                // Test against the cylinder's axis segment above the step height.
                let lo = pos.y
                    + if self.stairs[i as usize] {
                        stair_step
                    } else {
                        step
                    };
                let hi = pos.y + height;
                let tmin = t.v[0].y.min(t.v[1].y).min(t.v[2].y);
                let tmax = t.v[0].y.max(t.v[1].y).max(t.v[2].y);
                if tmax < lo || tmin > hi {
                    continue;
                }
                let probe_y = ((tmin + tmax) * 0.5).clamp(lo, hi);
                let probe = Vec3::new(pos.x, probe_y, pos.z);
                // Walls are one-sided (BTriLineCol returns no hit when the
                // query starts behind the triangle's plane): a hero walks in
                // through the back of the Lich arena's fence lines, which
                // then keep the party (and the Lich) inside.
                if (probe - t.v[0]).dot(t.n) < 0.0 {
                    continue;
                }
                let c = closest_point_on_tri(probe, t);
                let d = Vec2::new(probe.x - c.x, probe.z - c.z);
                let dist = d.length();
                if dist < radius && (c.y - probe_y).abs() < height {
                    let push = if dist > 1e-4 {
                        d / dist
                    } else {
                        Vec2::new(t.n.x, t.n.z).normalize_or_zero()
                    };
                    let amt = radius - dist;
                    pos.x += push.x * amt;
                    pos.z += push.y * amt;
                    moved = true;
                    hit = Some((t.owner, c));
                }
            }
            if !moved {
                break;
            }
        }
        (pos, hit)
    }
}

impl Collision {
    /// First hit of the segment `from -> to` against enabled triangles, as a fraction 0..1.
    pub fn raycast(&self, from: Vec3, to: Vec3) -> Option<f32> {
        self.raycast_hit(from, to).map(|(t, _)| t)
    }

    /// The world hit of a sphere of radius `r` moving `from -> to`
    /// (WeaponWallCollide, as a ray reaching `r` past the move): where its
    /// centre stops, and the surface normal.
    pub fn sweep(&self, from: Vec3, to: Vec3, r: f32) -> Option<(Vec3, Vec3)> {
        self.sweep_tri(from, to, r).map(|(at, n, _)| (at, n))
    }

    /// [`Collision::sweep`], also giving the triangle hit.
    pub fn sweep_tri(&self, from: Vec3, to: Vec3, r: f32) -> Option<(Vec3, Vec3, usize)> {
        let d = (to - from).normalize_or_zero();
        let (t, n, i) = self.raycast_tri(from, to + d * r)?;
        let len = (to - from).length() + r;
        Some((from + d * (t * len - r).max(0.0), n, i))
    }

    /// Whether an upright cylinder at `pos` (`radius`, `height` above it)
    /// touches one of the enabled triangles in `range`.
    pub fn touches(
        &self,
        range: std::ops::Range<usize>,
        pos: Vec3,
        radius: f32,
        height: f32,
    ) -> bool {
        let end = range.end.min(self.tris.len());
        (range.start..end).any(|i| {
            if !self.enabled[i] {
                return false;
            }
            let t = &self.tris[i];
            let tmin = t.v[0].y.min(t.v[1].y).min(t.v[2].y);
            let tmax = t.v[0].y.max(t.v[1].y).max(t.v[2].y);
            let (lo, hi) = (pos.y, pos.y + height);
            if tmax < lo || tmin > hi {
                return false;
            }
            let probe = Vec3::new(pos.x, ((tmin + tmax) * 0.5).clamp(lo, hi), pos.z);
            let c = closest_point_on_tri(probe, t);
            Vec2::new(probe.x - c.x, probe.z - c.z).length() <= radius
                && (c.y - probe.y).abs() <= height
        })
    }

    /// raycast, also giving the hit triangle's normal.
    pub fn raycast_hit(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
        self.raycast_tri(from, to).map(|(t, n, _)| (t, n))
    }

    /// raycast, also giving the hit triangle's normal and index.
    pub fn raycast_tri(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3, usize)> {
        let dir = to - from;
        let len = dir.length();
        if len < 1e-4 {
            return None;
        }
        // Gather candidate triangles from cells along the segment.
        let steps = (len / (self.cell * 0.5)).ceil() as usize + 1;
        let mut cands: Vec<u32> = Vec::new();
        for s in 0..=steps {
            let p = from + dir * (s as f32 / steps as f32);
            for i in self.cells_around(Vec2::new(p.x, p.z), self.cell * 0.25) {
                if !cands.contains(&i) {
                    cands.push(i);
                }
            }
        }
        let mut best: Option<(f32, Vec3, usize)> = None;
        for i in cands {
            let t = &self.tris[i as usize];
            // Moller-Trumbore, double-sided.
            let e1 = t.v[1] - t.v[0];
            let e2 = t.v[2] - t.v[0];
            let p = dir.cross(e2);
            let det = e1.dot(p);
            if det.abs() < 1e-7 {
                continue;
            }
            let inv = 1.0 / det;
            let s = from - t.v[0];
            let u = s.dot(p) * inv;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let q = s.cross(e1);
            let v = dir.dot(q) * inv;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let tt = e2.dot(q) * inv;
            if (0.0..=1.0).contains(&tt) && best.is_none_or(|b| tt < b.0) {
                best = Some((tt, t.n, i as usize));
            }
        }
        best
    }
}

/// One hit of [`Collision::floor_collide`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FloorHit {
    /// The hit point on the triangle: under the query for a hit inside it,
    /// else the closest point of its nearest edge (BTriLineCol's o2).
    pub at: Vec3,
    pub tri: usize,
    pub owner: i32,
}

/// The floor's normal.y range (lbl_80345750 .. lbl_80345724: 0.5 .. 2).
const FLOOR_NY_MAX: f32 = 2.0;
/// The walls' normal.y range (lbl_80345740 .. lbl_80345744: -0.866 .. 0.866).
const WALL_NY: f32 = 0.866;

/// The squared distance between the segments `p1 -> q1` and `p2 -> q2`, and
/// the closest point on the second (Ericson, Real-Time Collision Detection
/// 5.1.9): btricol.c's LineLineDist (and PointLineDist2D, for a query with
/// no extent in the triangle's plane).
fn seg_seg(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (f32, Vec3) {
    const EPS: f32 = 1e-9;
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (s, t) = if a <= EPS && e <= EPS {
        (0.0, 0.0)
    } else if a <= EPS {
        (0.0, (f / e).clamp(0.0, 1.0))
    } else {
        let c = d1.dot(r);
        if e <= EPS {
            ((-c / a).clamp(0.0, 1.0), 0.0)
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s = if denom > EPS {
                ((b * f - c * e) / denom).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let t = (b * s + f) / e;
            if t < 0.0 {
                ((-c / a).clamp(0.0, 1.0), 0.0)
            } else if t > 1.0 {
                (((b - c) / a).clamp(0.0, 1.0), 1.0)
            } else {
                (s, t)
            }
        }
    };
    let (c1, c2) = (p1 + d1 * s, p2 + d2 * t);
    ((c1 - c2).length_squared(), c2)
}

/// BTriLineCol (btricol.c): the query segment `from -> to`, swept with
/// `radius`, against triangle `t`, in world space (its body frame is a
/// rotation). No hit unless `from` is on or in front of the plane and the
/// query moves into it. A segment crossing the plane inside the triangle
/// hits there with distance 0; one crossing outside it hits the nearest
/// edge the crossing is outside of, within `radius`. One that stays in
/// front (within `radius` of the plane) hits with distance 0 where its far
/// end projects inside, else the nearest edge within `radius`. Gives the
/// squared distance and the hit point.
fn btri_line_col(t: &Tri, from: Vec3, to: Vec3, radius: f32) -> Option<(f32, Vec3)> {
    let n = t.n;
    let sb = (from - t.v[0]).dot(n);
    if sb < 0.0 {
        return None;
    }
    let sa = (to - t.v[0]).dot(n);
    if sb < sa {
        return None;
    }
    let wind = (t.v[1] - t.v[0]).cross(t.v[2] - t.v[0]).dot(n);
    let outside = |p: Vec3, k: usize| {
        let (a, b) = (t.v[k], t.v[(k + 1) % 3]);
        (b - a).cross(p - a).dot(n) * wind < 0.0
    };
    let edge = |k: usize| seg_seg(from, to, t.v[k], t.v[(k + 1) % 3]);
    let r2 = radius * radius;
    let nearest = |ks: &mut dyn Iterator<Item = usize>| {
        ks.map(edge)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .filter(|b| b.0 <= r2)
    };
    if sa <= 0.0 && sb + sa.abs() != 0.0 {
        // Crossing the plane.
        let x = from + (to - from) * (sb / (sb - sa));
        if !(0..3).any(|k| outside(x, k)) {
            return Some((0.0, x));
        }
        return nearest(&mut (0..3).filter(|&k| outside(x, k)));
    }
    if sa > radius {
        return None;
    }
    let pa = to - n * sa;
    let pb = from - n * sb;
    let inside = |p: Vec3| !(0..3).any(|k| outside(p, k));
    if sb == sa && inside(pb) {
        return Some((0.0, pb));
    }
    if inside(pa) {
        return Some((0.0, pa));
    }
    nearest(&mut (0..3))
}

impl Collision {
    /// FloorCollide (worldcol.c:389) -> WorldCollide: the floor triangles
    /// (normal.y 0.5 .. 2) the segment from `y_from` to `y_to` about `pos`,
    /// swept with `radius`, hits (BTriLineCol). The best hit is the one
    /// whose point is closest to the query's start; with `extra` (the 0x10
    /// bit, CTriListCollide) a hit off a triangle's edge scores 10000 times
    /// its squared distance from the edge instead, so a floor under the
    /// query beats one beside it.
    pub fn floor_collide(
        &self,
        pos: Vec3,
        radius: f32,
        y_from: f32,
        y_to: f32,
        extra: bool,
    ) -> Option<FloorHit> {
        let from = pos + Vec3::Y * y_from;
        let to = pos + Vec3::Y * y_to;
        let q = (to - from).normalize_or_zero();
        let (ylo, yhi) = (from.y.min(to.y) - radius, from.y.max(to.y) + radius);
        let mut best: Option<(f32, FloorHit)> = None;
        for i in self.cells_around(Vec2::new(pos.x, pos.z), radius) {
            let t = &self.tris[i as usize];
            if t.n.y < FLOOR_NY || t.n.y > FLOOR_NY_MAX {
                continue;
            }
            let tmin = t.v[0].y.min(t.v[1].y).min(t.v[2].y);
            let tmax = t.v[0].y.max(t.v[1].y).max(t.v[2].y);
            if tmax < ylo || tmin > yhi {
                continue;
            }
            let Some((d, at)) = btri_line_col(t, from, to, radius) else {
                continue;
            };
            let mut dd = if extra && d != 0.0 {
                10000.0 * d
            } else {
                (at - from).length_squared()
            };
            if q.dot(t.n).abs() < 0.25 {
                dd *= 0.95;
            }
            if best.is_none_or(|b| dd < b.0) {
                best = Some((
                    dd,
                    FloorHit {
                        at,
                        tri: i as usize,
                        owner: t.owner,
                    },
                ));
            }
        }
        best.map(|b| b.1)
    }

    /// PlayerCollideFloor's floor query (pmotion.c:4348): FloorCollide with
    /// the hero's radius from the body centre (effectpos, `height` =
    /// col_height above the feet) down 3 + col_height, to 3 below the feet.
    pub fn player_floor(
        &self,
        feet: Vec3,
        radius: f32,
        height: f32,
        extra: bool,
    ) -> Option<FloorHit> {
        self.floor_collide(feet + Vec3::Y * height, radius, 0.0, -(3.0 + height), extra)
    }

    /// PlayerCollideFloor (pmotion.c:4318) for a step `dpos` from `feet` by
    /// a hero on the floor (`floor_base`, the floor object `under`), with
    /// no wall touched: the step's new displacement and floor height, or
    /// None when it is refused.
    ///
    /// The floor under the step's end (see [`Collision::player_floor`]) must
    /// be within lbl_80347BF8 (3) of `floor_base`, 4 on the object already
    /// underfoot. A floor beside the end rather than under it (an edge
    /// within the radius, as across a seam between two floor objects)
    /// holds the hero if the floor goes on a radius further ahead, or if
    /// the edge is ahead of the end; otherwise the step is pulled back so
    /// as to end at the edge (a radius back from the end with no floor
    /// ahead), if there is floor there.
    pub fn player_collide_floor(
        &self,
        feet: Vec3,
        dpos: Vec3,
        floor_base: f32,
        under: Option<i32>,
        radius: f32,
        height: f32,
    ) -> Option<(Vec3, f32)> {
        let pos = feet + Vec3::Y * height;
        let zoff = -(3.0 + height);
        let mut end = pos + dpos;
        let hit = self.floor_collide(end, radius, 0.0, zoff, true)?;
        let fh = hit.at.y;
        let lim = 3.0 + if Some(hit.owner) == under { 1.0 } else { 0.0 };
        if (fh - floor_base).abs() > lim {
            return None;
        }
        let mut dx = end.x - hit.at.x;
        let mut dz = end.z - hit.at.z;
        let mut dq = dpos.y * (end.y - fh) + dpos.x * dx + dpos.z * dz;
        let mut d = dx.det_hypot(dz);
        if d >= 0.001 {
            let nrm = dpos.normalize_or_zero() * radius;
            end += nrm;
            match self.floor_collide(end, radius, 0.0, zoff, false) {
                Some(h) if self.obj_flags[h.tri] & 8 != 0 && (h.at.y - fh).abs() < 4.0 => d = 0.0,
                Some(h) => {
                    dx = end.x - h.at.x;
                    dz = end.z - h.at.z;
                    dq = dpos.y * (end.y - h.at.y) + dpos.x * dx + dpos.z * dz;
                    d = dx.det_hypot(dz);
                }
                None => {
                    d = radius;
                    dx = nrm.x;
                    dz = nrm.z;
                    dq = 0.0;
                }
            }
        }
        if d < 0.001 || dq < 0.0 {
            return Some((dpos, fh));
        }
        let back = dpos - Vec3::new(dx, 0.0, dz);
        self.floor_collide(pos + back, radius, 0.0, zoff, false)
            .map(|h| (back, h.at.y))
    }

    /// PlayerWallCollide as DoPlayerMotion runs it (pmotion.c:1195): a
    /// sphere of `radius` whose centre is `lift` above the feet (the body
    /// centre, col_height up, plus lbl_80347BD0 = 1), against the walls
    /// (normal.y within +-0.866), pushed out across the floor. One-sided:
    /// no hit from behind a wall's plane. Stairs' walls (see
    /// [`Collision::stairs`]) count only above `stair_step`. Gives the
    /// corrected feet and the last wall touched: its owner and the contact
    /// point.
    pub fn resolve_player_walls(
        &self,
        mut feet: Vec3,
        radius: f32,
        lift: f32,
        stair_step: f32,
    ) -> (Vec3, Option<(i32, Vec3)>) {
        let mut hit = None;
        for _ in 0..3 {
            let mut moved = false;
            let s = feet + Vec3::Y * lift;
            let cands: Vec<u32> = self
                .cells_around(Vec2::new(s.x, s.z), radius + 0.5)
                .collect();
            for i in cands {
                let t = &self.tris[i as usize];
                if t.n.y.abs() > WALL_NY {
                    continue;
                }
                let s = feet + Vec3::Y * lift;
                let tmax = t.v[0].y.max(t.v[1].y).max(t.v[2].y);
                let tmin = t.v[0].y.min(t.v[1].y).min(t.v[2].y);
                if tmax < s.y - radius || tmin > s.y + radius {
                    continue;
                }
                if self.stairs[i as usize] && tmax < feet.y + stair_step {
                    continue;
                }
                if (s - t.v[0]).dot(t.n) < 0.0 {
                    continue;
                }
                let c = closest_point_on_tri(s, t);
                let v = s - c;
                if v.length_squared() >= radius * radius {
                    continue;
                }
                let want = (radius * radius - v.y * v.y).max(0.0).sqrt();
                let h = Vec2::new(v.x, v.z);
                let len = h.length();
                let dir = if len > 1e-4 {
                    h / len
                } else {
                    Vec2::new(t.n.x, t.n.z).normalize_or_zero()
                };
                if dir == Vec2::ZERO {
                    continue;
                }
                let amt = want - len;
                if amt > 0.0 {
                    feet.x += dir.x * amt;
                    feet.z += dir.y * amt;
                    moved = true;
                    hit = Some((t.owner, c));
                }
            }
            if !moved {
                break;
            }
        }
        (feet, hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f32 = 1.5;
    const H: f32 = 2.5;

    /// A flat floor x0..x1, z0..z1 at height y (two triangles, wound
    /// counter-clockwise seen from above), owned by `owner`.
    fn quad(x0: f32, x1: f32, z0: f32, z1: f32, y: f32, owner: i32) -> [Tri; 2] {
        let v = |x, z| Vec3::new(x, y, z);
        let n = Vec3::Y;
        [
            Tri {
                v: [v(x0, z0), v(x0, z1), v(x1, z1)],
                n,
                owner,
            },
            Tri {
                v: [v(x0, z0), v(x1, z1), v(x1, z0)],
                n,
                owner,
            },
        ]
    }

    /// Two floors at y 0 with a seam between x 0 and x `gap`.
    fn seam(gap: f32) -> Collision {
        let mut t = quad(-20.0, 0.0, -10.0, 10.0, 0.0, 0).to_vec();
        t.extend(quad(gap, 20.0, -10.0, 10.0, 0.0, 1));
        Collision::new(t)
    }

    /// Walk a hero `n` ticks of 0.16 along +x from `feet` as Game::update
    /// does (PlayerCollideFloor, then the floor).
    fn walk(c: &Collision, mut feet: Vec3, n: usize) -> Vec3 {
        let mut vy = 0.0;
        for _ in 0..n {
            let under = c.player_floor(feet, R, H, true).map(|h| h.owner);
            if let Some((d, _)) = c.player_collide_floor(feet, Vec3::X * 0.16, feet.y, under, R, H)
            {
                feet += d;
            }
            vy -= 60.0 / 60.0;
            feet.y += vy / 60.0;
            match c.player_floor(feet, R, H, true) {
                Some(h) if feet.y <= h.at.y || feet.y - h.at.y < 0.3 => {
                    feet.y = h.at.y;
                    vy = 0.0;
                }
                _ => {}
            }
        }
        feet
    }

    #[test]
    fn a_floor_under_the_query_beats_an_edge_beside_it() {
        // A ledge at 1 ending at x 0, a floor at 0 beyond it.
        let mut t = quad(-20.0, 0.0, -10.0, 10.0, 1.0, 0).to_vec();
        t.extend(quad(0.0, 20.0, -10.0, 10.0, 0.0, 1));
        let c = Collision::new(t);
        // Just past the ledge's edge: the floor below, not the edge.
        let h = c
            .player_floor(Vec3::new(0.3, 1.0, 0.0), R, H, true)
            .unwrap();
        assert_eq!((h.owner, h.at.y), (1, 0.0));
        // Without the 0x10 scoring the nearer hit (the ledge's edge) wins.
        let h = c
            .player_floor(Vec3::new(0.3, 1.0, 0.0), R, H, false)
            .unwrap();
        assert_eq!(h.owner, 0);
        assert!((h.at - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn a_seam_between_floors_is_crossed() {
        // levelA6's start: the ramp ends at x -33.0, the floor starts at
        // -33.31; a point test finds nothing in between.
        let c = seam(0.31);
        assert!(c.ground_height(Vec3::new(0.15, 0.5, 0.0), 0.5).is_none());
        let h = c
            .player_floor(Vec3::new(0.15, 0.0, 0.0), R, H, true)
            .unwrap();
        assert_eq!(h.at.y, 0.0);
        let end = walk(&c, Vec3::new(-1.0, 0.0, 0.0), 30);
        assert!(end.x > 3.0 && end.y == 0.0, "crossed: {end}");
        // Half a unit too.
        let end = walk(&seam(0.5), Vec3::new(-1.0, 0.0, 0.0), 30);
        assert!(end.x > 3.0 && end.y == 0.0, "crossed: {end}");
    }

    #[test]
    fn a_pit_stops_a_walk_and_drops_a_hero_over_it() {
        // A pit 6 wide and deep (no floor within reach).
        let c = seam(6.0);
        // Walking at it: the hero stays on the near side.
        let end = walk(&c, Vec3::new(-3.0, 0.0, 0.0), 120);
        assert!(end.x < 1.5 && end.y == 0.0, "stopped at the edge: {end}");
        // Over its middle there is no floor: a hero there falls.
        assert!(c
            .player_floor(Vec3::new(3.0, 0.0, 0.0), R, H, true)
            .is_none());
        // Within a radius of its edge the edge holds him up.
        assert!(c
            .player_floor(Vec3::new(1.0, 0.0, 0.0), R, H, true)
            .is_some());
    }

    #[test]
    fn a_ledge_over_three_is_not_walked_off() {
        // Floor 3.5 below beyond x 0: found (down to 4.5) but too far down.
        let mut t = quad(-20.0, 0.0, -10.0, 10.0, 0.0, 0).to_vec();
        t.extend(quad(0.0, 20.0, -10.0, 10.0, -3.5, 1));
        let c = Collision::new(t);
        let end = walk(&c, Vec3::new(-2.0, 0.0, 0.0), 60);
        assert!(end.x < 0.5 && end.y == 0.0, "stopped: {end}");
        // A 2 drop is walked down.
        let mut t = quad(-20.0, 0.0, -10.0, 10.0, 0.0, 0).to_vec();
        t.extend(quad(0.0, 20.0, -10.0, 10.0, -2.0, 1));
        let c = Collision::new(t);
        let end = walk(&c, Vec3::new(-2.0, 0.0, 0.0), 120);
        assert!(end.x > 3.0 && end.y == -2.0, "walked down: {end}");
    }

    #[test]
    fn walls_up_to_two_are_stepped_over() {
        // A box 1.78 high (levelG1's lowered lift) and one 2.5 high.
        let riser = |h: f32| {
            let mut t = quad(-20.0, 20.0, -10.0, 10.0, 0.0, 0).to_vec();
            t.extend(quad(0.0, 20.0, -10.0, 10.0, h, 1));
            // Its face toward -x.
            let (a, b) = (Vec3::new(0.0, 0.0, -10.0), Vec3::new(0.0, h, 10.0));
            let n = -Vec3::X;
            t.push(Tri {
                v: [a, Vec3::new(0.0, h, -10.0), b],
                n,
                owner: 1,
            });
            t.push(Tri {
                v: [a, b, Vec3::new(0.0, 0.0, 10.0)],
                n,
                owner: 1,
            });
            Collision::new(t)
        };
        let lift = H + 1.0;
        let c = riser(1.78);
        let (p, wall) = c.resolve_player_walls(Vec3::new(-0.5, 0.0, 0.0), R, lift, 3.0);
        assert!(wall.is_none() && p.x == -0.5);
        let c = riser(2.5);
        let (p, wall) = c.resolve_player_walls(Vec3::new(-0.5, 0.0, 0.0), R, lift, 3.0);
        assert!(wall.is_some() && p.x < -0.5, "pushed back: {p}");
    }
}
