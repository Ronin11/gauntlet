//! The original's 2D drawing primitives on its 512x384 virtual screen (mb_blit.c,
//! mb_font.c, btext.c), mapped 4:3 and centred into the window.
//!
//! Conventions follow the game: blit `fade` is 0 (opaque) to 255 (invisible); font
//! alpha is MBSetFontAlpha's 0 (opaque) to 255 (invisible) and only reaches half
//! transparency at 255 >> 1; colours are 0xRRGGBB; a negative text x centres the
//! string on -x.

use crate::hud::Canvas;
use crate::ui::{Font, Tex, UiAssets, FONT_32};

/// The glow colour, radius and period of DrawGlowText (btext.c globals).
pub const GLOW_COLOR: u32 = 0x8200EA;
const GLOW_RADIUS: i32 = 40;
const GLOW_PERIOD: i32 = 5;
/// mb_font.c widens each glyph of a glow pass by this many virtual pixels a side.
const GLOW_PAD: f32 = 2.0;

pub fn rgb(c: u32, opacity: f32) -> [f32; 4] {
    [
        ((c >> 16) & 0xFF) as f32 / 255.0,
        ((c >> 8) & 0xFF) as f32 / 255.0,
        (c & 0xFF) as f32 / 255.0,
        opacity,
    ]
}

/// Opacity of a blit with MBBlitSetAlpha `fade` (GX alpha 128 - fade/2).
pub fn fade_opacity(fade: i32) -> f32 {
    (128 - fade.clamp(0, 256) / 2) as f32 / 128.0
}

/// Opacity of text with MBSetFontAlpha `a` (stored as 128 - a/2).
pub fn font_opacity(a: i32) -> f32 {
    (128 - (a.clamp(0, 255) >> 1)) as f32 / 128.0
}

/// The glow pulse: a 0..40..0 triangle over 80 ticks, then 5 ticks at 0, turned into
/// the glow pass's font alpha (`0x7F - a/2`).
pub fn glow_alpha(ticks: u32) -> i32 {
    let span = GLOW_RADIUS * 2;
    let mut phase = (ticks % (GLOW_PERIOD + span) as u32) as i32;
    if phase > span {
        phase = 0;
    } else if phase > GLOW_RADIUS {
        phase = span - phase;
    }
    let a = (GLOW_RADIUS + phase * 0xFF - 1) / GLOW_RADIUS;
    0x7F - a / 2
}

/// A frame's 2D drawing on the virtual screen.
pub struct Scr<'a> {
    pub c: &'a mut Canvas,
    pub ui: &'a UiAssets,
    /// The game's 60 Hz tick counter (pbLoad), which drives the glow pulses.
    pub ticks: u32,
    x0: f32,
    y0: f32,
    k: f32,
}

impl<'a> Scr<'a> {
    pub fn new(c: &'a mut Canvas, ui: &'a UiAssets, ticks: u32) -> Self {
        let k = (c.width() / 512.0).min(c.height() / 384.0);
        let (x0, y0) = (
            (c.width() - 512.0 * k) * 0.5,
            (c.height() - 384.0 * k) * 0.5,
        );
        Scr {
            c,
            ui,
            ticks,
            x0,
            y0,
            k,
        }
    }

    /// Window pixels per virtual pixel.
    pub fn scale(&self) -> f32 {
        self.k
    }

    /// Black over everything outside the 512x384 screen.
    pub fn bars(&mut self, opacity: f32) {
        let (w, h, x0, y0) = (self.c.width(), self.c.height(), self.x0, self.y0);
        let black = [0.0, 0.0, 0.0, opacity];
        self.c.rect(0.0, 0.0, x0, h, black);
        self.c.rect(w - x0, 0.0, x0, h, black);
        self.c.rect(x0, 0.0, w - 2.0 * x0, y0, black);
        self.c.rect(x0, h - y0, w - 2.0 * x0, y0, black);
    }

