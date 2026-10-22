use gdl_formats::Worlds;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let w = Worlds::parse(&std::fs::read(p).unwrap()).unwrap();
    let pos = w.world_positions();
    let start = w
        .locators
        .iter()
        .find(|l| format!("{:?}", l.kind) == "Start")
        .map(|l| l.pos)
        .unwrap();
    println!("start {:?}", start);
    let mut owned = vec![false; w.coll_tris.len()];
    let mut n_obj = 0;
    for (i, o) in w.world_objects.iter().enumerate() {
        if o.coll_tri_count > 0 && o.coll_tri_index >= 0 {
            n_obj += 1;
            for t in
                o.coll_tri_index as usize..(o.coll_tri_index as usize + o.coll_tri_count as usize)
            {
                if t < owned.len() {
                    owned[t] = true;
                }
            }
            if n_obj < 6 {
                println!(
                    "obj {} {} pos {:?} tris {}@{}",
                    i, o.name, pos[i], o.coll_tri_count, o.coll_tri_index
                );
            }
        }
    }
    println!(
        "objects with coll: {n_obj}, tris owned {} / {}",
        owned.iter().filter(|x| **x).count(),
        owned.len()
    );
    // ground test: tris whose xz contains start (world-space interpretation)
    let inside = |t: &[[f32; 3]; 3], x: f32, z: f32| {
        let s = |a: [f32; 3], b: [f32; 3]| (b[0] - a[0]) * (z - a[2]) - (b[2] - a[2]) * (x - a[0]);
        let (d1, d2, d3) = (s(t[0], t[1]), s(t[1], t[2]), s(t[2], t[0]));
        (d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0) || (d1 <= 0.0 && d2 <= 0.0 && d3 <= 0.0)
    };
    for (k, t) in w.coll_tris.iter().enumerate() {
        if t.norm[1] > 0.5 && inside(&t.verts, start[0], start[2]) {
            println!(
                "world-space hit tri {k} y {:?} norm {:?}",
                t.verts.map(|v| v[1]),
                t.norm
            );
        }
    }
    for (i, o) in w.world_objects.iter().enumerate() {
        if o.coll_tri_count <= 0 {
            continue;
        }
        for k in o.coll_tri_index as usize..(o.coll_tri_index as usize + o.coll_tri_count as usize)
        {
            let t = &w.coll_tris[k];
            let v = t
                .verts
                .map(|v| [v[0] + pos[i][0], v[1] + pos[i][1], v[2] + pos[i][2]]);
            if t.norm[1] > 0.5 && inside(&v, start[0], start[2]) {
                println!(
                    "obj-relative hit obj {} tri {k} y {:?}",
                    o.name,
                    v.map(|v| v[1])
                );
            }
        }
    }
}
