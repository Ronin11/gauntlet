//! GameCube texture decoding (`textures.ngc` pixel data, headers from `objects.ngc`).
//!
//! The GameCube build stores textures in GX tile order with big-endian 16-bit
//! colours. 16-bit direct colour and palettes are RGB5A3 ("ABGR_3555" in the
//! community tools): high bit set = opaque RGB555, clear = ARGB3444.

use crate::objects::{bitmap_flags, Bitmap};
use anyhow::{bail, Result};

/// Pixel format ids stored in bitmap headers.
pub mod fmt {
    pub const ABGR_1555: u8 = 0;
    pub const XBGR_1555: u8 = 1;
    pub const ABGR_8888: u8 = 2;
    pub const XBGR_8888: u8 = 3;
    pub const ABGR_1555_IDX_4: u8 = 16;
    pub const XBGR_1555_IDX_4: u8 = 17;
    pub const ABGR_3555_IDX_4_NGC: u8 = 18;
    pub const ABGR_8888_IDX_4: u8 = 34;
    pub const XBGR_8888_IDX_4: u8 = 35;
    pub const ABGR_1555_IDX_8: u8 = 48;
    pub const XBGR_1555_IDX_8: u8 = 49;
    pub const ABGR_3555_IDX_8_NGC: u8 = 50;
    pub const IA_8_IDX_88: u8 = 56;
    pub const ABGR_8888_IDX_8: u8 = 66;
    pub const XBGR_8888_IDX_8: u8 = 67;
    pub const A_8_IDX_8: u8 = 130;
    pub const I_8_IDX_8: u8 = 131;
    pub const A_4_IDX_4: u8 = 146;
    pub const I_4_IDX_4: u8 = 147;
}

#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8.
    pub rgba: Vec<u8>,
}

impl Image {
    /// White radial gradient with soft alpha falloff.
    pub fn glow(width: u32, height: u32) -> Image {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let dx = (x as f32 + 0.5) / width as f32 * 2.0 - 1.0;
                let dy = (y as f32 + 0.5) / height as f32 * 2.0 - 1.0;
                let a = (1.0 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                rgba.extend_from_slice(&[255, 240, 200, (a * a * 255.0) as u8]);
            }
        }
        Image {
            width,
            height,
            rgba,
        }
    }

    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Image {
        Image {
            width,
            height,
            rgba: rgba.repeat((width * height) as usize),
        }
    }
}

/// RGB5A3 to RGBA8.
pub fn rgb5a3(c: u16) -> [u8; 4] {
    if c & 0x8000 != 0 {
        let r = ((c >> 10) & 31) as u8;
        let g = ((c >> 5) & 31) as u8;
        let b = (c & 31) as u8;
        [
            (r << 3) | (r >> 2),
            (g << 3) | (g >> 2),
            (b << 3) | (b >> 2),
            255,
        ]
    } else {
        let a = ((c >> 12) & 7) as u8;
        let r = ((c >> 8) & 15) as u8;
        let g = ((c >> 4) & 15) as u8;
        let b = (c & 15) as u8;
        [r * 17, g * 17, b * 17, (a << 5) | (a << 2) | (a >> 1)]
    }
}

/// Bits per pixel and palette entry count for a format.
fn layout(format: u8) -> Result<(u32, usize)> {
    use fmt::*;
    Ok(match format {
        ABGR_1555 | XBGR_1555 => (16, 0),
        ABGR_8888 | XBGR_8888 => (32, 0),
        ABGR_1555_IDX_4 | XBGR_1555_IDX_4 | ABGR_3555_IDX_4_NGC => (4, 16),
        ABGR_8888_IDX_4 | XBGR_8888_IDX_4 => (4, 16),
        ABGR_1555_IDX_8 | XBGR_1555_IDX_8 | ABGR_3555_IDX_8_NGC => (8, 256),
        ABGR_8888_IDX_8 | XBGR_8888_IDX_8 => (8, 256),
        IA_8_IDX_88 => (16, 0),
        A_8_IDX_8 | I_8_IDX_8 => (8, 0),
        A_4_IDX_4 | I_4_IDX_4 => (4, 0),
        _ => bail!("unknown pixel format {format}"),
    })
}

/// Size in bytes of the palette + top mip for a bitmap.
pub fn data_size(b: &Bitmap) -> Result<usize> {
    let (bpp, pal) = layout(b.format)?;
    let pal_bytes = pal
        * if matches!(b.format, 34 | 35 | 66 | 67) {
            4
        } else {
            2
        };
    Ok(pal_bytes + (b.width as usize * b.height as usize * bpp as usize).div_ceil(8))
}

