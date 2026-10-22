mod character;
mod fs;
mod gpu;
mod level;
mod mesh;

use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use std::path::PathBuf;

struct Args {
    data: Option<PathBuf>,
    level: String,
    snapshot: Option<PathBuf>,
    eye: Option<Vec3>,
    at: Option<Vec3>,
    size: (u32, u32),
    character: Option<String>,
    seq: String,
    frame: f32,
}

fn parse_vec3(s: &str) -> Result<Vec3> {
    let v: Vec<f32> = s
        .split(',')
        .map(|p| p.trim().parse())
        .collect::<Result<_, _>>()?;
    anyhow::ensure!(v.len() == 3, "expected x,y,z");
    Ok(Vec3::new(v[0], v[1], v[2]))
}

fn parse_args() -> Result<Args> {
    let mut a = Args {
        data: None,
        level: "levelA1".into(),
        snapshot: None,
        eye: None,
        at: None,
        size: (1280, 720),
        character: None,
        seq: "READY".into(),
        frame: 0.0,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut val = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--data" => a.data = Some(val()?.into()),
            "--level" => a.level = val()?,
            "--snapshot" => a.snapshot = Some(val()?.into()),
            "--char" => a.character = Some(val()?),
            "--seq" => a.seq = val()?,
            "--frame" => a.frame = val()?.parse()?,
            "--eye" => a.eye = Some(parse_vec3(&val()?)?),
            "--at" => a.at = Some(parse_vec3(&val()?)?),
            "--size" => {
                let v = val()?;
                let (w, h) = v.split_once('x').context("--size WxH")?;
                a.size = (w.parse()?, h.parse()?);
            }
            "-h" | "--help" => {
                println!("gauntlet [--data ISO|DIR] [--level levelA1] [--snapshot out.png --eye x,y,z --at x,y,z --size WxH]");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument {other}"),
        }
    }
    Ok(a)
}

/// The game is left-handed; mirror X so it looks right in a right-handed renderer.
pub fn mirror() -> Mat4 {
    Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))
}

fn snapshot(args: &Args) -> Result<()> {
    let fs = match &args.data {
        Some(p) => fs::GameFs::open(p)?,
        None => fs::GameFs::discover()?,
    };
    let mut char_model = None;
    let mesh = if let Some(c) = &args.character {
        let (class, color) = c.split_once('/').unwrap_or((c.as_str(), "RED"));
        let cm = if class.eq_ignore_ascii_case("monster") {
            character::CharacterModel::load_monster(&fs, color)?
        } else {
            character::CharacterModel::load_player(&fs, class, color)?
        };
        let mesh = std::mem::take(&mut { cm.mesh.clone_shallow() });
        char_model = Some(cm);
        mesh
    } else {
        level::Level::load(&fs, &args.level)?.world_mesh()
    };
    println!(
        "mesh: {} verts, {} tris, {} batches, {} textures, bounds {:?}..{:?}",
        mesh.vertices.len(),
        mesh.indices.len() / 3,
        mesh.batches.len(),
        mesh.textures.len(),
        mesh.bounds_min,
        mesh.bounds_max
    );

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    println!("adapter: {:?}", adapter.get_info().name);
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let gpu = gpu::Gpu::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let gm = gpu.upload_mesh(&mesh)?;
    let mut inst = gpu.new_instance(&gm);
    inst.transform = mirror();
    if let Some(cm) = &char_model {
        let seq = cm.sequence(&args.seq).with_context(|| format!("no sequence {}", args.seq))?;
        let mut an = character::Animator::new(seq);
        an.frame = args.frame;
        inst.parts = cm.part_transforms(&an.poses(&cm.skeleton));
        let names: Vec<_> = cm.skeleton.sequences.iter().map(|s| s.name.as_str()).collect();
        log::info!("sequences: {}", names.join(" "));
    }

    let center = (mesh.bounds_min + mesh.bounds_max) * 0.5;
    let extent = (mesh.bounds_max - mesh.bounds_min).length();
    let at = args.at.unwrap_or(center);
    let eye = args
        .eye
        .unwrap_or(center + Vec3::new(0.0, extent * 0.6, -extent * 0.6));
    let (w, h) = args.size;
    let m = mirror();
    let view = Mat4::look_at_rh(m.transform_point3(eye), m.transform_point3(at), Vec3::Y);
    let proj = Mat4::perspective_rh(50f32.to_radians(), w as f32 / h as f32, 0.5, 2000.0);
    let g = gpu::globals(
        proj * view,
        m.transform_point3(eye),
        [0.05, 0.05, 0.08],
        0.0,
    );
    let img = gpu.render_to_image(w, h, &g, [0.05, 0.05, 0.08], &[(&gm, &inst)])?;
    let out = args.snapshot.as_ref().unwrap();
    let f = std::io::BufWriter::new(std::fs::File::create(out)?);
    let mut enc = png::Encoder::new(f, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&img.rgba)?;
    println!("wrote {}", out.display());
    Ok(())
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = parse_args()?;
    if args.snapshot.is_some() {
        return snapshot(&args);
    }
    anyhow::bail!("interactive mode not built yet; use --snapshot")
}
