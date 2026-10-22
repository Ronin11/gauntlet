//! Dump trigger items (levers, pads) and what they might link to.
use gdl_formats::Worlds;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let w = Worlds::parse(&std::fs::read(p).unwrap()).unwrap();
    for (i, it) in w.item_instances.iter().enumerate() {
        let Some(info) = w.item_infos.get(it.item_index as usize) else {
            continue;
        };
        let t = format!("{:?}", info.item_type);
        if t.contains("Trigger")
            || std::env::args()
                .nth(2)
                .is_some_and(|f| t.contains(&f) || info.subtype_name().contains(&f))
        {
            println!(
                "#{i:3} {t}/{} {} inst {:16} pos {:?} rot {:?} flags {:#x} coll {}@{} params {:02x?}",
                info.subtype_name(),
                info.name,
                it.name,
                it.pos,
                it.rot,
                it.flags,
                it.coll_tri_count,
                it.coll_tri_index,
                it.params
            );
            let t = i16::from_le_bytes([it.params[0], it.params[1]]);
            let mut stack: Vec<(i16, usize)> = vec![(t, 1)];
            while let Some((idx, depth)) = stack.pop() {
                let Some(o) = (idx >= 0)
                    .then(|| w.world_objects.get(idx as usize))
                    .flatten()
                else {
                    continue;
                };
                println!(
                    "{:w$}wobj {idx} {:16} flags {:#010x} mb {:#010x} pos {:?} ctris {} rad {}",
                    "",
                    o.name,
                    o.flags,
                    o.mb_flags,
                    o.pos,
                    o.coll_tri_count,
                    o.radius,
                    w = depth * 2
                );
                if depth < 4 {
                    let mut c = o.child_index;
                    while c >= 0 {
                        stack.push((c, depth + 1));
                        c = w.world_objects[c as usize].next_index;
                    }
                }
            }
        }
    }
}
