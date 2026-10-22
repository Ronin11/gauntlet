//! Decode a `VQMOVIES/*.avi` movie: dump selected frames as PNG and the
//! soundtrack as WAV.
//!
//! Usage: `movie <file.avi> <out-dir> [frame...]` (frames default to 0, 30 and
//! the last; `all` writes every frame). Every frame is decoded either way.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gdl_formats::movie::Movie;

fn write_png(path: &Path, w: usize, h: usize, rgba: &[u8]) -> Result<()> {
    let f = BufWriter::new(File::create(path)?);
    let mut enc = png::Encoder::new(f, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(rgba)?;
    Ok(())
}

fn write_wav(path: &Path, rate: u32, channels: usize, pcm: &[i16]) -> Result<()> {
    let mut f = BufWriter::new(File::create(path)?);
    let data = (pcm.len() * 2) as u32;
    let ch = channels as u16;
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?;
    f.write_all(&ch.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&(rate * ch as u32 * 2).to_le_bytes())?;
    f.write_all(&(ch * 2).to_le_bytes())?;
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data.to_le_bytes())?;
    for s in pcm {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(
        args.next()
            .context("usage: movie <file.avi> <out-dir> [frame...]")?,
    );
    let out = PathBuf::from(args.next().context("missing output directory")?);
    let picks: Vec<String> = args.collect();
    std::fs::create_dir_all(&out)?;
    let stem = input.file_stem().unwrap().to_string_lossy().into_owned();

    let data = std::fs::read(&input).with_context(|| input.display().to_string())?;
    let mut movie = Movie::open(&data)?;
    let (w, h) = (movie.avi.width(), movie.avi.height());
    let n = movie.frame_count();
    println!(
        "{stem}: {w}x{h} {:.2} fps, {n} frames (strh {}), {} audio chunks",
        movie.avi.frame_rate(),
        movie.avi.video_header.length,
        movie.avi.audio_chunks.len()
    );
    if let Some(f) = movie.avi.audio_format {
        println!(
            "  AVI audio strf: tag {} ch {} {} Hz {} bits (ignored by the game)",
            f.format_tag, f.channels, f.samples_per_sec, f.bits_per_sample
        );
    }

    let all = picks.iter().any(|p| p == "all");
    let wanted: Vec<usize> = if picks.is_empty() {
        vec![0, 30.min(n.saturating_sub(1)), n.saturating_sub(1)]
    } else {
        picks.iter().filter_map(|p| p.parse().ok()).collect()
    };
    let (mut keys, mut deltas) = (0, 0);
    while movie.position() < n {
        let idx = movie.position();
        movie.advance()?;
        if movie.decoder().last_was_key() {
            keys += 1;
        } else {
            deltas += 1;
        }
        if all || wanted.contains(&idx) {
            let p = out.join(format!("{stem}_{idx:04}.png"));
            write_png(&p, w, h, &movie.decoder().rgba())?;
            println!("  wrote {}", p.display());
        }
    }
    println!(
        "  decoded {} frames ({keys} key / {deltas} delta)",
        movie.decoder().frames_decoded()
    );

    if let Some(a) = movie.decode_audio()? {
        let p = out.join(format!("{stem}.wav"));
        write_wav(&p, a.sample_rate, a.channels, &a.pcm)?;
        println!(
            "  audio: DSP-ADPCM {} ch {} Hz (AX plays at {} Hz), {:.2} s -> {}",
            a.channels,
            a.sample_rate,
            a.playback_rate,
            a.pcm.len() as f64 / a.channels as f64 / a.sample_rate as f64,
            p.display()
        );
    }
    Ok(())
}
