//! Front-end art from the disc: the menu/HUD texture banks (`STATIC`, `SELECT`,
//! `TITLE`, ...) and the bitmap fonts described by `FONTS/*.FNT`.

use crate::fs::GameFs;
use gdl_formats::texture::{self, Image};
use gdl_formats::Objects;
use std::collections::HashMap;

/// A texture usable by the 2D overlay: an index into the overlay's texture list and
/// its size in pixels. Index 0 is a plain white texture for solid rectangles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tex {
    pub id: u32,
    pub w: u32,
    pub h: u32,
}

impl Tex {
    pub const WHITE: Tex = Tex { id: 0, w: 1, h: 1 };
}

/// A proportional bitmap font: glyph rectangles in one texture.
#[derive(Clone, Debug)]
pub struct Font {
    pub tex: Tex,
    /// Cell height in texture pixels.
    pub height: f32,
    /// char -> (x, top, width) in texture pixels.
    pub glyphs: HashMap<char, (f32, f32, f32)>,
    /// Advance for characters without a glyph (space).
    pub space: f32,
    /// Extra space between letters, in texture pixels.
    pub gap: f32,
}

impl Font {
    /// FNT layout: u32 0, u32 cell height, u32 0, then 16-byte records
    /// (u32 char, u32 width, u32 x, u32 top).
    fn parse(data: &[u8], tex: Tex) -> Option<Font> {
        let u = |o: usize| -> Option<u32> {
            Some(u32::from_le_bytes(data.get(o..o + 4)?.try_into().ok()?))
        };
        let height = u(4)? as f32;
        let mut glyphs = HashMap::new();
        let mut o = 12;
        while o + 16 <= data.len() {
            let (c, w, x, y) = (u(o)?, u(o + 4)?, u(o + 8)?, u(o + 12)?);
            if let Some(ch) = char::from_u32(c) {
                glyphs.insert(ch, (x as f32, y as f32, w as f32));
            }
            o += 16;
        }
        let space = glyphs.get(&'n').map(|g| g.2).unwrap_or(height * 0.5) * 0.8;
        Some(Font {
            tex,
            height,
            glyphs,
            space,
            gap: 0.0,
        })
    }

    /// Glyph for a character; fonts without lower case fall back to upper case.
    pub fn glyph(&self, ch: char) -> Option<(f32, f32, f32)> {
        self.glyphs
            .get(&ch)
            .or_else(|| self.glyphs.get(&ch.to_ascii_uppercase()))
            .copied()
    }

    /// Width in pixels of `s` drawn `px` pixels tall.
    pub fn width(&self, s: &str, px: f32) -> f32 {
        let k = px / self.height;
        s.chars()
            .map(|c| self.glyph(c).map(|g| g.2 + self.gap).unwrap_or(self.space))
            .sum::<f32>()
            * k
    }
}

/// The game's font slots (gFontDefs8x8 / gFontDefs in btext.c): FNT and texture
/// name, and the advance of a space.
pub const FONT_DEFS: [(&str, f32); 10] = [
    ("FONT8X8", 8.0),
    ("8HIFONTS", 8.0),
    ("BARS", 4.0),
    ("ARROWS", 8.0),
    ("SCORE", 9.0),
    ("SCORATT", 12.0),
    ("FONT32", 16.0),
    ("INITIALS", 12.0),
    ("SCORATT8", 8.0),
    ("NAMEFONT", 8.0),
];
pub const FONT_8X8: usize = 0;
pub const FONT_8HI: usize = 1;
pub const FONT_32: usize = 6;
pub const FONT_INITIALS: usize = 7;

/// Decode a model directory's named textures (`<dir>/objects.ngc` + `textures.ngc`),
/// for banks loaded on demand (the map screen's `MAPS/level<name>`). Names are
/// upper-cased bitmap-def names; the first def of a name wins.
pub fn load_bank_images(fs: &GameFs, dir: &str) -> anyhow::Result<Vec<(String, Image)>> {
    let objs = Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?)?;
    let tex = fs.read(&format!("{dir}/textures.ngc"))?;
    let mut out: Vec<(String, Image)> = Vec::new();
    for d in &objs.bitmap_defs {
        let name = d.name.to_ascii_uppercase();
        if name.is_empty() || out.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let Some(b) = objs.bitmaps.get(d.tex_index as usize) else {
            continue;
        };
        match texture::decode_ngc(b, &tex) {
            Ok(img) => out.push((name, img)),
            Err(e) => log::warn!("{dir}: {name}: {e:#}"),
        }
    }
    Ok(out)
}

/// Texture banks loaded for the front end, in lookup order.
const BANKS: [&str; 5] = ["STATIC", "SELECT", "TITLE", "SONY", "GROUP1"];

/// Every front-end texture by name plus the fonts, ready for upload.
#[derive(Default)]
pub struct UiAssets {
    /// Decoded images; index i + 1 is texture id i + 1 (id 0 is white).
    pub images: Vec<Image>,
    /// Upper-cased bitmap-def name -> (bank, bitmap index).
    names: HashMap<String, (usize, usize)>,
    /// (bank, bitmap index) -> texture, for animation frames that follow a named one.
    bitmaps: HashMap<(usize, usize), Tex>,
    /// Font slots as the game numbers them (see [`FONT_DEFS`]).
    pub fonts: Vec<Option<Font>>,
    /// FONT32 laid out with the glow texture, for DrawGlowText's glow pass.
    pub font32_glow: Option<Font>,
    /// HUD fonts, tightened to their ink (the HUD is not yet laid out as the game does).
    pub font32: Option<Font>,
    pub font8: Option<Font>,
    pub name_font: Option<Font>,
}

