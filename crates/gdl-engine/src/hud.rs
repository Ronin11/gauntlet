//! 2D overlay: coloured rectangles and 8x8 bitmap text, drawn after the 3D scene.

use crate::game::{Game, PlayerState};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct V {
    pos: [f32; 2],
    color: [f32; 4],
}

const SHADER: &str = r#"
struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) color: vec4<f32> };
@vertex fn vs(@location(0) p: vec2<f32>, @location(1) c: vec4<f32>) -> VsOut {
    var o: VsOut; o.pos = vec4<f32>(p, 0.0, 1.0); o.color = c; return o;
}
@fragment fn fs(v: VsOut) -> @location(0) vec4<f32> { return v.color; }
"#;

pub struct Hud {
    pipeline: wgpu::RenderPipeline,
}

/// Collects quads in pixel coordinates.
pub struct Canvas {
    w: f32,
    h: f32,
    verts: Vec<V>,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Self {
        Canvas {
            w: w as f32,
            h: h as f32,
            verts: Vec::new(),
        }
    }

    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        let to = |px: f32, py: f32| [px / self.w * 2.0 - 1.0, 1.0 - py / self.h * 2.0];
        let (a, b, c, d) = (to(x, y), to(x + w, y), to(x + w, y + h), to(x, y + h));
        for p in [a, b, c, a, c, d] {
            self.verts.push(V { pos: p, color });
        }
    }

    /// Draw text with the top-left at (x, y); `scale` pixels per font pixel.
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

    pub fn width(&self) -> f32 {
        self.w
    }
    pub fn height(&self) -> f32 {
        self.h
    }
}

pub const PLAYER_RGB: [[f32; 3]; 4] = [
    [0.9, 0.2, 0.15],
    [0.25, 0.45, 1.0],
    [1.0, 0.85, 0.2],
    [0.2, 0.85, 0.3],
];

const CLASS_NAMES: [(&str, &str); 4] = [
    ("WAR", "WARRIOR"),
    ("VAL", "VALKYRIE"),
    ("WIZ", "WIZARD"),
    ("ARC", "ARCHER"),
];

/// Build the in-game HUD for the current game state.
pub fn build(c: &mut Canvas, game: &Game, classes: &[String]) {
    let scale = (c.height() / 360.0).min(c.width() / 560.0).clamp(1.0, 4.0).floor();
    let panel_w = 120.0 * scale;
    let panel_h = 34.0 * scale;
    let n = game.players.len();
    for (i, p) in game.players.iter().enumerate() {
        let rgb = PLAYER_RGB[i % 4];
        // Corners: 0 top-left, 1 top-right, 2 bottom-left, 3 bottom-right.
        let x = if i % 2 == 0 {
            6.0 * scale
        } else {
            c.width() - panel_w - 6.0 * scale
        };
        let y = if i < 2 {
            6.0 * scale
        } else {
            c.height() - panel_h - 6.0 * scale
        };
        c.rect(x, y, panel_w, panel_h, [0.0, 0.0, 0.0, 0.55]);
        c.rect(x, y, panel_w, 2.0 * scale, [rgb[0], rgb[1], rgb[2], 1.0]);
        let class = classes.get(i).map(String::as_str).unwrap_or("");
        let name = CLASS_NAMES
            .iter()
            .find(|(k, _)| *k == class)
            .map(|(_, n)| *n)
            .unwrap_or(class);
        c.text_shadow(
            x + 4.0 * scale,
            y + 5.0 * scale,
            scale,
            [rgb[0], rgb[1], rgb[2], 1.0],
            name,
        );
        let health = p.health.max(0.0) as u32;
        let dead = matches!(p.state, PlayerState::Dead { .. });
        let hcol = if dead || health < 100 {
            [1.0, 0.3, 0.3, 1.0]
        } else {
            [1.0, 1.0, 1.0, 1.0]
        };
        let htxt = if dead {
            "DEAD".to_string()
        } else {
            format!("{health}")
        };
        c.text_shadow(
            x + panel_w - (htxt.len() as f32 * 8.0 + 4.0) * scale,
            y + 5.0 * scale,
            scale,
            hcol,
            &htxt,
        );
        // Health bar.
        let frac = (p.health / 999.0).clamp(0.0, 1.0);
        c.rect(
            x + 4.0 * scale,
            y + 15.0 * scale,
            panel_w - 8.0 * scale,
            3.0 * scale,
            [0.2, 0.0, 0.0, 0.9],
        );
        c.rect(
            x + 4.0 * scale,
            y + 15.0 * scale,
            (panel_w - 8.0 * scale) * frac,
            3.0 * scale,
            [0.85, 0.1, 0.1, 1.0],
        );
        c.text_shadow(
            x + 4.0 * scale,
            y + 22.0 * scale,
            scale,
            [1.0, 0.85, 0.3, 1.0],
            &format!("${}", p.score),
        );
        let inv = format!("K{} P{}", p.keys, p.potions);
        c.text_shadow(
            x + panel_w - (inv.len() as f32 * 8.0 + 4.0) * scale,
            y + 22.0 * scale,
            scale,
            [0.8, 0.9, 1.0, 1.0],
            &inv,
        );
    }
    let _ = n;
    // Centre messages.
    for (k, (msg, t)) in game.messages.iter().enumerate() {
        let a = t.min(1.0);
        let s = scale * 2.0;
        let w = msg.len() as f32 * 8.0 * s;
        c.text_shadow(
            (c.width() - w) * 0.5,
            c.height() * 0.3 + k as f32 * 12.0 * s,
            s,
            [1.0, 1.0, 0.6, a],
            msg,
        );
    }
}

impl Hud {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud"),
            bind_group_layouts: &[],
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
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4],
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
        Hud { pipeline }
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
            pass.draw(0..canvas.verts.len() as u32, 0..1);
        }
        queue.submit([enc.finish()]);
    }
}
