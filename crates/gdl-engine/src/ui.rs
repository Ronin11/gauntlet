//! Front-end art from the disc: the menu/HUD textures in `STATIC` and `SELECT`, and the
//! bitmap fonts described by `FONTS/*.FNT`.

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
        })
    }

    /// Glyph for a character; fonts without lower case fall back to upper case.
    pub fn glyph(&self, ch: char) -> Option<(f32, f32, f32)> {
        self.glyphs
            .get(&ch)
            .or_else(|| self.glyphs.get(&ch.to_ascii_uppercase()))
            .copied()
    }

    /// Space between letters, in texture pixels.
    pub fn gap(&self) -> f32 {
        (self.height * 0.1).max(1.0)
    }

    /// Width in pixels of `s` drawn `px` pixels tall.
    pub fn width(&self, s: &str, px: f32) -> f32 {
        let k = px / self.height;
        s.chars()
            .map(|c| {
                self.glyph(c)
                    .map(|g| g.2 + self.gap())
                    .unwrap_or(self.space)
            })
            .sum::<f32>()
            * k
    }
}

/// Every front-end texture by name plus the fonts, ready for upload.
#[derive(Default)]
pub struct UiAssets {
    /// Decoded images; index i + 1 is texture id i + 1 (id 0 is white).
    pub images: Vec<Image>,
    names: HashMap<String, Tex>,
    /// The big menu font, and its blurred "glow" twin used as an outline.
    pub font32: Option<Font>,
    pub font32_glow: Option<Font>,
    /// Small fonts for the HUD.
    pub font8: Option<Font>,
    pub name_font: Option<Font>,
}

impl UiAssets {
    pub fn load(fs: &GameFs) -> UiAssets {
        let mut ui = UiAssets::default();
        for dir in ["STATIC", "SELECT"] {
            if let Err(e) = ui.load_dir(fs, dir) {
                log::warn!("front-end textures in {dir}: {e:#}");
            }
        }
        let font = |ui: &UiAssets, file: &str, tex: &str| {
            let data = fs.read(&format!("FONTS/{file}")).ok()?;
            Font::parse(&data, ui.get(tex)?)
        };
        ui.font32 = font(&ui, "FONT32.FNT", "FONT32").map(|f| ui.tighten(f));
        ui.font32_glow = font(&ui, "FONT32.FNT", "FONT32_GLOW");
        ui.font8 = font(&ui, "8Hifonts.fnt", "8HIFONTS").map(|f| ui.tighten(f));
        ui.name_font = font(&ui, "NAMEFONT.FNT", "NAMEFONT").map(|f| ui.tighten(f));
        // The glow layer must line up with the lettering it outlines.
        if let (Some(f), Some(g)) = (&ui.font32, &mut ui.font32_glow) {
            g.glyphs = f.glyphs.clone();
            g.space = f.space;
        }
        log::info!(
            "front end: {} textures, fonts {}",
            ui.images.len(),
            [
                ui.font32.is_some(),
                ui.font8.is_some(),
                ui.name_font.is_some()
            ]
            .iter()
            .filter(|b| **b)
            .count()
        );
        ui
    }

    fn load_dir(&mut self, fs: &GameFs, dir: &str) -> anyhow::Result<()> {
        let objs = Objects::parse(&fs.read(&format!("{dir}/objects.ngc"))?)?;
        let tex = fs.read(&format!("{dir}/textures.ngc"))?;
        let mut names: HashMap<u16, &str> = HashMap::new();
        for d in &objs.bitmap_defs {
            names.entry(d.tex_index).or_insert(d.name.as_str());
        }
        for (i, b) in objs.bitmaps.iter().enumerate() {
            let Ok(img) = texture::decode_ngc(b, &tex) else {
                continue;
            };
            // The Archer's name banner is the one unnamed banner-sized texture.
            let name = match names.get(&(i as u16)) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ if dir == "SELECT" && i == 0 => "ARC_NAME".to_string(),
                _ => continue,
            };
            let t = Tex {
                id: self.images.len() as u32 + 1,
                w: img.width,
                h: img.height,
            };
            self.images.push(img);
            self.names.entry(name).or_insert(t);
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
        f
    }

    pub fn get(&self, name: &str) -> Option<Tex> {
        self.names.get(name).copied()
    }

    /// First texture whose name starts with each prefix in turn.
    pub fn find(&self, prefixes: &[&str]) -> Option<Tex> {
        prefixes.iter().find_map(|p| {
            self.get(p).or_else(|| {
                let mut v: Vec<(&String, &Tex)> = self
                    .names
                    .iter()
                    .filter(|(n, _)| n.starts_with(p))
                    .collect();
                v.sort_by_key(|(n, _)| *n);
                v.first().map(|(_, t)| **t)
            })
        })
    }
}