impl UiAssets {
    pub fn load(fs: &GameFs) -> UiAssets {
        let mut ui = UiAssets::default();
        for (bank, dir) in BANKS.iter().enumerate() {
            if let Err(e) = ui.load_dir(fs, bank, dir) {
                log::warn!("front-end textures in {dir}: {e:#}");
            }
        }
        let font = |ui: &UiAssets, file: &str, tex: &str| {
            let data = fs.read(&format!("FONTS/{file}.FNT")).ok()?;
            Font::parse(&data, ui.get(tex)?)
        };
        // Laid out as MBDrawText does: each glyph advances by its FNT width and a
        // space by the slot's space width.
        ui.fonts = FONT_DEFS
            .iter()
            .map(|&(name, space)| font(&ui, name, name).map(|f| Font { space, ..f }))
            .collect();
        ui.font32_glow = ui.fonts[FONT_32].clone().and_then(|f| {
            Some(Font {
                tex: ui.get("FONT32_GLOW")?,
                ..f
            })
        });
        ui.font32 = font(&ui, "FONT32", "FONT32").map(|f| ui.tighten(f));
        ui.font8 = font(&ui, "8HIFONTS", "8HIFONTS").map(|f| ui.tighten(f));
        ui.name_font = font(&ui, "NAMEFONT", "NAMEFONT").map(|f| ui.tighten(f));
        log::info!(
            "front end: {} textures, {} fonts",
            ui.images.len(),
            ui.fonts.iter().flatten().count()
        );
        ui
    }

    fn load_dir(&mut self, fs: &GameFs, bank: usize, dir: &str) -> anyhow::Result<()> {
        let objs = Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?)?;
        let tex = fs.read(&format!("{dir}/textures.ngc"))?;
        for (i, b) in objs.bitmaps.iter().enumerate() {
            let Ok(img) = texture::decode_ngc(b, &tex) else {
                continue;
            };
            let t = Tex {
                id: self.images.len() as u32 + 1,
                w: img.width,
                h: img.height,
            };
            self.images.push(img);
            self.bitmaps.insert((bank, i), t);
        }
        for d in &objs.bitmap_defs {
            if !d.name.is_empty() {
                self.names
                    .entry(d.name.to_ascii_uppercase())
                    .or_insert((bank, d.tex_index as usize));
            }
        }
        // The Archer's name banner is the one unnamed banner-sized texture.
        if dir == "SELECT" {
            self.names.entry("ARC_NAME".into()).or_insert((bank, 0));
        }
        Ok(())
    }

    /// Shrink each glyph to the columns that actually have ink: the FNT widths
    /// include padding, which spaces small text out.
    fn tighten(&self, mut f: Font) -> Font {
        let Some(img) = self.images.get(f.tex.id as usize - 1) else {
            return f;
        };
        let alpha = |x: u32, y: u32| -> u8 {
            if x >= img.width || y >= img.height {
                return 0;
            }
            img.rgba[((y * img.width + x) * 4 + 3) as usize]
        };
        let h = f.height as u32;
        for g in f.glyphs.values_mut() {
            let (x0, y0, w) = (g.0 as u32, g.1 as u32, g.2 as u32);
            let ink = |x: u32| (y0..y0 + h).any(|y| alpha(x, y) > 40);
            let Some(l) = (x0..x0 + w).find(|&x| ink(x)) else {
                continue;
            };
            let r = (x0..x0 + w).rev().find(|&x| ink(x)).unwrap_or(l);
            *g = (l as f32, g.1, (r - l + 1) as f32);
        }
        f.space = f.height * 0.3;
        f.gap = (f.height * 0.1).max(1.0);
        f
    }

    /// Texture by bitmap-def name (case-insensitive, as MBOX_FindTexture).
    pub fn get(&self, name: &str) -> Option<Tex> {
        self.frame(name, 0)
    }

    /// Frame `n` of an animated texture: the n-th bitmap after the named one.
    pub fn frame(&self, name: &str, n: usize) -> Option<Tex> {
        let &(bank, i) = self.names.get(&name.to_ascii_uppercase())?;
        self.bitmaps.get(&(bank, i + n)).copied()
    }

    /// Font slot `i` (see [`FONT_DEFS`]).
    pub fn font(&self, i: usize) -> Option<&Font> {
        self.fonts.get(i)?.as_ref()
    }

    /// First texture whose name starts with each prefix in turn.
    pub fn find(&self, prefixes: &[&str]) -> Option<Tex> {
        prefixes.iter().find_map(|p| {
            self.get(p).or_else(|| {
                let p = p.to_ascii_uppercase();
                let mut v: Vec<&String> = self.names.keys().filter(|n| n.starts_with(&p)).collect();
                v.sort();
                self.get(v.first()?)
            })
        })
    }
}
