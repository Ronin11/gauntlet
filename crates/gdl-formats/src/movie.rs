//! Full-motion video: `VQMOVIES/*.avi` (RIFF AVI, video FourCC `MVDV`).
//!
//! Reconstructed from the GameCube player (`movieplayer.cpp` in the decomp:
//! `PlayVQMovie`, the open/parse pair `fn_800DA920`/`fn_800DACD8`, the demuxer
//! `fn_800DB3D4`, and the VQ codec `fn_800D93D4` → `fn_800D87FC`/`fn_800D8BCC`).
//!
//! # Container
//! A plain AVI: `hdrl` with one `vids` stream (`strf` = BITMAPINFOHEADER,
//! compression `MVDV`, 24 bpp) and an optional `auds` stream, then a `movi`
//! list of `00dc`/`00db` video, `01wb` audio and `JUNK` chunks (optionally
//! grouped in `LIST` records). The player walks `movi` one top-level chunk at a
//! time, stops at the first unknown tag or after `strh.length` video frames.
//!
//! # Video (`MVDV`)
//! Every video chunk is LZ-compressed (optionally behind an 8-byte
//! `{u32 length, u32 0}` prefix) into a decoder-owned buffer that **persists
//! between frames**; a back-reference may copy from bytes the previous frame
//! left there. The decompressed frame is:
//!
//! ```text
//! u16 count        codebook entries; > 256 means 9-bit indices
//! u16 key          != 0: every 2x2 block coded; 0: masked (delta) frame
//! key frame:    count * 12 bytes codebook | [9th-bit array] | indices
//! delta frame:  u32 nbits | 4x4 change mask (h/4 rows of (w+31)/32 bytes)
//!               | codebook | [9th-bit array, nbits bits] | indices
//! ```
//!
//! Each codebook entry is four YCbCr triples for one 2x2 pixel block (bottom
//! row first, the image is a bottom-up DIB). The player converts them in place
//! to BGR with float BT.601 maths, packs them to RGB565 and writes blocks into
//! a persistent 512x(384 padded to 512) GX texture; blocks whose 4x4 mask bit
//! is clear keep the previous frame's pixels.
//!
//! # Audio
//! The `auds` stream claims 8-bit mono PCM, but the player never looks at its
//! `strf`: it pushes the raw `01wb` bytes into the ADSTREAM layer
//! (`sndCmd16`/`sndCmd17` → `AdsPutBuffer`), which parses them as a Midway
//! `dhSS`/`dbSS` stream: GameCube DSP-ADPCM, 1-2 channels interleaved in
//! `sizeBlock`-byte blocks. Decoding is shared with [`crate::audio::Ads`].

use std::ops::Range;

use anyhow::{bail, Context, Result};

use crate::audio::{Ads, DspChannel, DspHeader};

fn le32(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn le16(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

const fn tag(s: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*s)
}
const RIFF: u32 = tag(b"RIFF");
const AVI: u32 = tag(b"AVI ");
const LIST: u32 = tag(b"LIST");
const JUNK: u32 = tag(b"JUNK");
const MOVI: u32 = tag(b"movi");
const VIDS: u32 = tag(b"vids");
const AUDS: u32 = tag(b"auds");
const STRH: u32 = tag(b"strh");
const STRF: u32 = tag(b"strf");
const STRL: u32 = tag(b"strl");
const HDRL: u32 = tag(b"hdrl");
const DC00: u32 = tag(b"00dc");
const DB00: u32 = tag(b"00db");
const WB01: u32 = tag(b"01wb");
/// `MVDV`, the BITMAPINFOHEADER compression the player insists on.
pub const MVDV: u32 = tag(b"MVDV");

/// The fields of an AVI `strh` the player reads.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamHeader {
    pub fcc_type: u32,
    pub fcc_handler: u32,
    pub scale: u32,
    pub rate: u32,
    pub start: u32,
    /// Frames (video) or bytes (audio).
    pub length: u32,
    pub sample_size: u32,
}

impl StreamHeader {
    fn parse(b: &[u8]) -> Option<Self> {
        Some(StreamHeader {
            fcc_type: le32(b, 0)?,
            fcc_handler: le32(b, 4)?,
            scale: le32(b, 0x14)?,
            rate: le32(b, 0x18)?,
            start: le32(b, 0x1C)?,
            length: le32(b, 0x20)?,
            sample_size: le32(b, 0x2C)?,
        })
    }
}

