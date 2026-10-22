//! 2D overlay drawn after the 3D scene: textured quads (the disc's menu art and
//! fonts, see [`crate::ui`]), solid rectangles, and an 8x8 fallback font.

use crate::fs::GameFs;
use crate::game::{Game, PlayerState};
use crate::screen::Scr;
use crate::ui::{Font, Tex, UiAssets};
use bytemuck::{Pod, Zeroable};
use gdl_formats::texture::Image;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct V {
    pos: [f32; 2],
    uv: [f32; 2],
    color: [f32; 4],
}

const SHADER: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@vertex fn vs(@location(0) p: vec2<f32>, @location(1) uv: vec2<f32>, @location(2) c: vec4<f32>) -> VsOut {
    var o: VsOut; o.pos = vec4<f32>(p, 0.0, 1.0); o.uv = uv; o.color = c; return o;
}
@fragment fn fs(v: VsOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, v.uv) * v.color;
}
"#;

/// Textures uploaded at runtime from one model directory, by upper-cased name.
#[derive(Debug, Default)]
pub struct TexBank {
    pub textures: HashMap<String, Tex>,
}

impl TexBank {
    /// Texture by bitmap-def name (case-insensitive, as MBOX_FindTexture).
    pub fn get(&self, name: &str) -> Option<Tex> {
        self.textures.get(&name.to_ascii_uppercase()).copied()
    }
}

