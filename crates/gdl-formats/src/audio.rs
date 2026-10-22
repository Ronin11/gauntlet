//! GameCube DSP-ADPCM audio: music streams (`STREAMS/*.ads`, "dhSS"/"dbSS" headers).

use anyhow::{bail, Result};

/// Per-channel DSP-ADPCM header (the standard 0x60-byte Nintendo layout, big-endian).
#[derive(Debug, Clone)]
pub struct DspHeader {
    pub num_samples: u32,
    pub num_nibbles: u32,
    pub sample_rate: u32,
    pub loop_flag: bool,
    pub loop_start_nibble: u32,
    pub loop_end_nibble: u32,
    pub coefs: [i16; 16],
}

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}
fn be16(b: &[u8], o: usize) -> u16 {
    u16::from_be_bytes(b[o..o + 2].try_into().unwrap())
}

impl DspHeader {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < 0x60 {
            bail!("short DSP header");
        }
        let mut coefs = [0i16; 16];
        for (i, c) in coefs.iter_mut().enumerate() {
            *c = be16(b, 0x1C + i * 2) as i16;
        }
        Ok(DspHeader {
            num_samples: be32(b, 0),
            num_nibbles: be32(b, 4),
            sample_rate: be32(b, 8),
            loop_flag: be16(b, 0x0C) != 0,
            loop_start_nibble: be32(b, 0x10),
            loop_end_nibble: be32(b, 0x14),
            coefs,
        })
    }

    /// Convert a nibble address to a sample index.
    pub fn nibble_to_sample(n: u32) -> u32 {
        let frames = n / 16;
        let rem = n % 16;
        frames * 14 + rem.saturating_sub(2)
    }
}

/// Decoder state for one channel.
#[derive(Clone, Default)]
pub struct DspChannel {
    hist1: i32,
    hist2: i32,
}

impl DspChannel {
    /// Decode one 8-byte frame (14 samples).
    pub fn decode_frame(&mut self, coefs: &[i16; 16], frame: &[u8], out: &mut Vec<i16>) {
        let ps = frame[0];
        let pred = ((ps >> 4) & 7) as usize;
        let scale = 1i32 << (ps & 0xF);
        let c1 = coefs[pred * 2] as i32;
        let c2 = coefs[pred * 2 + 1] as i32;
        for i in 0..14 {
            let byte = frame[1 + i / 2];
            let nib = if i % 2 == 0 { byte >> 4 } else { byte & 0xF } as i32;
            let nib = if nib >= 8 { nib - 16 } else { nib };
            let s = ((nib * scale) << 11) + 1024 + c1 * self.hist1 + c2 * self.hist2;
            let s = (s >> 11).clamp(-32768, 32767);
            self.hist2 = self.hist1;
            self.hist1 = s;
            out.push(s as i16);
        }
    }
}

/// A music stream file.
#[derive(Debug, Clone)]
pub struct Ads {
    pub sample_rate: u32,
    pub channels: usize,
    pub interleave: usize,
    pub headers: Vec<DspHeader>,
    pub data_offset: usize,
    pub data_size: usize,
}

impl Ads {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < 0x40 || &b[0..4] != b"dhSS" || &b[0x20..0x24] != b"dbSS" {
            bail!("not a dhSS/dbSS stream");
        }
        let sample_rate = be32(b, 0x0C);
        let channels = be32(b, 0x10) as usize;
        if !(1..=2).contains(&channels) {
            bail!("unsupported channel count {channels}");
        }
        let data_size = be32(b, 0x24) as usize;
        let mut headers = Vec::new();
        for c in 0..channels {
            headers.push(DspHeader::parse(&b[0x28 + c * 0x60..])?);
        }
        let data_offset = 0x28 + channels * 0x60;
        Ok(Ads {
            sample_rate,
            channels,
            interleave: be32(b, 0x14) as usize,
            headers,
            data_offset,
            data_size,
        })
    }

    /// Decode the whole stream to interleaved i16 PCM using the header's interleave
    /// (verified: 0x20-byte blocks give strongly correlated L/R channels).
    pub fn decode_all(&self, b: &[u8]) -> Vec<i16> {
        self.decode(b, self.interleave)
    }

    /// Decode the whole stream to interleaved i16 PCM with an explicit interleave.
    pub fn decode(&self, b: &[u8], interleave: usize) -> Vec<i16> {
        let data = &b[self.data_offset..(self.data_offset + self.data_size).min(b.len())];
        let ch = self.channels;
        let mut chans: Vec<Vec<i16>> = vec![Vec::new(); ch];
        let mut st = vec![DspChannel::default(); ch];
        let block = if ch == 1 {
            data.len()
        } else {
            interleave.max(8)
        };
        let mut off = 0;
        'outer: while off < data.len() {
            for c in 0..ch {
                let start = off + c * block;
                let end = (start + block).min(data.len());
                if start >= data.len() {
                    break 'outer;
                }
                for frame in data[start..end].chunks_exact(8) {
                    st[c].decode_frame(&self.headers[c].coefs, frame, &mut chans[c]);
                }
            }
            off += block * ch;
        }
        let n = chans
            .iter()
            .map(Vec::len)
            .min()
            .unwrap_or(0)
            .min(self.headers[0].num_samples as usize);
        let mut out = Vec::with_capacity(n * ch);
        for i in 0..n {
            for c in &chans {
                out.push(c[i]);
            }
        }
        out
    }
}

/// One sound in a bank: samples played in sequence, and a volume 0..=127.
#[derive(Debug, Clone)]
pub struct SoundDef {
    pub samples: Vec<usize>,
    pub volume: u8,
}