/// BITMAPINFOHEADER from the video `strf`.
#[derive(Debug, Clone, Copy, Default)]
pub struct BitmapInfo {
    pub width: i32,
    /// Positive: bottom-up rows.
    pub height: i32,
    pub bit_count: u16,
    pub compression: u32,
    pub size_image: u32,
}

/// WAVEFORMATEX from the audio `strf`. Informational only: the game ignores it
/// (it claims 8-bit mono PCM at the stream's byte rate).
#[derive(Debug, Clone, Copy, Default)]
pub struct WaveFormat {
    pub format_tag: u16,
    pub channels: u16,
    pub samples_per_sec: u32,
    pub avg_bytes_per_sec: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
}

/// A parsed movie file. Chunk lists are byte ranges into the file.
#[derive(Debug, Clone)]
pub struct Avi {
    pub video_header: StreamHeader,
    pub video_format: BitmapInfo,
    pub audio_header: Option<StreamHeader>,
    pub audio_format: Option<WaveFormat>,
    /// Payload of each video chunk, in presentation order.
    pub video_chunks: Vec<Range<usize>>,
    /// Payload of each `01wb` chunk; concatenated they form a `dhSS` stream.
    pub audio_chunks: Vec<Range<usize>>,
}

/// One RIFF chunk header: (fourcc, payload range).
fn chunk_at(b: &[u8], o: usize) -> Option<(u32, Range<usize>)> {
    let id = le32(b, o)?;
    let size = le32(b, o + 4)? as usize;
    let start = o + 8;
    Some((id, start..start.saturating_add(size).min(b.len())))
}

/// Iterate the chunks inside `range`, honouring RIFF word alignment.
fn chunks(b: &[u8], range: Range<usize>) -> impl Iterator<Item = (u32, Range<usize>)> + '_ {
    let mut o = range.start;
    std::iter::from_fn(move || {
        if o + 8 > range.end {
            return None;
        }
        let c = chunk_at(b, o)?;
        o = (c.1.end + 1) & !1;
        Some(c)
    })
}

fn parse_bitmap_info(f: &[u8]) -> Option<BitmapInfo> {
    Some(BitmapInfo {
        width: le32(f, 4)? as i32,
        height: le32(f, 8)? as i32,
        bit_count: le16(f, 0xE)?,
        compression: le32(f, 0x10)?,
        size_image: le32(f, 0x14)?,
    })
}

fn parse_wave_format(f: &[u8]) -> Option<WaveFormat> {
    Some(WaveFormat {
        format_tag: le16(f, 0)?,
        channels: le16(f, 2)?,
        samples_per_sec: le32(f, 4)?,
        avg_bytes_per_sec: le32(f, 8)?,
        block_align: le16(f, 0xC)?,
        bits_per_sample: le16(f, 0xE)?,
    })
}

impl Avi {
    pub fn parse(b: &[u8]) -> Result<Self> {
        if le32(b, 0) != Some(RIFF) || le32(b, 8) != Some(AVI) {
            bail!("not a RIFF AVI file");
        }
        let mut video: Option<(StreamHeader, BitmapInfo)> = None;
        let mut audio: Option<(StreamHeader, Option<WaveFormat>)> = None;
        let mut movi = None;
        for (id, r) in chunks(b, 12..b.len()) {
            if id != LIST || r.len() < 4 {
                continue;
            }
            match le32(b, r.start) {
                Some(HDRL) => {
                    for (id, r) in chunks(b, r.start + 4..r.end) {
                        if id != LIST || le32(b, r.start) != Some(STRL) {
                            continue;
                        }
                        let mut strh = None;
                        let mut strf = None;
                        for (id, r) in chunks(b, r.start + 4..r.end) {
                            match id {
                                STRH => strh = StreamHeader::parse(&b[r]),
                                STRF => strf = Some(&b[r]),
                                _ => {}
                            }
                        }
                        let Some(h) = strh else { continue };
                        match h.fcc_type {
                            VIDS if video.is_none() => {
                                let f = strf.context("video stream without strf")?;
                                let bi = parse_bitmap_info(f).context("short video strf")?;
                                video = Some((h, bi));
                            }
                            AUDS if audio.is_none() => {
                                audio = Some((h, strf.and_then(parse_wave_format)));
                            }
                            _ => {}
                        }
                    }
                }
                Some(MOVI) => movi = Some(r.start + 4..r.end),
                _ => {}
            }
        }
        let (video_header, video_format) = video.context("no video stream")?;
        let movi = movi.context("no movi list")?;

        // Demux like fn_800DB3D4: one node per top-level movi chunk; a LIST node
        // contributes its sub-chunks. A node keeps only its last video chunk, every
        // audio chunk feeds the audio ring. An unknown top-level tag ends the movie.
        let limit = video_header.length as usize;
        let mut video_chunks = Vec::new();
        let mut audio_chunks = Vec::new();
        for (id, r) in chunks(b, movi) {
            if !matches!(id, DC00 | DB00 | LIST | JUNK | WB01) {
                break;
            }
            let subs: Vec<(u32, Range<usize>)> = if id == LIST {
                chunks(b, r.start + 4..r.end).collect()
            } else {
                vec![(id, r)]
            };
            let mut node_video = None;
            for (id, r) in subs {
                match id {
                    WB01 => audio_chunks.push(r),
                    DC00 | DB00 => node_video = Some(r),
                    _ => {}
                }
            }
            if let Some(v) = node_video {
                video_chunks.push(v);
            }
            if video_chunks.len() >= limit {
                break;
            }
        }
        Ok(Avi {
            video_header,
            video_format,
            audio_header: audio.map(|a| a.0),
            audio_format: audio.and_then(|a| a.1),
            video_chunks,
            audio_chunks,
        })
    }

