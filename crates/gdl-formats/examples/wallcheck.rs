use gdl_formats::Worlds;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let w = Worlds::parse(&std::fs::read(&a[1]).unwrap()).unwrap();
    let (x, y, z): (f32, f32, f32) = (
        a[2].parse().unwrap(),
        a[3].parse().unwrap(),
        a[4].parse().unwrap(),
    );
    // owners
    let mut owner = vec![String::new(); w.coll_tris.len()];
    for o in &w.world_objects {
        if o.coll_tri_count > 0 && o.coll_tri_index >= 0 {
            for t in
                o.coll_tri_index as usize..(o.coll_tri_index + o.coll_tri_count as i32) as usize
            {
                if t < owner.len() {
                    owner[t] = o.name.clone();
                }
            }
        }
    }
    for it in &w.item_instances {
        if it.coll_tri_count > 0 && it.coll_tri_index >= 0 {
            for t in it.coll_tri_index as usize..(it.coll_tri_index + it.coll_tri_count) as usize {
                if t < owner.len() {
                    owner[t] = format!(
                        "ITEM:{} {}",
                        it.name,
                        w.item_infos
                            .get(it.item_index as usize)
                            .map(|i| i.name.as_str())
                            .unwrap_or("")
                    );
                }
            }
        }
    }
    for (i, t) in w.coll_tris.iter().enumerate() {
        if t.norm[1] >= 0.5 {
            continue;
        }
        let c = [
            (t.verts[0][0] + t.verts[1][0] + t.verts[2][0]) / 3.0,
            (t.verts[0][1] + t.verts[1][1] + t.verts[2][1]) / 3.0,
            (t.verts[0][2] + t.verts[1][2] + t.verts[2][2]) / 3.0,
        ];
        let d = ((c[0] - x).powi(2) + (c[2] - z).powi(2)).sqrt();
        if d < 6.0 && (c[1] - y).abs() < 8.0 {
            println!(
                "tri {i} owner {:?} c {:?} n {:?} verts {:?}",
                owner[i], c, t.norm, t.verts
            );
        }
    }
}