    /// A solid rectangle in virtual pixels.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let k = self.k;
        self.c
            .rect(self.x0 + x * k, self.y0 + y * k, w * k, h * k, color);
    }

    /// A texture stretched over (x, y, w, h) virtual pixels, tinted.
    pub fn blit_tint(&mut self, t: Tex, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let k = self.k;
        self.c
            .sprite(t, self.x0 + x * k, self.y0 + y * k, w * k, h * k, color);
    }

    /// A texture over (x, y, w, h) with MBBlitSetAlpha `fade`.
    pub fn blit(&mut self, t: Tex, x: f32, y: f32, w: f32, h: f32, fade: i32) {
        self.blit_tint(t, x, y, w, h, [1.0, 1.0, 1.0, fade_opacity(fade)]);
    }

    /// A blit with flag 0x4000: a light sweeping diagonally across the screen every
    /// 150 ticks brightens each corner (mbBlitCalcLight).
    pub fn blit_shimmer(&mut self, t: Tex, x: f32, y: f32, w: f32, h: f32, fade: i32) {
        let a = fade_opacity(fade);
        let phase = 7.0 * ((self.ticks % 150) as f32 / 150.0);
        let corner = |cx: f32, cy: f32| {
            // Blit colours are 0x80 = 1.0; the light adds up to 127.
            let l = 1.0 + calc_light(cx, 384.0 - cy, phase) as f32 / 128.0;
            [l, l, l, a]
        };
        let colors = [
            corner(x, y),
            corner(x + w, y),
            corner(x + w, y + h),
            corner(x, y + h),
        ];
        let (hu, hv) = (0.5 / t.w as f32, 0.5 / t.h as f32);
        let k = self.k;
        self.c.quad_colors(
            t,
            self.x0 + x * k,
            self.y0 + y * k,
            w * k,
            h * k,
            [hu, hv, 1.0 - hu, 1.0 - hv],
            colors,
        );
    }

    /// A named texture at (x, y) at its own size (MBNewBlit); false if missing.
    pub fn blit_named(&mut self, name: &str, x: f32, y: f32, fade: i32) -> bool {
        match self.ui.get(name) {
            Some(t) => {
                self.blit(t, x, y, t.w as f32, t.h as f32, fade);
                true
            }
            None => false,
        }
    }

    /// A full-screen picture split into four textures `<prefix>00..03` laid out as
    /// the game's Screen2D does: (0,0), (256,0), (0,256), (256,256).
    pub fn quad_screen(&mut self, prefix: &str, fade: i32) {
        for (i, (x, y)) in [(0.0, 0.0), (256.0, 0.0), (0.0, 256.0), (256.0, 256.0)]
            .into_iter()
            .enumerate()
        {
            self.blit_named(&format!("{prefix}{i:02}"), x, y, fade);
        }
    }

    /// Width of `s` in font slot `font` at `scale` (MBFontStringWidth).
    pub fn text_width(&self, font: usize, scale: f32, s: &str) -> f32 {
        self.ui
            .font(font)
            .map_or(0.0, |f| self.text_width_font(f, scale, s))
    }

    /// Height of font slot `font` at `scale` (FontHeight).
    pub fn font_height(&self, font: usize, scale: f32) -> f32 {
        self.ui
            .font(font)
            .map_or(8.0 * scale, |f| (f.height * scale).floor())
    }

    /// DrawTextKeepScale: `s` in font slot `font` with its top-left at (x, y), or
    /// centred on -x when x is negative. Returns the width.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        scale: f32,
        x: f32,
        y: f32,
        font: usize,
        color: u32,
        alpha: i32,
        s: &str,
    ) -> f32 {
        let Some(f) = self.ui.font(font) else {
            return 0.0;
        };
        self.draw_text(
            f,
            f.tex,
            scale,
            x,
            y,
            rgb(color, font_opacity(alpha)),
            0.0,
            s,
        )
    }

    /// Lay out `s` as mb_font.c does; `pad` widens each glyph quad (glow passes).
    #[allow(clippy::too_many_arguments)]
    fn draw_text(
        &mut self,
        f: &Font,
        tex: Tex,
        scale: f32,
        x: f32,
        y: f32,
        color: [f32; 4],
        pad: f32,
        s: &str,
    ) -> f32 {
        let w = self.text_width_font(f, scale, s);
        let mut x = if x < 0.0 { -x - (w * 0.5).floor() } else { x };
        let start = x;
        let (tw, th) = (tex.w as f32, tex.h as f32);
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '*' {
                if let Some(icon) = chars.peek().and_then(|c| icon_name(*c)) {
                    chars.next();
                    // Button icons sit inline: (height + 4) square, raised 2 pixels.
                    let size = (f.height + 4.0) * scale;
                    if pad == 0.0 {
                        if let Some(t) = self.ui.get(icon) {
                            self.blit_tint(t, x, y - 2.0, size, size, [1.0; 4]);
                        }
                    }
                    x += (f.height + 2.0) * scale;
                    continue;
                }
            }
            match f.glyph(ch) {
                Some((gx, gy, gw)) if gw > 0.0 => {
                    let uv = [gx / tw, gy / th, (gx + gw) / tw, (gy + f.height) / th];
                    let k = self.k;
                    self.c.quad(
                        tex,
                        self.x0 + (x - pad) * k,
                        self.y0 + (y - pad) * k,
                        (gw * scale + 2.0 * pad) * k,
                        (f.height * scale + 2.0 * pad) * k,
                        uv,
                        color,
                    );
                    x += gw * scale;
                }
                _ if ch == ' ' => x += f.space * scale,
                _ => {}
            }
        }
        x - start
    }

    fn text_width_font(&self, f: &Font, scale: f32, s: &str) -> f32 {
        let mut w = 0.0;
        let mut chars = s.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '*' && chars.peek().is_some_and(|c| icon_name(*c).is_some()) {
                chars.next();
                w += (f.height + 2.0) * scale;
                continue;
            }
            w += match f.glyph(ch) {
                Some(g) if g.2 > 0.0 => g.2 * scale,
                _ if ch == ' ' => f.space * scale,
                _ => 0.0,
            };
        }
        w
    }

    /// The glow pass alone: FONT32 glyphs from FONT32_GLOW, widened, in `color` at
    /// font alpha `alpha`.
    pub fn glow_pass(&mut self, scale: f32, x: f32, y: f32, color: u32, alpha: i32, s: &str) {
        if let Some(g) = &self.ui.font32_glow {
            let g = g.clone();
            let c = rgb(color, font_opacity(alpha));
            self.draw_text(&g, g.tex, scale, x, y, c, GLOW_PAD, s);
        }
    }

    /// DrawGlowText: white FONT32 over a pulsing purple glow.
    pub fn glow_text(&mut self, scale: f32, x: f32, y: f32, s: &str) {
        let a = glow_alpha(self.ticks);
        self.glow_pass(scale, x, y, GLOW_COLOR, a, s);
        self.text(scale, x, y, FONT_32, 0xFFFFFF, 0, s);
    }

    /// DrawGlowTextMLines: one glow line per `\n`, 32 * scale apart; a negative y
    /// centres the block vertically on -y.
    pub fn glow_text_lines(&mut self, scale: f32, x: f32, y: f32, s: &str) {
        let lh = self.font_height(FONT_32, scale);
        let lines: Vec<&str> = s.split('\n').collect();
        let mut y = if y < 0.0 {
            -(y + (lh * lines.len() as f32 / 2.0).floor())
        } else {
            y
        };
        for l in lines {
            self.glow_text(scale, x, y, l);
            y += lh;
        }
    }
}