    pub fn width(&self) -> usize {
        self.video_format.width.unsigned_abs() as usize
    }
    pub fn height(&self) -> usize {
        self.video_format.height.unsigned_abs() as usize
    }

    /// Frames per second (`strh.rate / strh.scale`).
    pub fn frame_rate(&self) -> f32 {
        self.video_header.rate as f32 / self.video_header.scale.max(1) as f32
    }

    /// The audio stream bytes (all `01wb` payloads, concatenated).
    pub fn audio_stream(&self, b: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for r in &self.audio_chunks {
            out.extend_from_slice(&b[r.clone()]);
        }
        out
    }

    /// Decode the soundtrack, or `None` if the movie has no audio stream.
    pub fn decode_audio(&self, b: &[u8]) -> Result<Option<MovieAudio>> {
        if self.audio_header.is_none() || self.audio_chunks.is_empty() {
            return Ok(None);
        }
        let stream = self.audio_stream(b);
        if let Ok(ads) = Ads::parse(&stream) {
            let pcm = ads.decode_all(&stream);
            return Ok(Some(MovieAudio {
                sample_rate: ads.sample_rate,
                playback_rate: ax_playback_rate(ads.sample_rate),
                channels: ads.channels,
                pcm,
                game_playable: true,
            }));
        }
        // Two unused test files (TEST.AVI, 512x256.avi) carry a bare mono DSP
        // header instead. AdsPutBuffer rejects that, so the game plays them silent.
        let h = DspHeader::parse(&stream).context("movie audio is neither dhSS nor DSP")?;
        if !(4000..=50000).contains(&h.sample_rate) || h.num_samples == 0 {
            bail!("movie audio is neither dhSS nor DSP");
        }
        let mut ch = DspChannel::default();
        let mut pcm = Vec::with_capacity(h.num_samples as usize + 14);
        for frame in stream[0x60..].chunks_exact(8) {
            ch.decode_frame(&h.coefs, frame, &mut pcm);
        }
        pcm.truncate(h.num_samples as usize);
        Ok(Some(MovieAudio {
            sample_rate: h.sample_rate,
            playback_rate: ax_playback_rate(h.sample_rate),
            channels: 1,
            pcm,
            game_playable: false,
        }))
    }
}

/// The rate the AX voice actually plays a stream at: `adsInitFromHeader` builds
/// the SRC ratio from `((rate << 12) / 48000 * 48000) >> 12` samples/s.
pub fn ax_playback_rate(rate: u32) -> u32 {
    let k = 48000u64;
    ((((rate as u64) << 12) / k * k) >> 12) as u32
}

/// Decoded movie soundtrack.
#[derive(Debug, Clone)]
pub struct MovieAudio {
    /// Rate from the `dhSS` header.
    pub sample_rate: u32,
    /// Rate after the AX resampler's truncated ratio (see [`ax_playback_rate`]).
    pub playback_rate: u32,
    pub channels: usize,
    /// Interleaved i16 PCM.
    pub pcm: Vec<i16>,
    /// False for a stream the game's ADSTREAM layer would reject (bare DSP).
    pub game_playable: bool,
}

