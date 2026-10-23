//! World collision: ground height queries and cylinder-vs-wall resolution against
//! the level's collision triangles (which are stored in world space).

use glam::{Vec2, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Tri {
    pub v: [Vec3; 3],
    pub n: Vec3,
    /// Owning world object (for switching moving/destroyable geometry on and off).
    pub owner: i32,
}

pub struct Collision {
    pub tris: Vec<Tri>,
    pub enabled: Vec<bool>,
    /// The triangle's world object is stairs or a ramp (flags & 0x38): its
    /// risers don't stop a walking player (fn_80088714), who climbs them by
    /// the floor (PlayerCollideFloor).
    pub stairs: Vec<bool>,
    min: Vec2,
    cell: f32,
    nx: usize,
    nz: usize,
    grid: Vec<Vec<u32>>,
    /// Triangles of moving world objects, kept out of the grid
    /// (WorldDynCollide): per group, an XZ bounding circle and the triangles.
    dynamic: Vec<DynGroup>,
}

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
        Collision {
            tris,
            enabled,
            stairs,
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
        let moving: std::collections::HashSet<u32> =
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
        let d = (to - from).normalize_or_zero();
        let (t, n) = self.raycast_hit(from, to + d * r)?;
        let len = (to - from).length() + r;
        Some((from + d * (t * len - r).max(0.0), n))
    }

    /// raycast, also giving the hit triangle's normal.
    pub fn raycast_hit(&self, from: Vec3, to: Vec3) -> Option<(f32, Vec3)> {
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
        let mut best: Option<(f32, Vec3)> = None;
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
                best = Some((tt, t.n));
            }
        }
        best
    }
}