/// Upload an RGBA picture as an overlay texture.
fn upload_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bgl: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    w: u32,
    h: u32,
    rgba: &[u8],
) -> wgpu::BindGroup {
    let t = device.create_texture_with_data(
        queue,
        &wgpu::TextureDescriptor {
            label: Some("hud tex"),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        rgba,
    );
    let view = t.create_view(&Default::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hud tex"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}

pub struct Hud {
    pipeline: wgpu::RenderPipeline,
    textures: Vec<wgpu::BindGroup>,
    /// Slots of textures released by [`Hud::free_texture`], for reuse.
    free: Vec<u32>,
    /// A texture rewritten every frame (movies): the texture and its overlay id.
    stream: Option<(wgpu::Texture, Tex)>,
    bgl: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Texture names, sizes and fonts for building canvases.
    pub ui: UiAssets,
}

/// Collects quads in pixel coordinates, batched by texture.
pub struct Canvas {
    w: f32,
    h: f32,
    verts: Vec<V>,
    /// (texture id, first vertex) for each run of quads sharing a texture.
    batches: Vec<(u32, u32)>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        Canvas {
            w: w as f32,
            h: h as f32,
            verts: Vec::new(),
            batches: Vec::new(),
        }
    }

    pub fn width(&self) -> f32 {
        self.w
    }
    pub fn height(&self) -> f32 {
        self.h
    }

    /// Quad at (x, y, w, h) showing texture rectangle `uv` (u0, v0, u1, v1, 0..1).
    pub fn quad(
        &mut self,
        tex: Tex,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        uv: [f32; 4],
        color: [f32; 4],
    ) {
        self.quad_colors(tex, x, y, w, h, uv, [color; 4]);
    }

    /// As [`Canvas::quad`] with a colour per corner: top-left, top-right,
    /// bottom-right, bottom-left. Colours above 1 brighten the texture.
    #[allow(clippy::too_many_arguments)]
    pub fn quad_colors(
        &mut self,
        tex: Tex,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        uv: [f32; 4],
        colors: [[f32; 4]; 4],
    ) {
        if self.batches.last().map(|b| b.0) != Some(tex.id) {
            self.batches.push((tex.id, self.verts.len() as u32));
        }
        let to = |px: f32, py: f32| [px / self.w * 2.0 - 1.0, 1.0 - py / self.h * 2.0];
        let [u0, v0, u1, v1] = uv;
        let v = |px: f32, py: f32, u: f32, vv: f32, color: [f32; 4]| V {
            pos: to(px, py),
            uv: [u, vv],
            color,
        };
        let a = v(x, y, u0, v0, colors[0]);
        let b = v(x + w, y, u1, v0, colors[1]);
        let c = v(x + w, y + h, u1, v1, colors[2]);
        let d = v(x, y + h, u0, v1, colors[3]);
        self.verts.extend([a, b, c, a, c, d]);
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        self.quad(Tex::WHITE, x, y, w, h, [0.0, 0.0, 1.0, 1.0], color);
    }

    /// A whole texture stretched over (x, y, w, h).
    pub fn sprite(&mut self, tex: Tex, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        // Inset half a texel so linear filtering doesn't wrap around the edges.
        let (hu, hv) = (0.5 / tex.w as f32, 0.5 / tex.h as f32);
        self.quad(tex, x, y, w, h, [hu, hv, 1.0 - hu, 1.0 - hv], color);
    }

    /// Text in a disc font, `px` pixels tall, top-left at (x, y). Returns the width.
    pub fn font_text(
        &mut self,
        f: &Font,
        x: f32,
        y: f32,
        px: f32,
        color: [f32; 4],
        s: &str,
    ) -> f32 {
        self.font_text_padded(f, x, y, px, color, s, 0.0)
    }

    /// As [`Canvas::font_text`], drawing `pad` extra texels either side of each glyph
    /// without changing the spacing (for blurred glow layers).
    pub fn font_text_padded(
        &mut self,
        f: &Font,
        x: f32,
        y: f32,
        px: f32,
        color: [f32; 4],
        s: &str,
        pad: f32,
    ) -> f32 {
        let k = px / f.height;
        let (tw, th) = (f.tex.w as f32, f.tex.h as f32);
        let mut cx = x;
        for ch in s.chars() {
            match f.glyph(ch) {
                Some((gx, gy, gw)) => {
                    let (ux, uw) = (gx - pad, gw + pad * 2.0);
                    let uv = [ux / tw, gy / th, (ux + uw) / tw, (gy + f.height) / th];
                    self.quad(f.tex, cx - pad * k, y, uw * k, px, uv, color);
                    cx += (gw + f.gap) * k;
                }
                None => cx += f.space * k,
            }
        }
        cx - x
    }

    /// Draw 8x8 bitmap text with the top-left at (x, y); `scale` pixels per font pixel.
    pub fn text(&mut self, x: f32, y: f32, scale: f32, color: [f32; 4], s: &str) {
        use font8x8::UnicodeFonts;
        let mut cx = x;
        for ch in s.chars() {
            if let Some(g) = font8x8::BASIC_FONTS.get(ch) {
                for (row, bits) in g.iter().enumerate() {
                    for col in 0..8 {
                        if bits & (1 << col) != 0 {
                            self.rect(
                                cx + col as f32 * scale,
                                y + row as f32 * scale,
                                scale,
                                scale,
                                color,
                            );
                        }
                    }
                }
            }
            cx += 8.0 * scale;
        }
    }

    pub fn text_shadow(&mut self, x: f32, y: f32, scale: f32, color: [f32; 4], s: &str) {
        self.text(x + scale, y + scale, scale, [0.0, 0.0, 0.0, 0.8], s);
        self.text(x, y, scale, color, s);
    }
}

/// The look shared by every menu and the HUD: the game's FONT32 lettering with its
/// dark glow behind it, falling back to the 8x8 font if the disc art is missing.
pub struct Style<'a> {
    pub ui: &'a UiAssets,
}

/// Menu gold, and the dim colour for unselected items.
pub const GOLD: [f32; 4] = [1.0, 0.82, 0.35, 1.0];
pub const DIM: [f32; 4] = [0.62, 0.6, 0.55, 1.0];

impl Style<'_> {
    /// Width of `s` in the menu font at height `px`.
    pub fn width(&self, s: &str, px: f32) -> f32 {
        match &self.ui.font32 {
            Some(f) => f.width(s, px),
            None => s.len() as f32 * px,
        }
    }

    /// Menu text with its glow outline. Returns the width.
    pub fn text(&self, c: &mut Canvas, x: f32, y: f32, px: f32, color: [f32; 4], s: &str) -> f32 {
        match (&self.ui.font32, &self.ui.font32_glow) {
            (Some(f), glow) => {
                if let Some(g) = glow {
                    c.font_text_padded(g, x, y, px, [0.0, 0.0, 0.0, 0.85 * color[3]], s, 3.0);
                }
                c.font_text(f, x, y, px, color, s)
            }
            _ => {
                let scale = (px / 8.0).max(1.0).floor();
                c.text_shadow(x, y + (px - 8.0 * scale) * 0.5, scale, color, s);
                s.len() as f32 * 8.0 * scale
            }
        }
    }

    /// Menu text centred on `cx`.
    pub fn centered(&self, c: &mut Canvas, cx: f32, y: f32, px: f32, color: [f32; 4], s: &str) {
        let w = self.width(s, px);
        self.text(c, cx - w * 0.5, y, px, color, s);
    }

    /// Small HUD text (the 8-pixel hi-res font). Returns the width.
    pub fn small(&self, c: &mut Canvas, x: f32, y: f32, px: f32, color: [f32; 4], s: &str) -> f32 {
        match &self.ui.font8 {
            Some(f) => {
                c.font_text(
                    f,
                    x + px * 0.12,
                    y + px * 0.12,
                    px,
                    [0.0, 0.0, 0.0, 0.8 * color[3]],
                    s,
                );
                c.font_text(f, x, y, px, color, s)
            }
            None => {
                let scale = (px / 8.0).max(1.0).floor();
                c.text_shadow(x, y, scale, color, s);
                s.len() as f32 * 8.0 * scale
            }
        }
    }

    pub fn small_width(&self, s: &str, px: f32) -> f32 {
        match &self.ui.font8 {
            Some(f) => f.width(s, px),
            None => s.len() as f32 * px,
        }
    }

    /// A dark stone panel (S4) in the game's ornate frame (S4_FRAME).
    pub fn panel(&self, c: &mut Canvas, x: f32, y: f32, w: f32, h: f32, tint: [f32; 4]) {
        let (ix, iy, iw, ih) = (x + w * 0.03, y + h * 0.06, w * 0.94, h * 0.88);
        match self.ui.get("S4") {
            Some(t) => c.sprite(t, ix, iy, iw, ih, [0.85, 0.85, 0.85, 0.88]),
            None => c.rect(ix, iy, iw, ih, [0.0, 0.0, 0.0, 0.6]),
        }
        match self.ui.get("S4_FRAME") {
            Some(t) => c.sprite(t, x, y, w, h, tint),
            None => c.rect(x, y, w, 2.0, tint),
        }
    }

    /// The original's 512x384 screen, fitted 4:3 and centred in the window: returns
    /// its top-left corner and window pixels per original pixel.
    pub fn screen(c: &Canvas) -> (f32, f32, f32) {
        let k = (c.width() / 512.0).min(c.height() / 384.0);
        (
            (c.width() - 512.0 * k) * 0.5,
            (c.height() - 384.0 * k) * 0.5,
            k,
        )
    }

    /// TransitionBlitShow (gamemain.c): TRANSITION_SCREEN stretched over the whole
    /// 512x384 screen, black either side of it, at opacity `alpha`.
    pub fn transition(&self, c: &mut Canvas, alpha: f32) {
        let (w, h) = (c.width(), c.height());
        let (x0, y0, k) = Self::screen(c);
        match self.ui.get("TRANSITION_SCREEN") {
            Some(t) => {
                c.rect(0.0, 0.0, x0, h, [0.0, 0.0, 0.0, alpha]);
                c.rect(w - x0, 0.0, x0, h, [0.0, 0.0, 0.0, alpha]);
                c.rect(x0, 0.0, w - 2.0 * x0, y0, [0.0, 0.0, 0.0, alpha]);
                c.rect(x0, h - y0, w - 2.0 * x0, y0, [0.0, 0.0, 0.0, alpha]);
                c.sprite(t, x0, y0, 512.0 * k, 384.0 * k, [1.0, 1.0, 1.0, alpha]);
            }
            None => c.rect(0.0, 0.0, w, h, [0.0, 0.0, 0.0, alpha]),
        }
    }

    /// DrawGlowText (btext.c) at scale 1, (x, y) in original screen pixels: FONT32
    /// in white over its glow layer in glow_color 0x8200EA, whose opacity pulses
    /// between 1/2 and 1 (glow_radius 40, glow_period 5 frames).
    pub fn glow_text(&self, c: &mut Canvas, x: f32, y: f32, s: &str, time: f32) {
        let (x0, y0, k) = Self::screen(c);
        let Some(f) = &self.ui.font32 else {
            self.text(c, x0 + x * k, y0 + y * k, 32.0 * k, [1.0; 4], s);
            return;
        };
        let (px, x, y) = (f.height * k, x0 + x * k, y0 + y * k);
        let frame = (time * 60.0) as i32 % 85;
        let phase = match frame {
            p if p > 80 => 0,
            p if p > 40 => 80 - p,
            p => p,
        };
        let a = (40 + phase * 255 - 1) / 40;
        let glow = (128 - (127 - a / 2) / 2) as f32 / 128.0;
        if let Some(g) = &self.ui.font32_glow {
            let purple = [130.0 / 255.0, 0.0, 234.0 / 255.0, glow];
            c.font_text_padded(g, x, y, px, purple, s, 3.0);
        }
        c.font_text(f, x, y, px, [1.0; 4], s);
    }

    /// Full-screen backdrop for menus: the loading-screen art, darkened.
    pub fn backdrop(&self, c: &mut Canvas, darken: f32) {
        let (w, h) = (c.width(), c.height());
        c.rect(0.0, 0.0, w, h, [0.0, 0.0, 0.0, 1.0]);
        if let Some(t) = self.ui.get("TRANSITION_SCREEN") {
            // Cover the screen, keeping the art square.
            let s = w.max(h);
            let k = 1.0 - darken;
            c.sprite(t, (w - s) * 0.5, (h - s) * 0.5, s, s, [k, k, k, 1.0]);
        }
    }

    /// The GAUNTLET DARK LEGACY logo, `w` pixels wide, centred on `cx`.
    pub fn logo(&self, c: &mut Canvas, cx: f32, y: f32, w: f32, time: f32) -> f32 {
        let Some(t) = self.ui.get("LOGO_B") else {
            self.centered(c, cx, y, w * 0.12, GOLD, "GAUNTLET");
            return w * 0.15;
        };
        let h = w * t.h as f32 / t.w as f32;
        // A slow green pulse, like the burning logo.
        let glow = 0.5 + 0.5 * (time * 2.0).sin();
        c.sprite(t, cx - w * 0.5 - 3.0, y + 3.0, w, h, [0.0, 0.0, 0.0, 0.6]);
        c.sprite(t, cx - w * 0.5, y, w, h, [1.0, 1.0, 1.0, 1.0]);
        c.sprite(t, cx - w * 0.5, y, w, h, [0.2, 1.0, 0.3, 0.12 * glow]);
        h
    }

    /// The green menu cursor, pointing right, with its left edge at x.
    pub fn marker(&self, c: &mut Canvas, x: f32, y: f32, size: f32, time: f32) {
        let bob = (time * 6.0).sin() * size * 0.08;
        match self.ui.get("MENU_MARKER") {
            Some(t) => c.sprite(t, x + bob, y, size, size, [1.0, 1.0, 1.0, 1.0]),
            None => c.rect(
                x + bob,
                y + size * 0.3,
                size * 0.6,
                size * 0.4,
                [0.2, 1.0, 0.3, 1.0],
            ),
        }
    }
}