/// LZ decompressor `fn_800D9DF0`. `src` starts at the 4-byte block header
/// (byte 0 == 1: stored). Output goes to the start of `dst`, which keeps its
/// previous contents beyond what this call writes. Returns the output length.
fn lz_decompress(src: &[u8], dst: &mut Vec<u8>) -> usize {
    let at = |i: usize| src.get(i).copied().unwrap_or(0);
    if src.first() == Some(&1) {
        let data = src.get(4..).unwrap_or(&[]);
        if dst.len() < data.len() {
            dst.resize(data.len(), 0);
        }
        dst[..data.len()].copy_from_slice(data);
        return data.len();
    }
    let end = src.len();
    let limit = end.saturating_sub(0x20);
    let mut p = 4;
    let mut out = 0usize;
    let mut flags = 1u32;
    while p < end {
        if flags == 1 {
            flags = at(p) as u32 | (at(p + 1) as u32) << 8 | 0x10000;
            p += 2;
        }
        // Far from the end a whole flag word is processed without end checks.
        let n = if p <= limit { 16 } else { 1 };
        for _ in 0..n {
            if flags & 1 != 0 {
                let (b0, b1) = (at(p), at(p + 1));
                p += 2;
                let dist = ((b0 as usize & 0xF0) << 4) | b1 as usize;
                let len = 3 + (b0 & 0xF) as usize;
                if dst.len() < out + len {
                    dst.resize(out + len, 0);
                }
                // Byte-wise forward copy: overlapping runs repeat, and distance 0
                // keeps what the previous frame left in the buffer. A source before
                // the buffer start (the GC would read heap memory) reads as 0.
                for k in out..out + len {
                    dst[k] = k.checked_sub(dist).map_or(0, |s| dst[s]);
                }
                out += len;
            } else {
                if dst.len() <= out {
                    dst.resize(out + 1, 0);
                }
                dst[out] = at(p);
                p += 1;
                out += 1;
            }
            flags >>= 1;
        }
    }
    out
}

/// YCbCr → BGR for one codebook triple, in place (`fn_800DBE98`). The float
/// constants are the player's `.sdata2` values; the multiply-adds are fused like
/// its `fmadds`/`fnmsubs`, then `+0.5`, truncated (`fctiwz`) and clamped through
/// the 0 / identity / 255 ramp table at `gDTextBuf`.
fn yuv_to_bgr(p: &mut [u8]) {
    const CR_R: f32 = 1.402; // 0x3fb374bc
    const CB_G: f32 = 0.34414; // 0x3eb0331e
    const CR_G: f32 = 0.71414; // 0x3f36d1e1
    const CB_B: f32 = 1.772; // 0x3fe2d0e5
    let y = p[0] as f32;
    let cb = p[1] as f32 - 128.0;
    let cr = p[2] as f32 - 128.0;
    let clamp = |v: f32| (v as i32).clamp(0, 255) as u8;
    p[2] = clamp(CR_R.mul_add(cr, y) + 0.5);
    p[1] = clamp((-CR_G).mul_add(cr, (-CB_G).mul_add(cb, y)) + 0.5);
    p[0] = clamp(CB_B.mul_add(cb, y) + 0.5);
}

/// Stateful `MVDV` decoder. Holds the picture as RGB565 (what the game's texture
/// holds), top row first.
pub struct VideoDecoder {
    width: usize,
    height: usize,
    mask_stride: usize,
    palette_offset: usize,
    /// The codec's decompression buffer (`MovieDecodeState.chunk`); persists.
    work: Vec<u8>,
    frame: Vec<u16>,
    frames_decoded: usize,
    last_key: bool,
}

impl VideoDecoder {
    pub fn new(width: usize, height: usize) -> Result<Self> {
        if !width.is_multiple_of(4) || !height.is_multiple_of(4) || width == 0 || height == 0 {
            bail!("MVDV needs dimensions divisible by 4, got {width}x{height}");
        }
        // fn_800D96B0's buffer sizing (allocation is size + 308).
        let mask_stride = width.div_ceil(32);
        let palette_offset = (height / 4) * mask_stride;
        let half = (width / 2) * height / 2;
        let size = half + 6146 + half / 8 + palette_offset + 308;
        Ok(VideoDecoder {
            width,
            height,
            mask_stride,
            palette_offset,
            work: vec![0; size],
            frame: vec![0; width * height],
            frames_decoded: 0,
            last_key: false,
        })
    }

