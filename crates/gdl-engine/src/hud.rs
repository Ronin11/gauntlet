//! 2D overlay drawn after the 3D scene: textured quads (the disc's menu art and
//! fonts, see [`crate::ui`]), solid rectangles, and an 8x8 fallback font.

use crate::game::{Game, PlayerState, PLAYER_COLORS};
use crate::ui::{Font, Tex, UiAssets};
use bytemuck::{Pod, Zeroable};
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

pub struct Hud {
    pipeline: wgpu::RenderPipeline,
    textures: Vec<wgpu::BindGroup>,
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
        if self.batches.last().map(|b| b.0) != Some(tex.id) {
            self.batches.push((tex.id, self.verts.len() as u32));
        }
        let to = |px: f32, py: f32| [px / self.w * 2.0 - 1.0, 1.0 - py / self.h * 2.0];
        let [u0, v0, u1, v1] = uv;
        let a = V {
            pos: to(x, y),
            uv: [u0, v0],
            color,
        };
        let b = V {
            pos: to(x + w, y),
            uv: [u1, v0],
            color,
        };
        let c = V {
            pos: to(x + w, y + h),
            uv: [u1, v1],
            color,
        };
        let d = V {
            pos: to(x, y + h),
            uv: [u0, v1],
            color,
        };
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
                    cx += (gw + f.gap()) * k;
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

/// Build the in-game HUD for the current game state: a framed stone panel in each
/// player's corner with name, health, gold, keys and potions.
pub fn build(c: &mut Canvas, ui: &UiAssets, game: &Game, names: &[String]) {
    let st = Style { ui };
    let u = (c.height() / 720.0).min(c.width() / 1120.0).max(0.5);
    let panel_w = 256.0 * u;
    let panel_h = 128.0 * u;
    let m = 8.0 * u;
    for (i, p) in game.players.iter().enumerate() {
        let rgb = PLAYER_RGB[i % 4];
        let col = [rgb[0], rgb[1], rgb[2], 1.0];
        // Corners: 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
        let x = if i % 2 == 0 {
            m
        } else {
            c.width() - panel_w - m
        };
        let y = if i < 2 { m } else { c.height() - panel_h - m };
        let frame = [
            0.75 + rgb[0] * 0.25,
            0.75 + rgb[1] * 0.25,
            0.75 + rgb[2] * 0.25,
            1.0,
        ];
        st.panel(c, x, y, panel_w, panel_h, frame);
        let (l, r) = (x + 22.0 * u, x + panel_w - 22.0 * u);

        let name = names.get(i).map(String::as_str).unwrap_or("");
        st.text(c, l, y + 14.0 * u, 28.0 * u, col, name);

        let dead = matches!(p.state, PlayerState::Dead { .. });
        let health = p.health.max(0.0) as u32;
        let htxt = if dead {
            "Dead".to_string()
        } else {
            health.to_string()
        };
        let hcol = if dead || health < 200 {
            let pulse = 0.6 + 0.4 * (game.time * 8.0).sin().abs();
            [1.0, 0.25 * pulse, 0.2 * pulse, 1.0]
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        let hw = st.width(&htxt, 30.0 * u);
        st.text(c, r - hw, y + 44.0 * u, 30.0 * u, hcol, &htxt);

        // Health bar: the metal tube with the pink fill over it.
        let (bx, by, bw, bh) = (l - 4.0 * u, y + 48.0 * u, r - l - 80.0 * u, 22.0 * u);
        let frac = (p.health / 999.0).clamp(0.0, 1.0);
        match ui.get("EMPTY_BAR") {
            Some(t) => c.sprite(t, bx, by, bw, bh, [1.0; 4]),
            None => c.rect(bx, by, bw, bh, [0.2, 0.2, 0.2, 1.0]),
        }
        let (fx, fy, fw, fh) = (bx + bw * 0.03, by + bh * 0.3, bw * 0.94, bh * 0.4);
        c.rect(fx, fy, fw, fh, [0.1, 0.0, 0.02, 0.85]);
        match ui.get("PINK_BAR") {
            Some(t) => c.sprite(t, fx, fy, fw * frac, fh, [1.0, 0.45, 0.45, 1.0]),
            None => c.rect(fx, fy, fw * frac, fh, [0.9, 0.15, 0.1, 1.0]),
        }

        // Gold on the left; potions and keys on the right.
        let ry = y + 80.0 * u;
        let icon = 26.0 * u;
        let mut cx = l;
        if let Some(t) = ui.get("COIN") {
            c.sprite(t, cx, ry, icon, icon, [1.0; 4]);
            cx += icon + 6.0 * u;
        }
        st.text(c, cx, ry, 26.0 * u, GOLD, &p.score.to_string());
        let key = ui.get("KEY_ICON");
        let potion = ui.get(&format!("POTION_ICON_{}", PLAYER_COLORS[i % 4]));
        let mut rx = r;
        for (tex, n) in [(potion, p.potions), (key, p.keys)] {
            let Some(t) = tex else { continue };
            if n == 0 {
                continue;
            }
            if n > 1 {
                let label = n.to_string();
                rx -= st.width(&label, 24.0 * u);
                st.text(c, rx, ry + 1.0 * u, 24.0 * u, [1.0; 4], &label);
            }
            rx -= icon + 2.0 * u;
            c.sprite(t, rx, ry, icon, icon, [1.0; 4]);
            rx -= 10.0 * u;
        }
    }
    // Centre messages.
    for (k, (msg, t)) in game.messages.iter().enumerate() {
        let a = t.min(1.0);
        st.centered(
            c,
            c.width() * 0.5,
            c.height() * 0.28 + k as f32 * 46.0 * u,
            38.0 * u,
            [GOLD[0], GOLD[1], GOLD[2], a],
            msg,
        );
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
        let upload = |w: u32, h: u32, rgba: &[u8]| {
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
                layout: &bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
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
            ui,
        }
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
