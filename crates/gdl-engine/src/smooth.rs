//! Rounding off the world: the level geometry is a few thousand large flat
//! triangles, so slopes and mounds show their facets. Curved PN triangles
//! (Vlachos et al.) bend each triangle toward the smooth normals of its
//! corners, and subdividing them shows the curve. Only corners whose faces
//! all meet gently are smoothed, so walls, steps and box edges stay sharp,
//! and a shared edge gets the same curve from both its faces (no cracks).

use gdl_sim::mesh::{MeshData, Vertex};
use glam::Vec3;
use std::collections::HashMap;

/// `GDL_SMOOTH=<levels>[,<crease degrees>]`: subdivisions of each world
/// triangle (`2` makes 16 of each; off unless set), and the angle between
/// faces above which an edge stays a crease (default 80). Off by default:
/// the levels light their world by vertex colours, so rounding the corners
/// only changes silhouettes, and those are mostly creases anyway.
pub fn settings() -> (u32, f32) {
    let s = std::env::var("GDL_SMOOTH").unwrap_or_default();
    let mut it = s.split(',').map(|x| x.trim().parse::<f32>().ok());
    let levels = it.next().flatten().map_or(0, |n| n as u32);
    let crease = it.next().flatten().unwrap_or(80.0);
    (levels, crease)
}

fn key(p: Vec3) -> (i32, i32, i32) {
    let q = |x: f32| (x * 64.0).round() as i32;
    (q(p.x), q(p.y), q(p.z))
}