    pub fn for_avi(avi: &Avi) -> Result<Self> {
        let f = &avi.video_format;
        if f.compression != MVDV || f.bit_count != 24 {
            bail!("video is not MVDV/24bpp");
        }
        Self::new(avi.width(), avi.height())
    }

    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    pub fn frames_decoded(&self) -> usize {
        self.frames_decoded
    }

    /// Whether the last decoded frame coded every block (no change mask).
    pub fn last_was_key(&self) -> bool {
        self.last_key
    }

    /// The current picture as RGB565, top row first.
    pub fn rgb565(&self) -> &[u16] {
        &self.frame
    }

    /// The current picture as RGBA8, top row first (565 expanded as GX does).
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.frame.len() * 4);
        for &c in &self.frame {
            let r = (c >> 11) as u8 & 31;
            let g = (c >> 5) as u8 & 63;
            let b = c as u8 & 31;
            out.extend_from_slice(&[r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2, 255]);
        }
        out
    }

    /// Decode one video chunk payload into the persistent picture (`fn_800D93D4`).
    pub fn decode(&mut self, chunk: &[u8]) -> Result<()> {
        let Some(first) = le32(chunk, 0) else {
            bail!("video chunk too short ({} bytes)", chunk.len());
        };
        // A leading u32 > 1 is the compressed length, followed by 4 more bytes.
        let src = if first as i32 > 1 {
            chunk
                .get(8..(8 + first as usize).min(chunk.len()))
                .unwrap_or(&[])
        } else {
            chunk
        };
        let n = lz_decompress(src, &mut self.work);
        if n < 4 {
            bail!("frame decompressed to {n} bytes");
        }
        let count = le16(&self.work, 0).unwrap_or(0) as usize;
        let key = le16(&self.work, 2).unwrap_or(0) != 0;
        let (masks, pal) = if key {
            (None, 4)
        } else {
            (Some(8), 8 + self.palette_offset)
        };
        self.convert_codebook(pal, count);
        let bits = pal + count * 12;
        let mut ip = bits;
        let nine_bit = count > 256;
        if nine_bit {
            ip += if key {
                (self.width / 2) * self.height / 2 / 8
            } else {
                (le32(&self.work, 4).unwrap_or(0) as usize).div_ceil(8)
            };
        }
        self.draw_blocks(masks, pal, ip, nine_bit.then_some(bits));
        self.frames_decoded += 1;
        self.last_key = key;
        Ok(())
    }

    /// Convert `count` codebook entries in place: each YCbCr triple to BGR, then
    /// to a big-endian RGB565 word at twice its index. The GC overwrites triples it
    /// has consumed, so the buffer the next frame's LZ sees holds this mixture.
    fn convert_codebook(&mut self, pal: usize, count: usize) {
        let need = pal + count * 12;
        if self.work.len() < need {
            self.work.resize(need, 0);
        }
        for i in 0..count * 4 {
            let t = pal + i * 3;
            yuv_to_bgr(&mut self.work[t..t + 3]);
            let (b, g, r) = (
                self.work[t] as u16,
                self.work[t + 1] as u16,
                self.work[t + 2] as u16,
            );
            let c = (b >> 3) | (g >> 2) << 5 | (r >> 3) << 11;
            self.work[pal + i * 2..pal + i * 2 + 2].copy_from_slice(&c.to_be_bytes());
        }
    }

    /// Write the 2x2 blocks (`fn_800D87FC` masked / `fn_800D8BCC` full, 16-bit
    /// output). Blocks run in bottom-up source order; `bits` is the offset of the
    /// 9th-index-bit array, read LSB first.
    fn draw_blocks(
        &mut self,
        masks: Option<usize>,
        pal: usize,
        mut ip: usize,
        bits: Option<usize>,
    ) {
        let work = &self.work;
        let byte = |i: usize| work.get(i).copied().unwrap_or(0);
        let word = |i: usize| u16::from_be_bytes([byte(i), byte(i + 1)]);
        let (w, h) = (self.width, self.height);
        let mut bp = bits.unwrap_or(0);
        let mut cur = byte(bp);
        bp += 1;
        let mut nb = 0;
        for y in (0..h).step_by(2) {
            let row = h - 1 - y;
            let mrow = masks.map(|m| m + (y / 4) * self.mask_stride);
            for bx in 0..w / 2 {
                if let Some(m) = mrow {
                    let b = bx / 2;
                    if byte(m + b / 8) & (1 << (b & 7)) == 0 {
                        continue;
                    }
                }
                let mut idx = byte(ip) as usize;
                ip += 1;
                if bits.is_some() {
                    idx |= ((cur >> nb) as usize & 1) << 8;
                    nb += 1;
                    if nb == 8 {
                        cur = byte(bp);
                        bp += 1;
                        nb = 0;
                    }
                }
                let e = pal + idx * 8;
                let x = row * w + bx * 2;
                self.frame[x] = word(e);
                self.frame[x + 1] = word(e + 2);
                self.frame[x - w] = word(e + 4);
                self.frame[x - w + 1] = word(e + 6);
            }
        }
    }
}