/// mbBlitCalcLight: the light (0..127) at screen point (x, y), y measured from the
/// bottom, for sweep phase 0..7.
fn calc_light(x: f32, y: f32, phase: f32) -> i32 {
    let mut v = (x + y) / (512.0 + 384.0);
    let p = phase;
    v = if v < 0.5 {
        if p < 1.0 {
            -(4.0 * v - p)
        } else if p < 2.0 {
            2.0 * (v * (p - 3.0)) + 1.0
        } else if p < 3.0 {
            2.0 * ((3.0 - p) * (0.5 - v)) + 2.0 * ((p - 2.0) * v)
        } else if p < 4.0 {
            2.0 * v + 2.0 * ((3.0 - p) * (0.5 - v))
        } else if p < 5.0 {
            2.0 * ((3.0 - p) * (0.5 - v)) + 2.0 * ((5.0 - p) * v)
        } else {
            0.0
        }
    } else {
        let v = 1.0 - v;
        if p < 2.0 {
            0.0
        } else if p < 3.0 {
            2.0 * ((p - 4.0) * (0.5 - v)) + 2.0 * ((p - 2.0) * v)
        } else if p < 4.0 {
            2.0 * v + 2.0 * ((p - 4.0) * (0.5 - v))
        } else if p < 5.0 {
            2.0 * ((p - 4.0) * (0.5 - v)) + 2.0 * ((5.0 - p) * v)
        } else if p < 6.0 {
            2.0 * (v * (4.0 - p)) + 1.0
        } else {
            -(4.0 * v - (7.0 - p))
        }
    };
    (v * 127.0).clamp(0.0, 127.0) as i32
}

/// The button icon drawn for `*<c>` in text (mb_font.c).
fn icon_name(c: char) -> Option<&'static str> {
    Some(match c {
        'X' => "BUTTON_X",
        'T' => "BUTTON_TRI",
        'O' => "BUTTON_O",
        'S' => "BUTTON_SQ",
        'L' => "BUTTON_L",
        'R' => "BUTTON_R",
        'U' => "BUTTON_U",
        'D' => "BUTTON_D",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glow_pulses_between_half_and_full() {
        let alphas: Vec<i32> = (0..85).map(glow_alpha).collect();
        assert_eq!(alphas[0], 0x7F);
        assert_eq!(alphas[40], 0);
        assert_eq!(alphas[84], 0x7F);
        assert!((font_opacity(0x7F) - 0.5).abs() < 0.02);
    }
}