pub const PLAYER_RGB: [[f32; 3]; 4] = [
    [0.95, 0.25, 0.2],
    [0.35, 0.55, 1.0],
    [1.0, 0.85, 0.25],
    [0.3, 0.9, 0.35],
];

/// A joined player's place on the HUD.
#[derive(Clone, Debug)]
pub struct PanelInfo {
    /// Column 0..3 (the player's slot).
    pub slot: usize,
    /// The name typed on the select screen.
    pub name: String,
    /// Costume colour, as class_id (0 yellow, 1 blue, 2 red, 3 green).
    pub colour: usize,
    /// Character class id (WAR 0 .. HYE 15, SUM 16).
    pub class: usize,
}

/// player_pal: text colours per costume colour.
const PLAYER_PAL: [u32; 4] = [0xFFFF80, 0x87CEEB, 0xFFC0E0, 0x80FF80];
/// player_rgb / player_inactive_rgb: the portrait tint for a joined or empty slot.
const PLAYER_TINT: [u32; 4] = [0x787800, 0x1E1E78, 0x780000, 0x006400];
const INACTIVE_TINT: [u32; 4] = [0x5A5A1E, 0x1E1E69, 0x642828, 0x1E4B1E];
/// potionicon_tab, indexed by potion_type[count - 1] = (count - 1) & 3.
const POTION_ICONS: [&str; 5] = ["RED", "RED", "BLU", "YEL", "GRE"];

