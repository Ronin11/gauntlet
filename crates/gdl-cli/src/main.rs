use anyhow::Result;
use clap::{Parser, Subcommand};
use gcdisc::Disc;
use gdl_formats::{texture, Mesh, Objects};
use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "gdl", about = "Gauntlet Dark Legacy asset tools")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print disc header and list files
    Ls { iso: PathBuf },
    /// Extract every file (and main.dol) into a directory
    Extract { iso: PathBuf, out: PathBuf },
    /// Summarise an objects.ngc file (objects, sub-objects, bitmaps)
    Objects { path: PathBuf },
    /// Decode every texture of a model directory (objects.ngc + textures.ngc) to PNGs
    Textures { dir: PathBuf, out: PathBuf },
    /// Decode every texture to `<out>/tex/<content-hash>.png`, the layout an
    /// override directory (`GDL_OVERRIDE`) is read back from, plus a manifest
    Hashed { dir: PathBuf, out: PathBuf },
    /// Write every object of a model directory as `<out>/parts/<NAME>.obj`
    /// (object space, textures by content hash in `<out>/tex/`), the starting
    /// point for a rebuilt part that the override directory loads back as
    /// `parts/<NAME>.glb`
    Parts { dir: PathBuf, out: PathBuf },
    /// Survey every objects.ngc under a directory: formats used, decode failures
    Survey { root: PathBuf },
    /// Summarise a WORLDS.PS2 level file
    World { path: PathBuf },
    /// Summarise an ANIM.PS2 file
    Anim { path: PathBuf },
    /// Read values from main.dol at virtual addresses (e.g. 0x80347B08:f32 0x80118100:u32x9)
    Dol { dol: PathBuf, addrs: Vec<String> },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Ls { iso } => {
            let disc = Disc::open(&iso)?;
            let h = &disc.header;
            println!(
                "{} \"{}\" dol@{:#x} fst@{:#x}+{:#x}",
                h.game_id, h.title, h.dol_offset, h.fst_offset, h.fst_size
            );
            for e in &disc.entries {
                if e.is_dir {
                    println!("{:>10}  {}/", "", e.path);
                } else {
                    println!("{:>10}  {}", e.size, e.path);
                }
            }
        }
        Cmd::Extract { iso, out } => {
            let mut disc = Disc::open(&iso)?;
            std::fs::create_dir_all(&out)?;
            std::fs::write(out.join("main.dol"), disc.read_dol()?)?;
            let files: Vec<_> = disc.entries.iter().filter(|e| !e.is_dir).cloned().collect();
            for e in &files {
                let dest = out.join("files").join(&e.path);
                std::fs::create_dir_all(dest.parent().unwrap())?;
                std::fs::write(&dest, disc.read_at(e.offset, e.size as usize)?)?;
            }
            println!("extracted {} files to {}", files.len(), out.display());
        }
        Cmd::Objects { path } => {
            let objs = Objects::parse(&std::fs::read(&path)?)?;
            println!(
                "{:?} {:?} v{:#x}",
                objs.dir_name, objs.model_name, objs.version
            );
            println!(
                "{} objects, {} defs, {} bitmaps, {} bitmap defs",
                objs.objects.len(),
                objs.object_defs.len(),
                objs.bitmaps.len(),
                objs.bitmap_defs.len()
            );
            for d in &objs.object_defs {
                println!(
                    "def {:16} idx {:4} frames {:3} rad {:.2}",
                    d.name, d.obj_index, d.frames, d.bnd_rad
                );
            }
            for (i, o) in objs.objects.iter().enumerate() {
                let mut verts = 0;
                let mut tris = 0;
                for g in &o.geometry {
                    match Mesh::decode(g) {
                        Ok(m) => {
                            verts += m.positions.len();
                            tris += m.indices.len() / 3;
                        }
                        Err(e) => println!("  obj {i}: decode error: {e:#}"),
                    }
                }
                println!(
                    "obj {i:4} flags {:#07x} subs {:2} hdr v/t {}/{} decoded v/t {verts}/{tris}",
                    o.flags,
                    o.sub_objects.len(),
                    o.vert_count,
                    o.tri_count
                );
            }
            for (i, b) in objs.bitmaps.iter().enumerate() {
                println!(
                    "bmp {i:4} fmt {:3} {}x{} mips {} flags {:#06x} ptr {:#x} frames {}",
                    b.format,
                    b.width,
                    b.height,
                    b.mipmap_count,
                    b.flags,
                    b.tex_pointer,
                    b.frame_count
                );
            }
            for d in &objs.bitmap_defs {
                println!(
                    "bdef {:30} idx {} {}x{}",
                    d.name, d.tex_index, d.width, d.height
                );
            }
        }
        Cmd::Textures { dir, out } => {
            let objs = Objects::parse(&std::fs::read(dir.join("objects.ngc"))?)?;
            let tex = std::fs::read(dir.join("textures.ngc"))?;
            std::fs::create_dir_all(&out)?;
            let names: BTreeMap<u16, &str> = objs
                .bitmap_defs
                .iter()
                .map(|d| (d.tex_index, d.name.as_str()))
                .collect();
            for (i, b) in objs.bitmaps.iter().enumerate() {
                match texture::decode_ngc(b, &tex) {
                    Ok(img) => {
                        let name = names.get(&(i as u16)).copied().unwrap_or("");
                        write_png(&out.join(format!("{i:04}_{name}_f{}.png", b.format)), &img)?;
                    }
                    Err(e) => eprintln!("bitmap {i}: {e:#}"),
                }
            }
        }
        Cmd::Parts { dir, out } => export_parts(&dir, &out)?,
        Cmd::Hashed { dir, out } => {
            let objs = Objects::parse(&std::fs::read(dir.join("objects.ngc"))?)?;
            let tex = std::fs::read(dir.join("textures.ngc"))?;
            let tex_dir = out.join("tex");
            std::fs::create_dir_all(&tex_dir)?;
            let names: BTreeMap<u16, &str> = objs
                .bitmap_defs
                .iter()
                .map(|d| (d.tex_index, d.name.as_str()))
                .collect();
            let mut manifest = String::new();
            for (i, b) in objs.bitmaps.iter().enumerate() {
                if b.flags & gdl_formats::objects::bitmap_flags::EXTERNAL != 0 || b.frame_count > 0
                {
                    continue;
                }
                match texture::decode_original(b, &tex) {
                    Ok(img) => {
                        let hash = img.content_hash();
                        let name = names.get(&(i as u16)).copied().unwrap_or("");
                        manifest.push_str(&format!(
                            "{hash} {i:04} {:>3}x{:<3} f{:<3} {name}\n",
                            img.width, img.height, b.format
                        ));
                        let path = tex_dir.join(format!("{hash}.png"));
                        if !path.exists() {
                            write_png(&path, &img)?;
                        }
                    }
                    Err(e) => eprintln!("bitmap {i}: {e:#}"),
                }
            }
            std::fs::write(out.join("manifest.txt"), manifest)?;
        }
        Cmd::Survey { root } => survey(&root)?,
        Cmd::Dol { dol, addrs } => {
            let d = std::fs::read(&dol)?;
            let be = |o: usize| u32::from_be_bytes(d[o..o + 4].try_into().unwrap());
            let to_off = |addr: u32| -> Option<usize> {
                (0..18).find_map(|i| {
                    let (off, a, sz) = (be(i * 4), be(0x48 + i * 4), be(0x90 + i * 4));
                    (addr >= a && addr < a + sz).then(|| (off + addr - a) as usize)
                })
            };
            for spec in addrs {
                let (a, ty) = spec.split_once(':').unwrap_or((spec.as_str(), "f32"));
                let (ty, n) = ty
                    .split_once('x')
                    .map(|(t, n)| (t, n.parse().unwrap_or(1)))
                    .unwrap_or((ty, 1usize));
                let addr =
                    u32::from_str_radix(a.trim_start_matches("0x").trim_start_matches("lbl_"), 16)?;
                let Some(off) = to_off(addr) else {
                    println!("{a}: not in any DOL section (bss?)");
                    continue;
                };
                let vals: Vec<String> = (0..n)
                    .map(|k| match ty {
                        "f64" => format!(
                            "{}",
                            f64::from_be_bytes(d[off + k * 8..off + k * 8 + 8].try_into().unwrap())
                        ),
                        "u32" => format!("{:#x}", be(off + k * 4)),
                        "i32" => format!("{}", be(off + k * 4) as i32),
                        "str" => {
                            let e = d[off..].iter().position(|&c| c == 0).unwrap_or(64);
                            String::from_utf8_lossy(&d[off..off + e]).into_owned()
                        }
                        _ => format!("{}", f32::from_bits(be(off + k * 4))),
                    })
                    .collect();
                println!("{a} {ty}: {}", vals.join(", "));
            }
        }
        Cmd::Anim { path } => {
            let a = gdl_formats::anim::Anim::parse(&std::fs::read(&path)?)?;
            println!(
                "version {} atrees {} texmods {}",
                a.version,
                a.atrees.len(),
                a.texmods.len()
            );
            let tm_line = |t: &gdl_formats::anim::TexMod| {
                format!(
                    "atree {:3} seq {:3} {:20} <- {:20} tex {:4} src {:4} frames {:5} start {:3} rate {:2} frame {}",
                    t.atree, t.seq, t.name, t.source_name, t.tex, t.source, t.frames,
                    t.start_frame, t.rate, t.frame
                )
            };
            for t in &a.texmods {
                println!("texmod {}", tm_line(t));
            }
            for t in &a.atrees {
                println!(
                    "atree {} prefix {:?}: {} nodes, {} seqs",
                    t.name,
                    t.prefix,
                    t.nodes.len(),
                    t.sequences.len()
                );
                for (i, n) in t.nodes.iter().enumerate() {
                    let ntracks = t.tracks[i].iter().filter(|x| x.is_some()).count();
                    println!(
                        "  node {i:2} {:20} type {} parent {:3} pos {:?} mb {:#x} tracks {ntracks}",
                        n.name, n.anim_type, n.parent, n.init_pos, n.mb_flags
                    );
                    if let Some(tm) = &n.texmod {
                        println!("    node texmod {}", tm_line(tm));
                    }
                    if let Some(first) = n.obj_anim_first {
                        for (si, oa) in t
                            .obj_anims
                            .iter()
                            .skip(first)
                            .take(t.sequences.len())
                            .enumerate()
                        {
                            println!(
                                "    node oanim seq {si} {:?} frames {} start {}",
                                oa.name, oa.frame_count, oa.start_frame
                            );
                        }
                    }
                    if let Some((k, dir)) = n.psys {
                        println!("    node psys {k} dir {dir:?} {:?}", a.psys.get(k));
                    }
                }
                for s in &t.sequences {
                    println!(
                        "  seq {:20} frames {:4} rate {:3} repeat {} flags {:#x}",
                        s.name, s.frame_count, s.frame_rate, s.repeat, s.flags
                    );
                    for tm in &s.texmods {
                        println!("    seq texmod {}", tm_line(tm));
                    }
                }
                if let Some(si) = t.sequence_index("WALK1").or(Some(0)) {
                    for (i, tr) in t.tracks.iter().enumerate().take(6) {
                        if let Some(Some(tr)) = tr.get(si) {
                            println!(
                                "  seq{si} node{i} flags {:#x} keys {} first {:?} last {:?}",
                                tr.flags,
                                tr.keys.len(),
                                tr.keys.first(),
                                tr.keys.last()
                            );
                        }
                    }
                }
            }
        }
        Cmd::World { path } => {
            let w = gdl_formats::Worlds::parse(&std::fs::read(&path)?)?;
            println!(
                "bounds {:?}..{:?} grid {} {}x{}",
                w.min_bounds, w.max_bounds, w.grid_size, w.grid_number_x, w.grid_number_z
            );
            println!(
                "{} world objects, {} coll tris, {} item infos, {} instances, {} locators",
                w.world_objects.len(),
                w.coll_tris.len(),
                w.item_infos.len(),
                w.item_instances.len(),
                w.locators.len()
            );
            let pos = w.world_positions();
            // GDL_WO_ALL=1 lists every world object (default: first 40).
            let wo_limit = if std::env::var_os("GDL_WO_ALL").is_some() {
                usize::MAX
            } else {
                40
            };
            for (o, p) in w.world_objects.iter().zip(&pos).take(wo_limit) {
                println!(
                    "wo {:16} flags {:#010x} pos {:?} abs {:?} child {} next {} tris {}@{}",
                    o.name,
                    o.flags,
                    o.pos,
                    p,
                    o.child_index,
                    o.next_index,
                    o.coll_tri_count,
                    o.coll_tri_index
                );
            }
            for t in w.coll_tris.iter().take(5) {
                println!("tri {:?}", t);
            }
            for (i, it) in w.item_infos.iter().enumerate() {
                println!(
                    "info {i:3} {:?}/{}({}) {:16} rad {} h {} hp {} armor {} val {} props {:#x} coll {} active {:#x} rnd {:?}",
                    it.item_type,
                    it.subtype_name(),
                    it.subtype,
                    it.name,
                    it.radius,
                    it.height,
                    it.hit_points,
                    it.armor,
                    it.value,
                    it.properties,
                    it.coll_type,
                    it.active_type,
                    it.random_indices
                );
            }
            for it in w.item_instances.iter() {
                println!(
                    "inst {:3} {:16} params {:?} pos {:?} rot {:?} minp {} flags {} tris {}@{}",
                    it.item_index,
                    it.name,
                    it.params,
                    it.pos,
                    it.rot,
                    it.min_players,
                    it.flags,
                    it.coll_tri_count,
                    it.coll_tri_index
                );
            }
            for l in &w.locators {
                println!("loc {:?} {:?} {:?} next {}", l.kind, l.pos, l.rot, l.next);
            }
        }
    }
    Ok(())
}

