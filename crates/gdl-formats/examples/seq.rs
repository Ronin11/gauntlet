//! Render the sequenced (chained / looping) music calls in the DCS banks to WAV and print
//! numeric checks.
//!
//! `cargo run -p gdl-formats --example seq -- [AUDIO dir] [out dir]`
use gdl_formats::audio::{parse_sound_names, BANKS};
use gdl_formats::sequence::{render_call, CallPlayer, DcsBank, PlayParams, Rendered, OUTPUT_RATE};
use std::path::{Path, PathBuf};

const DEFAULT_AUDIO: &str = "/home/ronin/Projects/gauntlet-rs/game-data/files/Gauntlet/AUDIO";
const DEFAULT_OUT: &str = "/home/ronin/.cache/claude-tmp/claude-1000/-home-ronin/dea31ce5-88e6-4ac1-b203-6dd1ecdf6375/scratchpad/music";

fn write_wav(
    path: &Path,
    pcm: &[i16],
    rate: u32,
    loop_start: Option<usize>,
) -> std::io::Result<()> {
    let data_len = (pcm.len() * 2) as u32;
    let mut smpl = Vec::new();
    if let Some(ls) = loop_start {
        // "smpl" chunk with one forward loop, so tools can loop the file seamlessly.
        let words = [
            0u32,
            0,
            1_000_000_000 / rate,
            60,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            ls as u32,
            (pcm.len() / 2 - 1) as u32,
            0,
            0,
        ];
        smpl.extend_from_slice(b"smpl");
        smpl.extend_from_slice(&((words.len() * 4) as u32).to_le_bytes());
        for w in words {
            smpl.extend_from_slice(&w.to_le_bytes());
        }
    }
    let mut out = Vec::with_capacity(44 + pcm.len() * 2 + smpl.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len + smpl.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in pcm {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out.extend_from_slice(&smpl);
    std::fs::write(path, out)
}

/// Mean |x[n] - x[n-1]| over the left channel, and the jump at stereo frame `at`.
fn jump_at(pcm: &[i16], at: usize) -> (f64, f64) {
    let l = |i: usize| pcm[i * 2] as f64;
    let n = pcm.len() / 2;
    let a = at.saturating_sub(200).max(1);
    let b = (at + 200).min(n);
    let mean = (a..b).map(|i| (l(i) - l(i - 1)).abs()).sum::<f64>() / (b - a) as f64;
    let jump = if at > 0 && at < n {
        (l(at) - l(at - 1)).abs()
    } else {
        0.0
    };
    (mean, jump)
}

fn report(name: &str, bank: &DcsBank, b: &[u8], r: &Rendered, rom_dur: Option<f32>) {
    let n = r.frames();
    let peak = r.pcm.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0);
    let clipped = r
        .pcm
        .iter()
        .filter(|&&s| s == i16::MAX || s == i16::MIN)
        .count();
    let rms =
        (r.pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / r.pcm.len().max(1) as f64).sqrt();
    let lr_equal = r.pcm.chunks_exact(2).all(|c| c[0] == c[1]);
    println!(
        "{name}: {:.3}s ({n} frames @{}), loop_start {:?} ({}), peak {peak}, rms {rms:.0}, clipped {clipped}, L==R {lr_equal}, rom duration {:?}",
        n as f64 / OUTPUT_RATE as f64,
        r.sample_rate,
        r.loop_start,
        r.loop_start.map_or("none".into(), |l| format!("{:.3}s, loop length {:.3}s", l as f64 / OUTPUT_RATE as f64, (n - l) as f64 / OUTPUT_RATE as f64)),
        rom_dur,
    );
    let mut full = 0.0;
    for s in &r.segments {
        let v = &bank.samples[s.sample];
        full += s.length as f64 / v.rate as f64;
        let (mean, jump) = jump_at(&r.pcm, s.start);
        println!(
            "   word {} sample {:>3} {:<10} @{:>8.3}s  played {:>7}/{:<7} ({:.1} ms dropped)  boundary jump {jump:.0} vs mean step {mean:.0}",
            s.word,
            s.sample,
            v.name,
            s.start as f64 / OUTPUT_RATE as f64,
            s.played,
            s.length,
            (s.length - s.played) as f64 * 1000.0 / v.rate as f64,
        );
    }
    println!("   sum of full sample lengths {full:.3}s");
    // Joins in the input domain: does the as-played (truncated) segment flow into the next
    // one better than the full sample would?
    let mut segs: Vec<_> = r.segments.iter().collect();
    if let Some(l) = r.loop_start {
        if let Some(s) = r.segments.iter().find(|s| s.start == l) {
            segs.push(s);
        }
    }
    for w in segs.windows(2) {
        let a = bank.decode_sample(b, w[0].sample).unwrap();
        let c = bank.decode_sample(b, w[1].sample).unwrap();
        let cut = w[0].played.max(1);
        let full_step = (a[a.len() - 1] as i32 - c[0] as i32).abs();
        let cut_step = (a[cut - 1] as i32 - c[0] as i32).abs();
        let tail_peak = a[cut..]
            .iter()
            .map(|&x| (x as i32).abs())
            .max()
            .unwrap_or(0);
        // Correlation of the dropped tail with the next sample's head.
        let n = a.len() - cut;
        let (mut xy, mut xx, mut yy) = (0f64, 0f64, 0f64);
        for i in 0..n.min(c.len()) {
            let (x, y) = (a[cut + i] as f64, c[i] as f64);
            xy += x * y;
            xx += x * x;
            yy += y * y;
        }
        println!(
            "   join {}->{}: step full {full_step}, as played {cut_step}; dropped tail peak {tail_peak}, tail/head correlation {:.2}",
            bank.samples[w[0].sample].name,
            bank.samples[w[1].sample].name,
            xy / (xx * yy).sqrt().max(1.0)
        );
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let audio = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT_AUDIO.into()));
    let out = PathBuf::from(args.next().unwrap_or_else(|| DEFAULT_OUT.into()));
    std::fs::create_dir_all(&out).unwrap();
    let names = parse_sound_names(&std::fs::read(audio.join("AUDATPS2.ROM")).unwrap());
    let by_pos: Names = names
        .iter()
        .map(|(n, &(b, i, d))| ((b, i), (n.clone(), d)))
        .collect();

    let params = PlayParams::default();
    let (mut other, mut rendered) = (0, 0);
    for (bi, bank_name) in BANKS.iter().enumerate() {
        let b = std::fs::read(audio.join(format!("{bank_name}.VBK"))).unwrap();
        let bank = DcsBank::parse(&b).unwrap_or_else(|e| panic!("{bank_name}: {e}"));
        for ci in 0..bank.calls.len() {
            let (name, dur) = by_pos
                .get(&(bi, ci))
                .cloned()
                .unwrap_or((format!("{bank_name}_{ci}"), 0.0));
            // The music: the select theme, the shop tunes and the map stingers.
            let music =
                name == "S_SELECTMUS" || name.starts_with("S_SHOP_") || name.starts_with("S_MAP_");
            if !music {
                if bank.calls[ci].is_sequence() {
                    other += 1;
                }
                continue;
            }
            let r = render_call(&bank, &b, ci, &params).unwrap();
            report(&name, &bank, &b, &r, Some(dur));
            if r.loop_start.is_some() {
                check_loop(&bank, &b, ci, &params, &r);
            }
            rms_profile(&r);
            let loud = render_call(&bank, &b, ci, &PlayParams::music(255, 255)).unwrap();
            println!(
                "   at full music+sfx volume: peak {}, clipped {}",
                loud.pcm
                    .iter()
                    .map(|&s| (s as i32).abs())
                    .max()
                    .unwrap_or(0),
                loud.pcm
                    .iter()
                    .filter(|&&s| s == i16::MAX || s == i16::MIN)
                    .count()
            );
            let path = out.join(format!("{name}.wav"));
            write_wav(&path, &r.pcm, r.sample_rate, r.loop_start).unwrap();
            println!("   -> {}", path.display());
            rendered += 1;
        }
    }
    println!("{rendered} music calls written; {other} other chained/looping calls (sfx, ambience)");
    check_rom_durations(&audio, &by_pos, &params);
}

