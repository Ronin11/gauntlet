use gdl_formats::{Mesh, Objects};
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let o = Objects::parse(&std::fs::read(p).unwrap()).unwrap();
    let mut hist = std::collections::BTreeMap::new();
    let (mut with_lm, mut without) = (0, 0);
    for obj in &o.objects {
        for (s, g) in obj.sub_objects.iter().zip(&obj.geometry) {
            let m = Mesh::decode(g).unwrap();
            if obj.flags & 0x20 != 0 {
                *hist.entry((s.lm_index, s.tex_index > 0)).or_insert(0) += 1;
                if m.lm_uvs.is_empty() {
                    without += 1
                } else {
                    with_lm += 1
                }
            }
        }
    }
    println!("lmap subobjects: with lm_uvs {with_lm}, without {without}");
    println!("{:?}", hist.iter().take(30).collect::<Vec<_>>());
    // sample lm uvs of first lmap object
    for obj in o.objects.iter().filter(|o| o.flags & 0x20 != 0).take(2) {
        let m = Mesh::decode(&obj.geometry[0]).unwrap();
        println!(
            "uvs {:?}\nlm {:?}",
            &m.uvs[..m.uvs.len().min(4)],
            &m.lm_uvs[..m.lm_uvs.len().min(4)]
        );
        let g = &obj.geometry[0];
        println!("{:02x?}", &g[..g.len().min(96)]);
    }
}