/// MBBlitSetColor: blit colours are 0x80 = 1.0, so a tint is about rgb / 255.
fn tint(rgb: u32) -> [f32; 4] {
    let c = |v: u32| (((v >> 1) & 0x7F) + 1) as f32 / 128.0;
    [c(rgb >> 16), c(rgb >> 8), c(rgb), 1.0]
}

/// The status panels along the bottom of the screen (create_player_blits,
/// setup_player_display and write_health_and_items in player.c): one 128-pixel
/// column per slot from y = 304, the empty slots dimmed.
pub fn build(s: &mut Scr, game: &Game, panels: &[PanelInfo]) {
    use crate::ui::{FONT_8HI, FONT_INITIALS};
    const FONT_SCORE: usize = 4;
    s.bars(1.0);
    for slot in 0..4 {
        let x0 = 128.0 * slot as f32;
        let xc = x0 + 64.0;
        let joined = panels.iter().position(|p| p.slot == slot);
        let Some(pi) = joined else {
            // An empty slot: S3 strip, S4 in the inactive tint, S4_FRAME.
            for (name, y, h, color) in [
                ("S3", 304.0, 16.0, [1.0; 4]),
                ("S4", 320.0, 64.0, tint(INACTIVE_TINT[slot])),
                ("S4_FRAME", 320.0, 64.0, [1.0; 4]),
            ] {
                if let Some(t) = s.ui.get(name) {
                    s.blit_tint(t, x0, y, 128.0, h, color);
                }
            }
            continue;
        };
        let info = &panels[pi];
        let Some(p) = game.players.get(pi) else {
            continue;
        };
        let pal = PLAYER_PAL[info.colour % 4];
        let dead = matches!(p.state, PlayerState::Dead { .. });
        if p.state == PlayerState::InTower {
            // Display mode 10: the plain panel, gold and health, "IN TOWER".
            for (name, y, h, color) in [
                ("S3", 304.0, 16.0, [1.0; 4]),
                ("S4", 320.0, 64.0, tint(PLAYER_TINT[info.colour % 4])),
                ("S4_FRAME", 320.0, 64.0, [1.0; 4]),
            ] {
                if let Some(t) = s.ui.get(name) {
                    s.blit_tint(t, x0, y, 128.0, h, color);
                }
            }
            let others = game.players.iter().any(|q| q.state.active());
            if p.tower_choice && others {
                for (icon, label, y) in [
                    ("BUTTON_X", "Wait In Tower", 332.0),
                    ("BUTTON_TRI", "Quit Game", 352.0),
                ] {
                    if let Some(t) = s.ui.get(icon) {
                        s.blit(t, x0 + 6.0, y, 14.0, 14.0, 0);
                    }
                    s.text(1.2, x0 + 20.0, y + 4.0, FONT_8HI, 0xFFFFFF, 0, label);
                }
                continue;
            }
            if !p.quit {
                s.text(1.2, -xc, 340.0, FONT_8HI, pal, 0, "IN TOWER");
            }
            continue;
        }
        // Display mode 1: the rune-stone strip and the class portrait.
        let portrait = format!("S4_{}", crate::select::CLASSES[info.class.min(16)]);
        for (name, y, h, color) in [
            ("BK_RUNE_STONE_02", 304.0, 16.0, [1.0; 4]),
            (
                portrait.as_str(),
                320.0,
                64.0,
                tint(PLAYER_TINT[info.colour % 4]),
            ),
            ("S4_FRAME", 320.0, 64.0, [1.0; 4]),
        ] {
            if let Some(t) = s.ui.get(name) {
                s.blit_tint(t, x0, y, 128.0, h, color);
            }
        }
        if !dead {
            power_meter(s, x0, p.power);
        }
        // The party's runestones along the strip (rune_blit).
        for j in 0..12u32 {
            if game.shards & (1 << j) != 0 {
                let c = ["BLU", "RED", "YEL", "GRE"][(j / 3) as usize];
                if let Some(t) = s.ui.get(&format!("SM_RUNE_{c}_{:02}", j % 3 + 1)) {
                    let x = x0 + (8 * j + j / 3 + 15) as f32;
                    s.blit(t, x, 306.0, 8.0, 8.0, 0);
                }
            }
        }
        for (name, x) in [("COIN", x0 + 6.0), ("HEART", x0 + 61.0)] {
            if let Some(t) = s.ui.get(name) {
                s.blit(t, x, 357.0, 20.0, 20.0, 0);
            }
        }
        let gold = p.gold.min(99999).to_string();
        let w = s.text_width(FONT_SCORE, 1.0, &gold);
        s.text(1.0, x0 + 60.0 - w, 359.0, FONT_SCORE, pal, 0, &gold);
        let health = (p.health.max(0.0) as u32).min(9999).to_string();
        let w = s.text_width(FONT_SCORE, 1.0, &health);
        s.text(1.0, x0 + 116.0 - w, 359.0, FONT_SCORE, pal, 0, &health);
        s.text(0.667, -xc, 339.0, FONT_INITIALS, pal, 0, &info.name);
        // The mini-inventory, fully open: the power-up's name above the panel,
        // glowing while it is on.
        if p.inv.state == 1 && p.inv.sel >= 0 {
            let sl = &p.powerups.slots[p.inv.sel as usize];
            let label = crate::powerups::label(sl);
            let y = 310.0 + 128.0 - p.inv.slide as f32;
            if sl.state == crate::powerups::ON {
                s.glow_text(0.45, x0 + 24.0, y, label);
            } else {
                s.text(0.45, x0 + 24.0, y, crate::ui::FONT_32, 0xFFFFFF, 0, label);
            }
        }
        s.text(
            1.0,
            -xc,
            326.0,
            FONT_8HI,
            0xFFFFFF,
            0,
            &format!("LV {}", p.level),
        );
        if p.keys > 0 {
            if let Some(t) = s.ui.get("KEY_ICON") {
                s.blit(t, x0 + 8.0, 323.0, t.w as f32, t.h as f32, 0);
            }
            s.text(
                0.8,
                x0 + 26.0,
                327.0,
                FONT_SCORE,
                pal,
                0,
                &p.keys.to_string(),
            );
        }
        // show_crystals: the colour just picked up, as found / needed.
        if let Some((c, _)) = game.crystal_shown {
            let need = crate::game::CRYSTALS_NEEDED[c];
            let n = game.crystals[c];
            let n = if n < 0 { need } else { n };
            let icon = format!("SM_CRYSTAL_{}", &crate::game::CRYSTAL_COLOURS[c][..3]);
            if let Some(t) = s.ui.get(&icon) {
                s.blit(t, x0 + 28.0, 288.0, 16.0, 16.0, 0);
            }
            s.text(
                1.5,
                x0 + 48.0,
                292.0,
                FONT_8HI,
                0xFFFFFF,
                0,
                &format!("{n}/{need}"),
            );
        }
        if p.potions > 0 {
            let icon = POTION_ICONS[((p.potions - 1) & 3) as usize];
            if let Some(t) = s.ui.get(&format!("POTION_ICON_{icon}")) {
                s.blit(t, x0 + 102.0, 323.0, t.w as f32, t.h as f32, 0);
            }
            s.text(
                0.8,
                x0 + 92.0,
                327.0,
                FONT_SCORE,
                pal,
                0,
                &p.potions.to_string(),
            );
        }
    }
    // A secret level's hourglass (gamemain.c fn_800552A4): the top sand drains
    // and the bottom fills as the time runs out.
    if let Some((total, left)) = game.secret_timer {
        let p = ((total - left.max(0.0)) / total).clamp(0.0, 1.0);
        if let Some(t) = s.ui.get("TIMER") {
            s.blit(t, 1.0, 1.0, t.w as f32, t.h as f32, 0);
        }
        if let Some(t) = s.ui.get("TIMER_SAND") {
            let w = t.w as f32;
            let th = t.h as f32;
            let drop = (39.0 * p).round();
            let v0 = (41.0 * p + 23.0) / 128.0;
            s.blit_uv(
                t,
                1.0,
                drop + 24.0,
                w,
                41.0 - drop,
                [0.0, v0, 1.0, 64.0 / 128.0],
                0,
            );
            let rise = (38.0 * p).round();
            let v0 = (105.0 - 38.0 * p) / 128.0;
            let _ = th;
            s.blit_uv(t, 1.0, 106.0 - rise, w, rise + 23.0, [0.0, v0, 1.0, 1.0], 0);
        }
    }
    // msgDraw: over the player it's about (62 pixels above them on screen), or
    // over slot 0 at y 250 for party messages.
    if let Some(m) = &game.msgs.shown {
        let lines: Vec<&str> = m.lines.iter().map(String::as_str).collect();
        let at = m
            .player
            .and_then(|pi| game.players.get(pi))
            .and_then(|p| crate::scene::project(game, p.pos + glam::Vec3::Y * 2.5));
        let (cx, cy, rgb) = match (m.player, at) {
            (Some(pi), Some((x, y))) => {
                let slot = panels.get(pi).map_or(pi, |p| p.slot);
                let colour = panels.get(pi).map_or(slot, |p| p.colour);
                let _ = slot;
                (x, y - 62.0, crate::messages::PLAYER_TEXT[colour % 4])
            }
            _ => (64.0, 250.0, crate::messages::NEUTRAL_TEXT),
        };
        message_box(s, cx, cy, &lines, rgb);
    }
    // A trigger camera letterboxes the view (bosscam.c), and Sumner's words
    // are written over the lower bar.
    if let Some(sm) = &game.sumner {
        if sm.cam.is_some() {
            s.rect(0.0, 0.0, 512.0, 48.0, [0.0, 0.0, 0.0, 1.0]);
            s.rect(0.0, 304.0, 512.0, 80.0, [0.0, 0.0, 0.0, 1.0]);
        }
        if let Some(t) = &sm.text {
            crate::menu::draw_caption(s, t, 312.0);
        }
    }
}