/// Tile size in pixels for a given bits-per-pixel (GX layout).
fn tile_dims(bpp: u32) -> (u32, u32) {
    match bpp {
        4 => (8, 8),
        8 => (8, 4),
        16 => (4, 4),
        _ => (4, 2),
    }
}

/// Map linear storage index `i` to an (x, y) pixel coordinate in a tiled image.
/// Tiles narrower than the image are clipped to the image size (small mips).
fn untile(i: u32, w: u32, h: u32, bpp: u32) -> (u32, u32) {
    let (tw, th) = tile_dims(bpp);
    let tw = tw.min(w);
    let th = th.min(h);
    let per_tile = tw * th;
    let tile = i / per_tile;
    let within = i % per_tile;
    let tiles_x = w.div_ceil(tw);
    let (tx, ty) = (tile % tiles_x, tile / tiles_x);
    (tx * tw + within % tw, ty * th + within / tw)
}

/// Decode the top mip level of a GameCube bitmap from the textures file.
pub fn decode_ngc(b: &Bitmap, textures: &[u8]) -> Result<Image> {
    let (w, h) = (b.width as u32, b.height as u32);
    if w == 0 || h == 0 || w > 1024 || h > 1024 {
        bail!("bad texture size {w}x{h}");
    }
    if b.flags & bitmap_flags::EXTERNAL != 0 || b.frame_count > 0 {
        // Animated/external textures (flames, glows) get their frames from texture
        // animations elsewhere; stand in with a soft radial glow.
        return Ok(Image::glow(w, h));
    }
    let (bpp, pal_count) = layout(b.format)?;
    let start = b.tex_pointer as usize;
    let need = data_size(b)?;
    let Some(data) = textures.get(start..start + need) else {
        bail!(
            "texture data {start:#x}+{need:#x} past end of file ({:#x})",
            textures.len()
        );
    };
    let be16 = |o: usize| u16::from_be_bytes([data[o], data[o + 1]]);

    let wide_pal = matches!(b.format, 34 | 35 | 66 | 67);
    let palette: Vec<[u8; 4]> = (0..pal_count)
        .map(|i| {
            if wide_pal {
                let o = i * 4;
                [data[o + 1], data[o + 2], data[o + 3], data[o]]
            } else {
                rgb5a3(be16(i * 2))
            }
        })
        .collect();
    let pix = &data[pal_count * if wide_pal { 4 } else { 2 }..];

    let mono = matches!(
        b.format,
        fmt::A_8_IDX_8 | fmt::I_8_IDX_8 | fmt::A_4_IDX_4 | fmt::I_4_IDX_4
    );
    // The "A" monochrome formats are used for lightmaps on GameCube, so expose the
    // value in both the colour and alpha channels.
    let alpha_mono = matches!(b.format, fmt::A_8_IDX_8 | fmt::A_4_IDX_4);

    let mut out = vec![0u8; (w * h * 4) as usize];
    for i in 0..w * h {
        let (x, y) = untile(i, w, h, bpp);
        if x >= w || y >= h {
            continue;
        }
        let px: [u8; 4] = match bpp {
            4 => {
                let byte = pix[(i / 2) as usize];
                let v = if i & 1 == 0 { byte >> 4 } else { byte & 15 };
                if mono {
                    let v = v * 17;
                    if alpha_mono {
                        [v, v, v, v]
                    } else {
                        [v, v, v, 255]
                    }
                } else {
                    palette[v as usize]
                }
            }
            8 => {
                let v = pix[i as usize];
                if mono {
                    if alpha_mono {
                        [v, v, v, v]
                    } else {
                        [v, v, v, 255]
                    }
                } else {
                    palette[v as usize]
                }
            }
            16 => {
                let c = u16::from_be_bytes([pix[i as usize * 2], pix[i as usize * 2 + 1]]);
                if b.format == fmt::IA_8_IDX_88 {
                    let (a, l) = ((c >> 8) as u8, c as u8);
                    [l, l, l, a]
                } else {
                    rgb5a3(c)
                }
            }
            _ => {
                let o = i as usize * 4;
                [pix[o + 1], pix[o + 2], pix[o + 3], pix[o]]
            }
        };
        let o = ((y * w + x) * 4) as usize;
        out[o..o + 4].copy_from_slice(&px);
    }
    Ok(Image {
        width: w,
        height: h,
        rgba: out,
    })
}