/// Stream the call on for two more passes and compare each with the first loop pass.
fn check_loop(bank: &DcsBank, b: &[u8], ci: usize, params: &PlayParams, r: &Rendered) {
    let ls = r.loop_start.unwrap();
    let pass = r.frames() - ls;
    let mut p = CallPlayer::new(bank, b, ci, params).unwrap();
    let mut pcm = Vec::new();
    while pcm.len() < (r.frames() + 2 * pass) * 2 {
        assert!(p.render_frame(&mut pcm));
    }
    let n = r.pcm.len();
    let first_identical = pcm[..n] == r.pcm[..];
    let a = &pcm[ls * 2..n];
    let p2 = &pcm[n..n + pass * 2];
    let p3 = &pcm[n + pass * 2..n + pass * 4];
    println!(
        "   loop check: streamed pass 1 identical {first_identical}, pass 2 == loop {}, pass 3 == loop {}",
        p2 == a,
        p3 == a
    );
}

/// RMS per second, to see the music is present all the way through.
fn rms_profile(r: &Rendered) {
    let sec = OUTPUT_RATE as usize * 2;
    let v: Vec<String> = r
        .pcm
        .chunks(sec)
        .map(|c| {
            format!(
                "{:.0}",
                (c.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / c.len() as f64).sqrt()
            )
        })
        .collect();
    println!("   rms per second: {}", v.join(" "));
}

type Names = std::collections::HashMap<(usize, usize), (String, f32)>;

/// For every one-shot call in every bank, compare the rendered length to the ROM duration.
fn check_rom_durations(audio: &Path, by_pos: &Names, params: &PlayParams) {
    let (mut n, mut worst, mut sum, mut peak) = (0, 0f64, 0f64, 0);
    let mut worst_name = String::new();
    for (bi, bank_name) in BANKS.iter().enumerate() {
        let b = std::fs::read(audio.join(format!("{bank_name}.VBK"))).unwrap();
        let bank = DcsBank::parse(&b).unwrap();
        for ci in 0..bank.calls.len() {
            let Some((name, dur)) = by_pos.get(&(bi, ci)) else {
                continue;
            };
            if *dur <= 0.0 {
                continue;
            }
            let r = render_call(&bank, &b, ci, params).unwrap();
            peak = peak.max(r.pcm.iter().map(|&s| (s as i32).abs()).max().unwrap_or(0));
            let d = (r.frames() as f64 / OUTPUT_RATE as f64 - *dur as f64).abs();
            if d > worst {
                worst = d;
                worst_name = name.clone();
            }
            sum += d;
            n += 1;
        }
    }
    println!(
        "one-shot calls: {n} rendered, |length - ROM duration| mean {:.1} ms, worst {:.1} ms ({worst_name}), peak sample {peak}",
        sum / n as f64 * 1000.0,
        worst * 1000.0
    );
}
