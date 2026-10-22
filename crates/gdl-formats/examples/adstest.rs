use gdl_formats::audio::Ads;
fn main() {
    let p = std::env::args().nth(1).unwrap();
    let b = std::fs::read(&p).unwrap();
    let a = Ads::parse(&b).unwrap();
    println!(
        "rate {} ch {} interleave field {:#x} samples {} loop {} {}..{}",
        a.sample_rate,
        a.channels,
        a.interleave,
        a.headers[0].num_samples,
        a.headers[0].loop_flag,
        a.headers[0].loop_start_nibble,
        a.headers[0].loop_end_nibble
    );
    let half = a.data_size / a.channels;
    for il in [
        0x8usize, 0x20, 0x100, 0x400, 0x800, 0x1000, 0x2000, 0x4000, 0x8000, 0x10000, half,
    ] {
        let pcm = a.decode(&b, il);
        // roughness: mean abs second difference / mean abs amplitude over the first 10 s of channel 0
        let n = (a.sample_rate as usize * 10).min(pcm.len() / a.channels);
        let s: Vec<f64> = (0..n).map(|i| pcm[i * a.channels] as f64).collect();
        let amp: f64 = s.iter().map(|v| v.abs()).sum::<f64>() / n as f64;
        let d2: f64 = s
            .windows(3)
            .map(|w| (w[0] - 2.0 * w[1] + w[2]).abs())
            .sum::<f64>()
            / n as f64;
        let mut corr = 0.0;
        if a.channels == 2 {
            let (mut sl, mut sr, mut slr, mut sll, mut srr) = (0f64, 0f64, 0f64, 0f64, 0f64);
            for i in 0..n {
                let l = pcm[i * 2] as f64;
                let r = pcm[i * 2 + 1] as f64;
                sl += l;
                sr += r;
                slr += l * r;
                sll += l * l;
                srr += r * r;
            }
            let nf = n as f64;
            corr = (slr - sl * sr / nf)
                / (((sll - sl * sl / nf) * (srr - sr * sr / nf)).sqrt() + 1e-9);
        }
        println!(
            "interleave {il:#8x}: amp {amp:8.1} rough {:.3} LRcorr {corr:.3}",
            d2 / amp.max(1.0)
        );
    }
}
