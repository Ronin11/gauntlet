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
        Ok(Ads { sample_rate, channels, interleave: be32(b, 0x14) as usize, headers, data_offset, data_size })
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
        let block = if ch == 1 { data.len() } else { interleave.max(8) };
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
        let n = chans.iter().map(Vec::len).min().unwrap_or(0).min(self.headers[0].num_samples as usize);
        let mut out = Vec::with_capacity(n * ch);
        for i in 0..n {
            for c in &chans {
                out.push(c[i]);
            }
        }
        out
    }
}
