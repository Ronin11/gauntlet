use gdl_formats::audio::{parse_sound_names, Vbk, BANKS};
fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    let names = parse_sound_names(&std::fs::read(root.join("AUDATPS2.ROM")).unwrap());
    println!("{} sound names", names.len());
    for n in [
        "S_NEEDSFOOD",
        "S_BADLY",
        "S_REDWAR1",
        "S_PICKUPKEY",
        "S_WEAPONHITMETA",
        "S_GRUNT1DIECLOS",
        "S_WARPAIN1",
        "S_CHEST",
    ] {
        let Some(&(bank, idx, dur)) = names.get(n) else {
            println!("{n}: missing");
            continue;
        };
        let file = root.join(format!("{}.VBK", BANKS[bank]));
        let b = std::fs::read(&file).unwrap();
        let v = Vbk::parse(&b).unwrap();
        let def = &v.sounds[idx];
        println!("  def {:?} samples parsed {}", def, v.samples.len());
        let Some((pcm, rate)) = def.samples.first().and_then(|&s| v.decode_sample(&b, s)) else {
            continue;
        };
        println!("{n}: bank {} idx {idx} rom {dur:.2}s -> samples {:?} vol {} decoded {:.2}s @{rate} ({} sounds, {} samples in bank)",
            BANKS[bank], def.samples, def.volume, pcm.len() as f32 / rate as f32, v.sounds.len(), v.samples.len());
    }
}