/// A movie file plus its decoder: decode frames in order.
pub struct Movie<'a> {
    pub avi: Avi,
    data: &'a [u8],
    decoder: VideoDecoder,
    next: usize,
}

impl<'a> Movie<'a> {
    pub fn open(data: &'a [u8]) -> Result<Self> {
        let avi = Avi::parse(data)?;
        let decoder = VideoDecoder::for_avi(&avi)?;
        Ok(Movie {
            avi,
            data,
            decoder,
            next: 0,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.avi.video_chunks.len()
    }

    /// Index of the frame the next call to [`Movie::advance`] decodes.
    pub fn position(&self) -> usize {
        self.next
    }

    pub fn decoder(&self) -> &VideoDecoder {
        &self.decoder
    }

    /// Decode the next frame into the persistent picture; `Ok(false)` at the end.
    pub fn advance(&mut self) -> Result<bool> {
        let Some(r) = self.avi.video_chunks.get(self.next).cloned() else {
            return Ok(false);
        };
        self.next += 1;
        self.decoder
            .decode(&self.data[r])
            .with_context(|| format!("frame {}", self.next - 1))?;
        Ok(true)
    }

    /// Decode the next frame and return it as RGBA8 (width x height, top row first).
    pub fn next_frame(&mut self) -> Option<Result<Vec<u8>>> {
        match self.advance() {
            Ok(true) => Some(Ok(self.decoder.rgba())),
            Ok(false) => None,
            Err(e) => Some(Err(e)),
        }
    }

    pub fn decode_audio(&self) -> Result<Option<MovieAudio>> {
        self.avi.decode_audio(self.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lz_literals_and_backrefs() {
        // Header, flags 0b1000 (4th item is a back-reference), "abc", then copy
        // 4 bytes from distance 3.
        let src = [0u8, 0, 0, 0, 0x08, 0x00, b'a', b'b', b'c', 0x01, 0x03];
        let mut dst = Vec::new();
        let n = lz_decompress(&src, &mut dst);
        assert_eq!(&dst[..n], b"abcabca");
    }

    #[test]
    fn lz_distance_zero_keeps_old_bytes() {
        let src = [0u8, 0, 0, 0, 0x01, 0x00, 0x00, 0x00];
        let mut dst = vec![9, 8, 7, 6];
        let n = lz_decompress(&src, &mut dst);
        assert_eq!(&dst[..n], &[9, 8, 7]);
    }

    #[test]
    fn yuv_constants_match_dol() {
        assert_eq!(1.402f32.to_bits(), 0x3fb374bc);
        assert_eq!(0.34414f32.to_bits(), 0x3eb0331e);
        assert_eq!(0.71414f32.to_bits(), 0x3f36d1e1);
        assert_eq!(1.772f32.to_bits(), 0x3fe2d0e5);
        let mut p = [100u8, 128, 128];
        yuv_to_bgr(&mut p);
        assert_eq!(p, [100, 100, 100]);
    }

    #[test]
    fn ax_rate() {
        assert_eq!(ax_playback_rate(48000), 48000);
        assert_eq!(ax_playback_rate(48042), 48035);
    }

    /// Decodes every shipped movie when the extracted game data is present.
    #[test]
    fn shipped_movies_decode() {
        let root = std::env::var_os("GDL_GAME_DATA").map_or_else(
            || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../game-data"),
            std::path::PathBuf::from,
        );
        let dir = root.join("files/Gauntlet/VQMOVIES");
        let Ok(rd) = std::fs::read_dir(&dir) else {
            eprintln!("skipping: no {}", dir.display());
            return;
        };
        for e in rd.flatten() {
            let data = std::fs::read(e.path()).unwrap();
            let mut m = Movie::open(&data).unwrap();
            let mut n = 0;
            while m.advance().unwrap() {
                n += 1;
            }
            assert!(n > 0, "{}", e.path().display());
            if let Some(a) = m.decode_audio().unwrap() {
                assert!(!a.pcm.is_empty());
            }
        }
    }
}