/// A sound-effect bank (`AUDIO/*.VBK`, magic "KNBV"): sound table + DSP-ADPCM samples.
#[derive(Debug, Clone)]
pub struct Vbk {
    pub sounds: Vec<SoundDef>,
    /// (header, offset of sample data, data length in bytes)
    pub samples: Vec<(DspHeader, usize, usize)>,
}

impl Vbk {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if b.len() < 0x14 || &b[0..4] != b"KNBV" {
            bail!("not a KNBV sound bank");
        }
        let nsounds = be32(b, 0x0C) as usize;
        let nsamples = be32(b, 0x10) as usize;
        // Sound table: 8-byte entries. Each holds sample references (bit 15 marks the
        // final one; earlier unflagged words are played before it) and a volume.
        let mut sounds = Vec::with_capacity(nsounds);
        for i in 0..nsounds {
            let o = 0x14 + i * 8;
            if o + 8 > b.len() {
                break;
            }
            let w: Vec<u16> = (0..4).map(|k| be16(b, o + k * 2)).collect();
            let mut samples = Vec::new();
            let mut volume = 100;
            for k in 0..3 {
                let idx = (w[k] & 0x3FFF) as usize;
                if w[k] & 0x8000 != 0 && w[k + 1] <= 0x7F && idx < nsamples {
                    // Earlier unflagged words are clips played first ("Red" + "Warrior").
                    for &prev in &w[..k] {
                        let p = (prev & 0x3FFF) as usize;
                        if prev & 0x8000 == 0 && p < nsamples {
                            samples.push(p);
                        }
                    }
                    samples.push(idx);
                    volume = w[k + 1] as u8;
                    break;
                }
            }
            sounds.push(SoundDef { samples, volume });
        }
        // Samples: standard 0x60-byte DSP headers followed by their data.
        let mut samples = Vec::with_capacity(nsamples);
        let mut o = 0x14 + nsounds * 8;
        while o + 0x60 <= b.len() && samples.len() < nsamples {
            let ns = be32(b, o);
            let nn = be32(b, o + 4);
            let rate = be32(b, o + 8);
            if (4000..=48000).contains(&rate)
                && ns > 0
                && ns < 5_000_000
                && nn > ns
                && nn <= ns / 7 * 8 + 20
            {
                let h = DspHeader::parse(&b[o..])?;
                let len = (nn as usize).div_ceil(2);
                samples.push((h, o + 0x60, len));
                o = (o + 0x60 + len + 1) & !1;
            } else {
                o += 2;
            }
        }
        Ok(Vbk { sounds, samples })
    }

    /// Decode sample `i` to mono i16 PCM, returning (pcm, sample rate).
    pub fn decode_sample(&self, b: &[u8], i: usize) -> Option<(Vec<i16>, u32)> {
        let (h, off, len) = self.samples.get(i)?;
        let data = b.get(*off..(*off + *len).min(b.len()))?;
        let mut ch = DspChannel::default();
        let mut out = Vec::with_capacity(h.num_samples as usize + 14);
        for frame in data.chunks_exact(8) {
            ch.decode_frame(&h.coefs, frame, &mut out);
        }
        out.truncate(h.num_samples as usize);
        Some((out, h.sample_rate))
    }
}

/// Bank names in sound-id order (from `AUDATPS2.ROM`); the file is `AUDIO/<NAME>.VBK`.
pub const BANKS: &[&str] = &[
    "COMMON", "VOICE1", "VOICE2", "TOWAMB", "WAR", "VAL", "WIZ", "ARC", "DWF", "KNI", "SOR", "JES",
    "SELECT", "PROMOTE", "WIZTOWER", "MAP_A", "MAP_B", "MAP_C", "MAP_D", "MAP_E", "MAP_F", "MAP_G",
    "MAP_H", "MAP_I", "MAP_J", "MAP_K", "SHOP_A", "SHOP_B", "SHOP_C", "SHOP_D", "SHOP_E", "SHOP_F",
    "SHOP_G", "SHOP_H", "SHOP_I", "SHOP_J", "SHOP_K", "CASTLE", "MOUNT", "DESERT", "FOREST",
    "TOWN", "BATTLE", "ICE", "DREAM", "SKY", "DRAGON", "CHIMERA", "DJINN", "DRIDER", "YETI",
    "PBOSS", "LICH", "WRAITH", "CATH", "SKORNE1", "HELL", "SKORNE2", "GARM", "SECRET",
];

/// Sound names from `AUDIO/AUDATPS2.ROM`: name -> (bank, index within bank, duration s).
pub fn parse_sound_names(rom: &[u8]) -> std::collections::HashMap<String, (usize, usize, f32)> {
    let mut out = std::collections::HashMap::new();
    let Some(start) = rom.windows(6).position(|w| w == b"S_WARN") else {
        return out;
    };
    let mut o = start;
    while o + 28 <= rom.len() && &rom[o..o + 2] == b"S_" {
        let end = rom[o..o + 16].iter().position(|&c| c == 0).unwrap_or(16);
        let name = String::from_utf8_lossy(&rom[o..o + end]).into_owned();
        let idx = u16::from_le_bytes([rom[o + 16], rom[o + 17]]) as usize;
        let bank = u16::from_le_bytes([rom[o + 18], rom[o + 19]]) as usize;
        let dur = f32::from_le_bytes(rom[o + 20..o + 24].try_into().unwrap());
        out.insert(name, (bank, idx, dur));
        o += 28;
    }
    out
}