/// draw_power_meter at power level `level` (0..100), not flashing.
fn power_meter(s: &mut Scr, x0: f32, level: f32) {
    let f = 0.01 * level;
    let (zone1, frac) = if f < 0.4 {
        (true, f / 0.4)
    } else if f < 0.99 {
        (false, (f - 0.4) / 0.6)
    } else {
        (false, 1.0)
    };
    let a = (127.0 * frac + 128.0) / 255.0;
    let (fill, bar) = if zone1 {
        ([a, a, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0])
    } else {
        ([a, 0.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0])
    };
    let Some(t) = s.ui.get("TRBO_FULL_NEW") else {
        return;
    };
    s.blit_tint(t, x0, 304.0, 128.0, 32.0, bar);
    let w = (((128.0 * frac) as i32) >> 1).max(1) as f32;
    s.blit_tint(t, x0 + 64.0 - w, 304.0, 2.0 * w, 32.0, fill);
    if let Some(g) = s.ui.get("TRBO_GLINT") {
        s.blit(g, x0, 304.0, 128.0, 16.0, 0);
    }
}

/// msgPost / msgDraw: dark text centred in a 75%-opaque SCROLL_A box, the block
/// centred on (cx, cy) and kept on screen.
pub fn message_box(s: &mut Scr, cx: f32, cy: f32, lines: &[&str], rgb: u32) {
    use crate::ui::FONT_8X8;
    let fh = 9.0;
    let gap = 8.0;
    let h = lines.len() as f32 * (fh + gap) + 16.0;
    let w = lines
        .iter()
        .map(|l| s.text_width(FONT_8X8, 1.0, l))
        .fold(0.0, f32::max)
        + 64.0;
    let top = (cy - h / 2.0).clamp(2.0, 304.0 - h);
    let left = (cx - w / 2.0).clamp(0.0, 511.0 - w);
    let (cx, cy) = (left + w / 2.0, top + h / 2.0);
    if let Some(t) = s.ui.get("SCROLL_A") {
        s.blit(t, left, top, w, h, 0x40);
    }
    let n = lines.len() as f32;
    let mut y = cy - (fh + 11.0 * (n - 1.0)) / 2.0;
    for l in lines {
        s.text(1.0, -cx, y, FONT_8X8, rgb, 0, l);
        y += 11.0;
    }
}

