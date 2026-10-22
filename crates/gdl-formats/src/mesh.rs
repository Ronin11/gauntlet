//! Decoder for the VIF-style geometry streams inside sub-object models.
//!
//! A stream is a sequence of 4-byte-aligned packets, each `[data_type, flags, count,
//! storage_type]` followed by `count` elements. A strip starts with a GEOM/FLOAT32
//! header (whose third field is the winding direction), then positions, normals,
//! optional colours and UVs. The high bit of each packed normal marks a triangle
//! that must not be drawn (strip restarts are encoded that way).

use crate::bin::Le;
use anyhow::{bail, Result};

const DATA_GEOM: u8 = 0;
const DATA_POS: u8 = 1;
const DATA_NORM: u8 = 2;
const DATA_COLOR: u8 = 3;
const DATA_UV: u8 = 4;

const ST_NULL: u8 = 0x00;
const ST_UNKNOWN: u8 = 0x01;
const ST_STRIP_0_END: u8 = 0x14;
const ST_STRIP_N_END: u8 = 0x17;
const ST_UINT32_UV: u8 = 0x64;
const ST_UINT16_UV: u8 = 0x65;
const ST_UINT8_UV: u8 = 0x66;
const ST_SINT32_XYZ: u8 = 0x68;
const ST_SINT16_XYZ: u8 = 0x69;
const ST_SINT8_XYZ: u8 = 0x6A;
const ST_FLOAT32: u8 = 0x6C;
const ST_UINT16_LMUV: u8 = 0x6D;
const ST_UINT16_BITPACKED: u8 = 0x6F;

const POS_SCALE: f32 = 1.0 / 128.0;
const UV_SCALE: f32 = 1.0 / 128.0;
const LM_UV_SCALE: f32 = 1.0 / 32768.0;

/// Triangle soup for one sub-object, ready for a GPU vertex buffer.
#[derive(Debug, Default, Clone)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// RGBA in 0..=1. White when the stream has no colours.
    pub colors: Vec<[f32; 4]>,
    pub uvs: Vec<[f32; 2]>,
    pub lm_uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

struct Strip {
    face_dir: f32,
    /// One entry per triangle (vertex count - 2): true = draw it.
    draw: Vec<bool>,
}

fn unpack_norm(n: u16) -> [f32; 3] {
    let x = (n & 31) as f32 / 15.0 - 1.0;
    let y = ((n >> 5) & 31) as f32 / 15.0 - 1.0;
    let z = ((n >> 10) & 31) as f32 / 15.0 - 1.0;
    let inv = 1.0 / ((x * x + y * y + z * z).sqrt() + 1e-7);
    [x * inv, y * inv, z * inv]
}

fn unpack_color(c: u16) -> [f32; 4] {
    [
        (c & 31) as f32 / 31.0,
        ((c >> 5) & 31) as f32 / 31.0,
        ((c >> 10) & 31) as f32 / 31.0,
        ((c >> 15) & 1) as f32,
    ]
}

fn stride(data_type: u8, storage: u8) -> Result<usize> {
    Ok(match (data_type, storage) {
        (DATA_GEOM, ST_NULL | ST_STRIP_0_END | ST_STRIP_N_END) => 0,
        (DATA_GEOM, ST_FLOAT32) => 16,
        (DATA_POS, ST_FLOAT32) => 16,
        (DATA_POS, ST_SINT32_XYZ) => 12,
        (DATA_POS, ST_SINT16_XYZ) => 6,
        (DATA_POS, ST_SINT8_XYZ) => 3,
        (DATA_NORM | DATA_COLOR, ST_UINT16_BITPACKED) => 2,
        (DATA_UV, ST_UINT32_UV) => 8,
        (DATA_UV, ST_UINT16_UV) => 4,
        (DATA_UV, ST_UINT8_UV) => 2,
        (DATA_UV, ST_UINT16_LMUV) => 8,
        (DATA_UV, ST_UNKNOWN) => 0,
        _ => bail!("unknown stream type {data_type:#x}/{storage:#x}"),
    })
}

