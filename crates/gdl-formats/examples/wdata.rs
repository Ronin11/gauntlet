//! Print every realm's level table: `cargo run --example wdata -- <files/Gauntlet>`.
use gdl_formats::wdata::Worlds;
fn main() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("path to files/Gauntlet"));
    let w = Worlds::load(|p| Ok(std::fs::read(root.join(p))?))?;
    for r in &w.realms {
        for (i, l) in r.levels.iter().enumerate() {
            println!(
                "{:#05x} {:3} {:3} {:10} movie={:10} boss={} wavetime={} map={} shop={:?}",
                (r.realm_type << 8) | i as u32,
                l.name,
                l.prep,
                l.title,
                l.movie,
                l.boss,
                l.wavetime,
                l.map_index,
                l.shop_max
            );
        }
    }
    for r in &w.realms {
        for i in 0..r.levels.len() {
            let wave = (r.realm_type << 8) | i as u32;
            println!("{wave:#05x} music {:?}", w.music(wave));
        }
    }
    println!("a2 -> {:x?}", w.find_wave("a2"));
    Ok(())
}