/// Subdivide the opaque batches of `mesh` `levels` times along PN triangles.
pub fn smooth_world(mesh: &mut MeshData, (levels, crease_deg): (u32, f32)) {
    if levels == 0 || mesh.indices.is_empty() {
        return;
    }
    let pos = |i: u32| Vec3::from(mesh.vertices[i as usize].pos);
    // Face normals, and each position's faces (across every batch, so an
    // edge shared by two objects is seen from both sides).
    let eligible = |b: &gdl_sim::mesh::Batch| !b.blend && !b.additive && b.count >= 3;
    let mut face_normal: HashMap<u32, Vec3> = HashMap::new();
    let mut at: HashMap<(i32, i32, i32), Vec<u32>> = HashMap::new();
    for b in mesh.batches.iter().filter(|b| eligible(b)) {
        for t in (b.start..b.start + b.count).step_by(3) {
            let (a, c, d) = (
                pos(mesh.indices[t as usize]),
                pos(mesh.indices[t as usize + 1]),
                pos(mesh.indices[t as usize + 2]),
            );
            let n = (c - a).cross(d - a).normalize_or_zero();
            face_normal.insert(t, n);
            for k in 0..3 {
                at.entry(key(pos(mesh.indices[t as usize + k])))
                    .or_default()
                    .push(t);
            }
        }
    }
    // A corner is soft when every pair of its faces meets within the crease
    // angle; its smooth normal is their mean.
    let cos_crease = crease_deg.to_radians().cos();
    let mut smooth: HashMap<(i32, i32, i32), Vec3> = HashMap::new();
    for (k, faces) in &at {
        let ns: Vec<Vec3> = faces.iter().filter_map(|t| face_normal.get(t)).copied().collect();
        let soft = ns.len() > 1
            && ns
                .iter()
                .enumerate()
                .all(|(i, a)| ns[i + 1..].iter().all(|b| a.dot(*b) >= cos_crease));
        if soft {
            let n = ns.iter().sum::<Vec3>().normalize_or_zero();
            if n != Vec3::ZERO {
                smooth.insert(*k, n);
            }
        }
    }
    let n_sub = 1u32 << levels;
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut batches = Vec::new();
    for b in &mesh.batches {
        let start = indices.len() as u32;
        if !eligible(b) {
            let base = vertices.len() as u32;
            let used: Vec<u32> = mesh.indices[b.start as usize..(b.start + b.count) as usize].to_vec();
            let mut remap: HashMap<u32, u32> = HashMap::new();
            for &i in &used {
                let n = remap.len() as u32;
                let j = *remap.entry(i).or_insert(n);
                if j == n {
                    vertices.push(mesh.vertices[i as usize]);
                }
                indices.push(base + j);
            }
            batches.push(gdl_sim::mesh::Batch {
                start,
                count: indices.len() as u32 - start,
                ..b.clone()
            });
            continue;
        }
        for t in (b.start..b.start + b.count).step_by(3) {
            let v: [Vertex; 3] = std::array::from_fn(|k| mesh.vertices[mesh.indices[t as usize + k] as usize]);
            let p: [Vec3; 3] = std::array::from_fn(|k| {
                // Snap to the position group so shared edges match bit for bit.
                let q = key(Vec3::from(v[k].pos));
                Vec3::new(q.0 as f32, q.1 as f32, q.2 as f32) / 64.0
            });
            let soft: [Option<Vec3>; 3] = std::array::from_fn(|k| smooth.get(&key(p[k])).copied());
            // PN edge control points: a soft corner pulls its two edge points
            // off the chord along its normal; a hard one leaves them on it.
            let edge = |i: usize, j: usize| -> Vec3 {
                match soft[i] {
                    Some(n) => (2.0 * p[i] + p[j] - (p[j] - p[i]).dot(n) * n) / 3.0,
                    None => (2.0 * p[i] + p[j]) / 3.0,
                }
            };
            let (b210, b120) = (edge(0, 1), edge(1, 0));
            let (b021, b012) = (edge(1, 2), edge(2, 1));
            let (b102, b201) = (edge(2, 0), edge(0, 2));
            let e = (b210 + b120 + b021 + b012 + b102 + b201) / 6.0;
            let centre = (p[0] + p[1] + p[2]) / 3.0;
            let b111 = e + (e - centre) / 2.0;
            let normal: [Vec3; 3] = std::array::from_fn(|k| soft[k].unwrap_or(Vec3::from(v[k].normal)));
            let base = vertices.len() as u32;
            // Grid of (n+1)(n+2)/2 vertices over the triangle.
            let mut row_start = Vec::new();
            for i in 0..=n_sub {
                row_start.push(vertices.len() as u32 - base);
                for j in 0..=n_sub - i {
                    let (u, w) = (i as f32 / n_sub as f32, j as f32 / n_sub as f32);
                    let vv = 1.0 - u - w;
                    // Bernstein weights over (vv, u, w) for corners p0, p1, p2.
                    let position = p[0] * (vv * vv * vv)
                        + p[1] * (u * u * u)
                        + p[2] * (w * w * w)
                        + b210 * (3.0 * vv * vv * u)
                        + b120 * (3.0 * vv * u * u)
                        + b201 * (3.0 * vv * vv * w)
                        + b021 * (3.0 * u * u * w)
                        + b102 * (3.0 * vv * w * w)
                        + b012 * (3.0 * u * w * w)
                        + b111 * (6.0 * vv * u * w);
                    let mix2 = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| -> [f32; 2] {
                        [a[0] * vv + b[0] * u + c[0] * w, a[1] * vv + b[1] * u + c[1] * w]
                    };
                    let normal = (normal[0] * vv + normal[1] * u + normal[2] * w).normalize_or_zero();
                    vertices.push(Vertex {
                        pos: (position * 64.0).round().to_array().map(|x| x / 64.0),
                        normal: normal.to_array(),
                        color: std::array::from_fn(|k| v[0].color[k] * vv + v[1].color[k] * u + v[2].color[k] * w),
                        uv: mix2(v[0].uv, v[1].uv, v[2].uv),
                        lm_uv: mix2(v[0].lm_uv, v[1].lm_uv, v[2].lm_uv),
                    });
                }
            }
            for i in 0..n_sub {
                for j in 0..n_sub - i {
                    let a = base + row_start[i as usize] + j;
                    let c = base + row_start[i as usize + 1] + j;
                    indices.extend([a, c, a + 1]);
                    if j + 1 < n_sub - i {
                        indices.extend([a + 1, c, c + 1]);
                    }
                }
            }
        }
        batches.push(gdl_sim::mesh::Batch {
            start,
            count: indices.len() as u32 - start,
            ..b.clone()
        });
    }
    log::info!(
        "smooth: {} -> {} triangles, {} soft corners of {}",
        mesh.indices.len() / 3,
        indices.len() / 3,
        smooth.len(),
        at.len()
    );
    mesh.vertices = vertices;
    mesh.indices = indices;
    mesh.batches = batches;
}
