use gdl_formats::Worlds;
use std::collections::BTreeMap;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let w = Worlds::parse(&std::fs::read(p).unwrap()).unwrap();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for it in &w.item_instances {
        let Some(info) = w.item_infos.get(it.item_index as usize) else {
            continue;
        };
        let key = format!("{:?}/{}/{}", info.item_type, info.subtype_name(), info.name);
        *counts.entry(key).or_default() += 1;
        if matches!(
            info.item_type,
            gdl_formats::worlds::ItemType::Generator | gdl_formats::worlds::ItemType::Enemy
        ) {
            let s = |k: usize| i16::from_le_bytes([it.params[k], it.params[k + 1]]);
            println!(
                "{:?} {} at {:?} params strength {} ai {} {} {} minp {} flags {}",
                info.item_type,
                info.name,
                it.pos,
                s(0),
                s(2),
                s(4),
                s(6),
                it.min_players,
                it.flags
            );
        }
    }
    for (k, v) in counts {
        println!("{v:4} {k}");
    }
}