/// `gdl parts`: each named object as a Wavefront OBJ in its own object space
/// (the frame its skeleton node drives), with normals, UVs and one material per
/// texture, plus `parts.txt` listing name, triangles, bounds and textures.
fn export_parts(dir: &Path, out: &Path) -> Result<()> {
    use std::fmt::Write as _;
    let objs = Objects::parse(&std::fs::read(dir.join("objects.ngc"))?)?;
    let tex = std::fs::read(dir.join("textures.ngc"))?;
    let parts_dir = out.join("parts");
    let tex_dir = out.join("tex");
    std::fs::create_dir_all(&parts_dir)?;
    std::fs::create_dir_all(&tex_dir)?;
    // Texture slot -> content hash (its file name), decoded once.
    let mut hashes: BTreeMap<u16, String> = BTreeMap::new();
    let mut hash_of = |slot: u16| -> Option<String> {
        if let Some(h) = hashes.get(&slot) {
            return Some(h.clone());
        }
        let b = objs.bitmaps.get(slot as usize)?;
        let img = texture::decode_ngc(b, &tex).ok()?;
        let h = img.content_hash();
        let path = tex_dir.join(format!("{h}.png"));
        if !path.exists() {
            write_png(&path, &img).ok()?;
        }
        hashes.insert(slot, h.clone());
        Some(h)
    };
    let mut manifest = String::from("# name tris min max textures\n");
    let mut written = 0;
    for def in &objs.object_defs {
        let Some(obj) = objs.objects.get(def.obj_index.max(0) as usize) else {
            continue;
        };
        let mut o = String::new();
        let mut mtl = String::new();
        let (mut vbase, mut tris) = (1u32, 0usize);
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        let mut used = Vec::new();
        writeln!(o, "# {} from {}", def.name, dir.display())?;
        writeln!(o, "mtllib {}.mtl", def.name)?;
        writeln!(o, "o {}", def.name)?;
        for (si, (sub, stream)) in obj.sub_objects.iter().zip(&obj.geometry).enumerate() {
            let m = match gdl_formats::Mesh::decode(stream) {
                Ok(m) if !m.indices.is_empty() => m,
                Ok(_) => continue,
                Err(e) => {
                    eprintln!("{}: sub-object {si}: {e:#}", def.name);
                    continue;
                }
            };
            let mat = match hash_of(sub.tex_index) {
                Some(h) => {
                    if !used.contains(&h) {
                        writeln!(mtl, "newmtl t_{h}\nmap_Kd ../tex/{h}.png\n")?;
                        used.push(h.clone());
                    }
                    format!("t_{h}")
                }
                None => "none".to_string(),
            };
            for p in &m.positions {
                for k in 0..3 {
                    lo[k] = lo[k].min(p[k]);
                    hi[k] = hi[k].max(p[k]);
                }
                writeln!(o, "v {} {} {}", p[0], p[1], p[2])?;
            }
            for uv in &m.uvs {
                writeln!(o, "vt {} {}", uv[0], 1.0 - uv[1])?;
            }
            for n in &m.normals {
                writeln!(o, "vn {} {} {}", n[0], n[1], n[2])?;
            }
            writeln!(o, "g {}_{si}\nusemtl {mat}", def.name)?;
            for t in m.indices.chunks_exact(3) {
                let [a, b, c] = [t[0] + vbase, t[1] + vbase, t[2] + vbase];
                writeln!(o, "f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}")?;
                tris += 1;
            }
            vbase += m.positions.len() as u32;
        }
        if tris == 0 {
            continue;
        }
        std::fs::write(parts_dir.join(format!("{}.obj", def.name)), o)?;
        std::fs::write(parts_dir.join(format!("{}.mtl", def.name)), mtl)?;
        writeln!(
            manifest,
            "{} {tris} {:.3},{:.3},{:.3} {:.3},{:.3},{:.3} {}",
            def.name,
            lo[0],
            lo[1],
            lo[2],
            hi[0],
            hi[1],
            hi[2],
            used.join(",")
        )?;
        written += 1;
    }
    std::fs::write(out.join("parts.txt"), manifest)?;
    println!("{written} parts -> {}", parts_dir.display());
    Ok(())
}

