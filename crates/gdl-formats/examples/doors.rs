use gdl_formats::Worlds;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let w = Worlds::parse(&std::fs::read(p).unwrap()).unwrap();
    for it in &w.item_instances {
        let Some(info) = w.item_infos.get(it.item_index as usize) else { continue };
        if matches!(info.item_type, gdl_formats::worlds::ItemType::Door) || info.subtype_name() == "wall" {
            println!("{:?} {} inst {:?} pos {:?} rot {:?} coll {}@{} minp {} radius {} xdim {} zdim {} colltype {}", info.item_type, info.name, it.name, it.pos, it.rot, it.coll_tri_count, it.coll_tri_index, it.min_players, info.radius, info.x_dim, info.z_dim, info.coll_type);
        }
    }
}