impl Hud {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        mut ui: UiAssets,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud tex"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let upload =
            |w: u32, h: u32, rgba: &[u8]| upload_texture(device, queue, &bgl, &sampler, w, h, rgba);
        let mut textures = vec![upload(1, 1, &[255; 4])];
        for img in ui.images.drain(..) {
            textures.push(upload(img.width, img.height, &img.rgba));
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<V>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x4],
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });
        Hud {
            pipeline,
            textures,
            free: Vec::new(),
            stream: None,
            bgl,
            sampler,
            ui,
        }
    }

    /// Upload a picture at runtime and return its overlay handle. Slots released
    /// with [`Hud::free_texture`] are reused.
    pub fn add_texture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, img: &Image) -> Tex {
        let bg = upload_texture(
            device,
            queue,
            &self.bgl,
            &self.sampler,
            img.width,
            img.height,
            &img.rgba,
        );
        let id = match self.free.pop() {
            Some(id) => {
                self.textures[id as usize] = bg;
                id
            }
            None => {
                self.textures.push(bg);
                self.textures.len() as u32 - 1
            }
        };
        Tex {
            id,
            w: img.width,
            h: img.height,
        }
    }

    /// Release a texture from [`Hud::add_texture`]; drawing it afterwards draws white.
    pub fn free_texture(&mut self, t: Tex) {
        if t.id == 0 || t.id as usize >= self.textures.len() || self.free.contains(&t.id) {
            return;
        }
        if self.stream.as_ref().is_some_and(|(_, s)| s.id == t.id) {
            return;
        }
        // Dropping the bind group releases the GPU texture.
        self.textures[t.id as usize] = self.textures[0].clone();
        self.free.push(t.id);
    }

    /// Load and upload a model directory's named textures (e.g. `MAPS/levelA6`).
    pub fn load_bank(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        fs: &GameFs,
        dir: &str,
    ) -> anyhow::Result<TexBank> {
        let mut bank = TexBank::default();
        for (name, img) in crate::ui::load_bank_images(fs, dir)? {
            let t = self.add_texture(device, queue, &img);
            bank.textures.insert(name, t);
        }
        Ok(bank)
    }

    /// Release every texture of a bank from [`Hud::load_bank`].
    pub fn free_bank(&mut self, bank: TexBank) {
        for t in bank.textures.into_values() {
            self.free_texture(t);
        }
    }

    /// Upload a w x h RGBA picture into the streaming texture (made, or remade,
    /// when its size changes) and return it for drawing.
    pub fn stream_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        rgba: &[u8],
    ) -> Tex {
        let size = wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        };
        if self.stream.as_ref().map(|(_, t)| (t.w, t.h)) != Some((w, h)) {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hud stream"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = t.create_view(&Default::default());
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hud stream"),
                layout: &self.bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let id = match &self.stream {
                Some((_, old)) => {
                    self.textures[old.id as usize] = bg;
                    old.id
                }
                None => {
                    self.textures.push(bg);
                    self.textures.len() as u32 - 1
                }
            };
            self.stream = Some((t, Tex { id, w, h }));
        }
        let (t, tex) = self.stream.as_ref().unwrap();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: t,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            size,
        );
        *tex
    }

    pub fn draw(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        canvas: &Canvas,
    ) {
        if canvas.verts.is_empty() {
            return;
        }
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("hud verts"),
            contents: bytemuck::cast_slice(&canvas.verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mut enc = device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, vbuf.slice(..));
            let end = canvas.verts.len() as u32;
            for (k, &(tex, start)) in canvas.batches.iter().enumerate() {
                let stop = canvas.batches.get(k + 1).map(|b| b.1).unwrap_or(end);
                let bg = self.textures.get(tex as usize).unwrap_or(&self.textures[0]);
                pass.set_bind_group(0, bg, &[]);
                pass.draw(start..stop, 0..1);
            }
        }
        queue.submit([enc.finish()]);
    }
}