fn write_png(path: &Path, img: &gdl_formats::Image) -> Result<()> {
    let f = std::io::BufWriter::new(std::fs::File::create(path)?);
    let mut enc = png::Encoder::new(f, img.width, img.height);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img.rgba)?;
    Ok(())
}

fn find_files(root: &Path, name: &str, out: &mut Vec<PathBuf>) -> Result<()> {
    for e in std::fs::read_dir(root)? {
        let p = e?.path();
        if p.is_dir() {
            find_files(&p, name, out)?;
        } else if p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)) {
            out.push(p);
        }
    }
    Ok(())
}

fn survey(root: &Path) -> Result<()> {
    let mut files = Vec::new();
    find_files(root, "objects.ngc", &mut files)?;
    files.sort();
    let mut formats: BTreeMap<u8, usize> = BTreeMap::new();
    let (mut n_obj, mut n_sub, mut sub_err, mut tex_ok, mut tex_err, mut mism) = (0, 0, 0, 0, 0, 0);
    for f in &files {
        let objs = match Objects::parse(&std::fs::read(f)?) {
            Ok(o) => o,
            Err(e) => {
                println!("PARSE FAIL {}: {e:#}", f.display());
                continue;
            }
        };
        let tex = std::fs::read(f.with_file_name("textures.ngc")).unwrap_or_default();
        for o in &objs.objects {
            n_obj += 1;
            let mut v = 0;
            for g in &o.geometry {
                n_sub += 1;
                match Mesh::decode(g) {
                    Ok(m) => v += m.positions.len(),
                    Err(e) => {
                        sub_err += 1;
                        if sub_err < 10 {
                            println!("mesh err {}: {e:#}", f.display());
                        }
                    }
                }
            }
            if o.vert_count > 0 && v as i32 != o.vert_count {
                mism += 1;
            }
        }
        for b in &objs.bitmaps {
            *formats.entry(b.format).or_default() += 1;
            match texture::decode_ngc(b, &tex) {
                Ok(_) => tex_ok += 1,
                Err(e) => {
                    tex_err += 1;
                    if tex_err < 10 {
                        println!("tex err {}: {e:#}", f.display());
                    }
                }
            }
        }
    }
    println!("{} files, {n_obj} objects, {n_sub} sub-objects ({sub_err} errors, {mism} objects w/ vert count != header)", files.len());
    println!("textures: {tex_ok} ok, {tex_err} errors; formats: {formats:?}");
    Ok(())
}