impl Mesh {
    pub fn decode(stream: &[u8]) -> Result<Mesh> {
        let r = Le::new(stream);
        let mut m = Mesh::default();
        let mut strips: Vec<Strip> = Vec::new();
        let mut at = 0usize;

        while at + 4 < stream.len() {
            at = (at + 3) & !3;
            if at + 4 > stream.len() {
                break;
            }
            let data_type = r.u8(at)?;
            let count = r.u8(at + 2)? as usize;
            let storage = r.u8(at + 3)?;
            at += 4;
            let st = stride(data_type, storage)?;
            if at + st * count > stream.len() {
                break;
            }
            let base = at;
            at += st * count;

            match (data_type, storage) {
                (DATA_GEOM, ST_NULL | ST_STRIP_0_END | ST_STRIP_N_END) | (DATA_UV, ST_UNKNOWN) => {}
                (DATA_GEOM, ST_FLOAT32) => {
                    strips.push(Strip {
                        face_dir: r.f32(base + 8)?,
                        draw: Vec::new(),
                    });
                }
                (DATA_POS, ST_FLOAT32) => {
                    // A single un-stripped triangle: 12 vec4s of position, normal, colour, uv.
                    let Some(strip) = strips.last_mut() else {
                        bail!("triangle before strip header")
                    };
                    strip.draw.push(true);
                    strip.face_dir = -1.0;
                    for j in 0..3 {
                        let o = base + j * 16;
                        let w = r.f32(o + 12)?;
                        let s = if w != 0.0 { 1.0 / w } else { 1.0 };
                        m.positions
                            .push([r.f32(o)? * s, r.f32(o + 4)? * s, r.f32(o + 8)? * s]);
                    }
                    for j in 3..6 {
                        let o = base + j * 16;
                        let s = r.f32(o + 12)?;
                        m.normals
                            .push([r.f32(o)? * s, r.f32(o + 4)? * s, r.f32(o + 8)? * s]);
                    }
                    for j in 6..9 {
                        let o = base + j * 16;
                        let c = |k| r.f32(o + k).map(|v| v / 255.0);
                        m.colors.push([c(0)?, c(4)?, c(8)?, c(12)?]);
                    }
                    for j in 9..12 {
                        let o = base + j * 16;
                        m.uvs.push([r.f32(o)?, r.f32(o + 4)?]);
                    }
                }
                (DATA_POS, _) => {
                    // The last vertex of every position packet is padding.
                    for i in 0..count.saturating_sub(1) {
                        let o = base + i * st;
                        let v = match storage {
                            ST_SINT8_XYZ => {
                                [r.i8(o)? as f32, r.i8(o + 1)? as f32, r.i8(o + 2)? as f32]
                            }
                            ST_SINT16_XYZ => {
                                [r.i16(o)? as f32, r.i16(o + 2)? as f32, r.i16(o + 4)? as f32]
                            }
                            _ => [r.i32(o)? as f32, r.i32(o + 4)? as f32, r.i32(o + 8)? as f32],
                        };
                        m.positions.push(v.map(|c| c * POS_SCALE));
                    }
                }
                (DATA_NORM, _) => {
                    let Some(strip) = strips.last_mut() else {
                        bail!("normals before strip header")
                    };
                    for i in 0..count {
                        let n = r.u16(base + i * 2)?;
                        m.normals.push(unpack_norm(n));
                        if i >= 2 {
                            strip.draw.push(n < 0x8000);
                        }
                    }
                }
                (DATA_COLOR, _) => {
                    for i in 0..count {
                        m.colors.push(unpack_color(r.u16(base + i * 2)?));
                    }
                }
                (DATA_UV, _) => {
                    for i in 0..count {
                        let o = base + i * st;
                        let uv = match storage {
                            ST_UINT8_UV => [r.u8(o)? as f32, r.u8(o + 1)? as f32],
                            ST_UINT16_UV | ST_UINT16_LMUV => {
                                [r.u16(o)? as f32, r.u16(o + 2)? as f32]
                            }
                            _ => [r.u32(o)? as f32, r.u32(o + 4)? as f32],
                        };
                        m.uvs.push(uv.map(|c| c * UV_SCALE));
                        if storage == ST_UINT16_LMUV {
                            m.lm_uvs.push([
                                r.u16(o + 4)? as f32 * LM_UV_SCALE,
                                r.u16(o + 6)? as f32 * LM_UV_SCALE,
                            ]);
                        }
                    }
                }
                _ => unreachable!(),
            }
        }

        // Build triangle indices from the strips.
        let mut start = 0u32;
        for s in &strips {
            if s.draw.is_empty() {
                continue;
            }
            let flip = (s.face_dir == -1.0) as u32;
            for (f, &draw) in s.draw.iter().enumerate() {
                if draw {
                    let v = start + f as u32;
                    if (f as u32 + flip) & 1 == 1 {
                        m.indices.extend([v, v + 1, v + 2]);
                    } else {
                        m.indices.extend([v + 1, v, v + 2]);
                    }
                }
            }
            start += s.draw.len() as u32 + 2;
        }

        // Pad optional attributes so every array matches the position count.
        let n = m.positions.len();
        m.normals.resize(n, [0.0, 1.0, 0.0]);
        m.colors.resize(n, [1.0; 4]);
        m.uvs.resize(n, [0.0; 2]);
        if !m.lm_uvs.is_empty() {
            m.lm_uvs.resize(n, [0.0; 2]);
        }
        // Drop any triangle that references a vertex we never saw (truncated streams).
        m.indices = m
            .indices
            .chunks_exact(3)
            .filter(|t| t.iter().all(|&i| (i as usize) < n))
            .flatten()
            .copied()
            .collect();
        Ok(m)
    }
}
