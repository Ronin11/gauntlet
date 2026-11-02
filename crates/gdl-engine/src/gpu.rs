//! wgpu renderer: one pipeline pair (opaque with alpha test, alpha blended) shared by
//! world geometry and actors.

use crate::mesh::{MeshData, TexAnimKind, Vertex};
use crate::psys::Blend;
use crate::texanim::{self, GlobalState};
use anyhow::{Context, Result};
use bytemuck::{Pod, Zeroable};
use gdl_formats::anim::TexMod;
use gdl_formats::Image;
use glam::{Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    pub light: [f32; 4],
    pub fog: [f32; 4],
    pub camera_pos: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ModelUniform {
    transform: [[f32; 4]; 4],
    tint: [f32; 4],
    params: [f32; 4],
    /// Texture coordinate transform: u * x + y, v * z + w (the texture's scroll
    /// record: uScale, uAdd, vScale, vAdd).
    uv: [f32; 4],
}

/// Identity texture coordinate transform.
const UV_IDENTITY: [f32; 4] = [1.0, 0.0, 1.0, 0.0];

pub struct Gpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub color_format: wgpu::TextureFormat,
    opaque: wgpu::RenderPipeline,
    blend: wgpu::RenderPipeline,
    additive: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bg: wgpu::BindGroup,
    material_layout: wgpu::BindGroupLayout,
    model_layout: wgpu::BindGroupLayout,
    sampler_repeat: wgpu::Sampler,
    white: wgpu::TextureView,
    /// Sprite pipelines by [blend][z compare][z write] (pbBlitSetDrawRegs).
    sprite: Vec<wgpu::RenderPipeline>,
}

/// One corner of a sprite or streak quad (world space, already mirrored).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug, Default)]
pub struct SpriteVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

/// A run of quads drawn with one texture and draw state.
#[derive(Clone, Copy, Debug)]
pub struct SpriteDraw {
    /// Index into the layer's textures (None: white).
    pub texture: Option<usize>,
    pub blend: Blend,
    pub ztest: bool,
    pub zwrite: bool,
    pub first: u32,
    pub count: u32,
}

/// The particle and streak quads of a frame, drawn after the meshes (the
/// psys queue MBDrawPsysObjects flushes), with their textures.
pub struct SpriteLayer {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    /// Quads the buffers hold.
    capacity: usize,
    materials: Vec<wgpu::BindGroup>,
    white: wgpu::BindGroup,
    draws: Vec<SpriteDraw>,
}

impl SpriteLayer {
    /// Textures uploaded so far.
    pub fn texture_count(&self) -> usize {
        self.materials.len()
    }
}

fn blend_index(b: Blend) -> usize {
    match b {
        Blend::Alpha => 0,
        Blend::Add => 1,
        Blend::AddOne => 2,
        Blend::Darken => 3,
        Blend::Replace => 4,
    }
}

struct GpuBatch {
    additive: bool,
    part: u16,
    start: u32,
    count: u32,
    material: usize,
    blend: bool,
    params: [f32; 4],
    uv: [f32; 4],
    /// Flipbook animating this batch's texture: (index into `GpuMesh::anims`,
    /// material of each frame).
    flip: Option<(usize, Vec<usize>)>,
    /// Scroll animations of this batch's texture (U, V), by index into `GpuMesh::anims`.
    scroll: [Option<usize>; 2],
    /// Sequence-texmod flipbooks of this batch's texture: (index into
    /// `MeshData::seq_anims`, material of each frame).
    seq_flips: Vec<(usize, Vec<usize>)>,
}

impl GpuBatch {
    /// Material, texture transform and opacity of this batch under `fx`.
    fn resolve(&self, fx: Option<&PartFx>) -> (usize, [f32; 4], f32) {
        let Some(fx) = fx else {
            return (self.material, self.uv, 1.0);
        };
        let material = fx
            .alt
            .and_then(|(anim, frame)| {
                let (_, mats) = self.seq_flips.iter().find(|(a, _)| *a == anim)?;
                mats.get(frame).or(mats.last()).copied()
            })
            .unwrap_or(self.material);
        // setTexShift: the texture's own scroll record wins over the node's
        // UV scale/add.
        let uv = match (self.scroll, fx.uv) {
            ([None, None], Some(uv)) => uv,
            _ => self.uv,
        };
        (material, uv, fx.alpha.map_or(1.0, |a| a as f32 / 255.0))
    }
}

pub use crate::mesh::PartFx;

/// A mesh uploaded to the GPU, with its textures.
pub struct GpuMesh {
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    batches: Vec<GpuBatch>,
    materials: Vec<wgpu::BindGroup>,
    /// The mesh's global texmods (see [`GpuMesh::animate_textures`]).
    anims: Vec<TexMod>,
    pub bounds_min: Vec3,
    pub bounds_max: Vec3,
}

impl GpuMesh {
    /// Show the mesh's animated textures as they are after `inf_frame` game
    /// frames (`DoTexMods` runs once per frame): flipbook frames swap the
    /// batch's texture, scrolls offset its UVs.
    pub fn animate_textures(&mut self, inf_frame: u32) {
        if self.anims.is_empty() {
            return;
        }
        let states: Vec<GlobalState> = self
            .anims
            .iter()
            .map(|t| texanim::global_state(t, inf_frame))
            .collect();
        for b in &mut self.batches {
            if let Some((a, frames)) = &b.flip {
                if let GlobalState::Frame(f) = states[*a] {
                    if let Some(&m) = frames.get(f) {
                        b.material = m;
                    }
                }
            }
            for (axis, a) in b.scroll.iter().enumerate() {
                if let Some(a) = a {
                    match states[*a] {
                        GlobalState::ScrollU(o) | GlobalState::ScrollV(o) => b.uv[axis * 2 + 1] = o,
                        _ => {}
                    }
                }
            }
        }
    }
}

/// One placement of a mesh in the world. Owns uniform buffers for its batches.
pub struct Instance {
    bufs: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
    pub transform: Mat4,
    /// Per-part transforms for rigid-part models (applied before `transform`).
    pub parts: Vec<Mat4>,
    /// Parts not drawn this frame (mesh-swap animation frames).
    pub hidden: Vec<bool>,
    pub tint: Vec4,
    pub visible: bool,
    /// Per-part texmod effects (empty: none).
    pub part_fx: Vec<PartFx>,
    /// `MBTreeSetAltTex` over the whole model: one texture for every batch.
    pub alt_tex: Option<std::sync::Arc<wgpu::BindGroup>>,
}

impl Gpu {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        color_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gdl shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let uniform_entry = |binding, vis| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let model_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("model"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let samp_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("material"),
            entries: &[tex_entry(0), samp_entry(1), tex_entry(2), samp_entry(3)],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("gdl layout"),
            bind_group_layouts: &[&globals_layout, &material_layout, &model_layout],
            push_constant_ranges: &[],
        });

        let attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x2, 4 => Float32x2];
        let additive_state = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let make = |fs: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(fs),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attrs,
                    }],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: color_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: blend.is_none(),
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview: None,
                cache: None,
            })
        };
        let opaque = make("fs_opaque", None);
        let blend = make("fs_blend", Some(wgpu::BlendState::ALPHA_BLENDING));
        let additive = make("fs_blend", Some(additive_state));

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let sampler_repeat = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            // Floors and walls are seen at a grazing angle from the game's
            // high camera; trilinear alone smears them into mud.
            anisotropy_clamp: 16,
            ..Default::default()
        });

        // Sprites: GX's blend factors, alpha compare > 2, z per the node.
        let sprite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sprite layout"),
            bind_group_layouts: &[&globals_layout, &material_layout],
            push_constant_ranges: &[],
        });
        let sprite_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4];
        let factor = |src, dst| wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: src,
                dst_factor: dst,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        use wgpu::BlendFactor as F;
        let blends = [
            factor(F::SrcAlpha, F::OneMinusSrcAlpha),
            factor(F::SrcAlpha, F::One),
            factor(F::One, F::One),
            factor(F::Zero, F::SrcAlpha),
            factor(F::One, F::Zero),
        ];
        let mut sprite = Vec::new();
        for blend in blends {
            for ztest in [false, true] {
                for zwrite in [false, true] {
                    sprite.push(
                        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                            label: Some("sprite"),
                            layout: Some(&sprite_layout),
                            vertex: wgpu::VertexState {
                                module: &shader,
                                entry_point: Some("vs_sprite"),
                                buffers: &[wgpu::VertexBufferLayout {
                                    array_stride: std::mem::size_of::<SpriteVertex>() as u64,
                                    step_mode: wgpu::VertexStepMode::Vertex,
                                    attributes: &sprite_attrs,
                                }],
                                compilation_options: Default::default(),
                            },
                            fragment: Some(wgpu::FragmentState {
                                module: &shader,
                                entry_point: Some("fs_sprite"),
                                targets: &[Some(wgpu::ColorTargetState {
                                    format: color_format,
                                    blend: Some(blend),
                                    write_mask: wgpu::ColorWrites::ALL,
                                })],
                                compilation_options: Default::default(),
                            }),
                            primitive: wgpu::PrimitiveState {
                                topology: wgpu::PrimitiveTopology::TriangleList,
                                cull_mode: None,
                                ..Default::default()
                            },
                            depth_stencil: Some(wgpu::DepthStencilState {
                                format: DEPTH_FORMAT,
                                depth_write_enabled: zwrite,
                                depth_compare: if ztest {
                                    wgpu::CompareFunction::LessEqual
                                } else {
                                    wgpu::CompareFunction::Always
                                },
                                stencil: Default::default(),
                                bias: Default::default(),
                            }),
                            multisample: Default::default(),
                            multiview: None,
                            cache: None,
                        }),
                    );
                }
            }
        }

        let white = upload_texture(&device, &queue, &Image::solid(4, 4, [255, 255, 255, 255]));
        Gpu {
            device,
            queue,
            color_format,
            opaque,
            blend,
            additive,
            globals_buf,
            globals_bg,
            material_layout,
            model_layout,
            sampler_repeat,
            white,
            sprite,
        }
    }

    fn material(&self, view: &wgpu::TextureView) -> wgpu::BindGroup {
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_repeat),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.white),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_repeat),
                },
            ],
        })
    }

    fn sprite_buffers(&self, quads: usize) -> (wgpu::Buffer, wgpu::Buffer) {
        let vbuf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sprite vbuf"),
            size: (quads * 4 * std::mem::size_of::<SpriteVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let idx: Vec<u32> = (0..quads as u32)
            .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|k| q * 4 + k))
            .collect();
        let ibuf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("sprite ibuf"),
                contents: bytemuck::cast_slice(&idx),
                usage: wgpu::BufferUsages::INDEX,
            });
        (vbuf, ibuf)
    }

    /// An empty sprite layer.
    pub fn new_sprite_layer(&self) -> SpriteLayer {
        let capacity = 1024;
        let (vbuf, ibuf) = self.sprite_buffers(capacity);
        SpriteLayer {
            vbuf,
            ibuf,
            capacity,
            materials: Vec::new(),
            white: self.material(&self.white),
            draws: Vec::new(),
        }
    }

    /// Upload a sprite texture; its index is the layer's next.
    pub fn add_sprite_texture(&self, layer: &mut SpriteLayer, img: &Image) {
        let view = self.upload_texture(img);
        layer.materials.push(self.material(&view));
    }

    /// This frame's quads (4 corners each, fan order) and their draws.
    pub fn set_sprites(
        &self,
        layer: &mut SpriteLayer,
        quads: &[[SpriteVertex; 4]],
        draws: Vec<SpriteDraw>,
    ) {
        if quads.len() > layer.capacity {
            let cap = quads.len().next_power_of_two();
            (layer.vbuf, layer.ibuf) = self.sprite_buffers(cap);
            layer.capacity = cap;
        }
        if !quads.is_empty() {
            self.queue
                .write_buffer(&layer.vbuf, 0, bytemuck::cast_slice(quads));
        }
        layer.draws = draws;
    }

    pub fn upload_texture(&self, img: &Image) -> wgpu::TextureView {
        upload_texture(&self.device, &self.queue, img)
    }

    pub fn upload_mesh(&self, mesh: &MeshData) -> Result<GpuMesh> {
        let views: Vec<wgpu::TextureView> = mesh
            .textures
            .iter()
            .map(|t| self.upload_texture(t))
            .collect();
        let view = |i: Option<usize>| i.and_then(|i| views.get(i)).unwrap_or(&self.white);
        let mut materials = Vec::new();
        let mut batches = Vec::new();
        let mut keys: Vec<(Option<usize>, Option<usize>)> = Vec::new();
        let mut material_for = |texture: Option<usize>, lightmap: Option<usize>| {
            let key = (texture, lightmap);
            match keys.iter().position(|k| *k == key) {
                Some(i) => i,
                None => {
                    keys.push(key);
                    materials.push(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: None,
                        layout: &self.material_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(view(texture)),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.sampler_repeat),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(view(lightmap)),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Sampler(&self.sampler_repeat),
                            },
                        ],
                    }));
                    materials.len() - 1
                }
            }
        };
        for b in &mesh.batches {
            let mut material = material_for(b.texture, b.lightmap);
            // Flipbooks an instance's sequence texmods may show instead.
            let mut seq_flips = Vec::new();
            for (ai, a) in mesh.seq_anims.iter().enumerate() {
                if let (TexAnimKind::Flip(frames), true) = (&a.kind, Some(a.slot) == b.texture) {
                    let mats: Vec<usize> = frames
                        .iter()
                        .map(|f| material_for(f.or(b.texture), b.lightmap))
                        .collect();
                    // InitTexMod points every texmod's texture at its first frame.
                    material = mats[0];
                    seq_flips.push((ai, mats));
                }
            }
            // Global texmods of this batch's texture; when several flip it, the
            // last one run each frame wins.
            let mut flip = None;
            let mut scroll = [None; 2];
            for (ai, a) in mesh.tex_anims.iter().enumerate() {
                if Some(a.slot) != b.texture {
                    continue;
                }
                match &a.kind {
                    TexAnimKind::Flip(frames) => {
                        let mats: Vec<usize> = frames
                            .iter()
                            .map(|f| material_for(f.or(b.texture), b.lightmap))
                            .collect();
                        // Before the first step InitTexMod shows frame 0.
                        material = mats[0];
                        flip = Some((ai, mats));
                    }
                    TexAnimKind::ScrollU => scroll[0] = Some(ai),
                    TexAnimKind::ScrollV => scroll[1] = Some(ai),
                }
            }
            batches.push(GpuBatch {
                additive: b.additive,
                part: b.part,
                start: b.start,
                count: b.count,
                material,
                blend: b.blend,
                params: [
                    b.lightmap.is_some() as u32 as f32,
                    b.prelit as u32 as f32,
                    0.0,
                    0.0,
                ],
                uv: UV_IDENTITY,
                flip,
                scroll,
                seq_flips,
            });
        }
        // Opaque first, then blended, so transparent surfaces draw over the world.
        batches.sort_by_key(|b| (b.blend, b.additive));

        let verts = if mesh.vertices.is_empty() {
            vec![Vertex::zeroed()]
        } else {
            mesh.vertices.clone()
        };
        let idx = if mesh.indices.is_empty() {
            vec![0u32; 3]
        } else {
            mesh.indices.clone()
        };
        Ok(GpuMesh {
            vbuf: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("vbuf"),
                    contents: bytemuck::cast_slice(&verts),
                    usage: wgpu::BufferUsages::VERTEX,
                }),
            ibuf: self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("ibuf"),
                    contents: bytemuck::cast_slice(&idx),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            batches,
            materials,
            anims: mesh.tex_anims.iter().map(|a| a.texmod.clone()).collect(),
            bounds_min: mesh.bounds_min,
            bounds_max: mesh.bounds_max,
        })
    }

    pub fn new_instance(&self, mesh: &GpuMesh) -> Instance {
        let bufs = mesh
            .batches
            .iter()
            .map(|_| {
                let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: None,
                    size: std::mem::size_of::<ModelUniform>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout: &self.model_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buf.as_entire_binding(),
                    }],
                });
                (buf, bg)
            })
            .collect();
        Instance {
            bufs,
            transform: Mat4::IDENTITY,
            parts: Vec::new(),
            hidden: Vec::new(),
            tint: Vec4::ONE,
            visible: true,
            part_fx: Vec::new(),
            alt_tex: None,
        }
    }

    /// A material showing `img` (for [`Instance::alt_tex`]).
    pub fn texture_material(&self, img: &Image) -> std::sync::Arc<wgpu::BindGroup> {
        let view = self.upload_texture(img);
        std::sync::Arc::new(self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.material_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_repeat),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.white),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_repeat),
                },
            ],
        }))
    }

    pub fn depth_view(&self, width: u32, height: u32) -> wgpu::TextureView {
        self.device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&Default::default())
    }

    /// Render `draws` into `target`.
    pub fn render(
        &self,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        globals: &Globals,
        clear: [f64; 3],
        draws: &[(&GpuMesh, &Instance)],
    ) {
        self.render_viewport(target, depth, globals, clear, draws, None, None);
    }

    /// Draw over what is already in `target` (keeping its colour, clearing depth),
    /// within `viewport`: 3D pieces of the 2D overlay such as the menu arrow.
    pub fn render_over(
        &self,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        globals: &Globals,
        draws: &[(&GpuMesh, &Instance)],
        viewport: [f32; 4],
    ) {
        self.render_pass(target, depth, globals, None, draws, Some(viewport), None);
    }

    /// As [`Gpu::render`], drawing the scene only into `viewport` (x, y, w, h in
    /// pixels); the rest of the target is just cleared. `sprites` draw last.
    #[allow(clippy::too_many_arguments)]
    pub fn render_viewport(
        &self,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        globals: &Globals,
        clear: [f64; 3],
        draws: &[(&GpuMesh, &Instance)],
        viewport: Option<[f32; 4]>,
        sprites: Option<&SpriteLayer>,
    ) {
        self.render_pass(
            target,
            depth,
            globals,
            Some(clear),
            draws,
            viewport,
            sprites,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn render_pass(
        &self,
        target: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        globals: &Globals,
        clear: Option<[f64; 3]>,
        draws: &[(&GpuMesh, &Instance)],
        viewport: Option<[f32; 4]>,
        sprites: Option<&SpriteLayer>,
    ) {
        self.queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(globals));
        for (mesh, inst) in draws {
            if !inst.visible {
                continue;
            }
            for (b, (buf, _)) in mesh.batches.iter().zip(&inst.bufs) {
                if inst.hidden.get(b.part as usize).copied().unwrap_or(false) {
                    continue;
                }
                let (_, uv, alpha) = b.resolve(inst.part_fx.get(b.part as usize));
                let mut tint = inst.tint;
                tint.w *= alpha;
                let u = ModelUniform {
                    transform: (inst.transform
                        * inst
                            .parts
                            .get(b.part as usize)
                            .copied()
                            .unwrap_or(Mat4::IDENTITY))
                    .to_cols_array_2d(),
                    tint: tint.to_array(),
                    params: b.params,
                    uv,
                };
                self.queue.write_buffer(buf, 0, bytemuck::bytes_of(&u));
            }
        }
        let mut enc = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: match clear {
                            Some(c) => wgpu::LoadOp::Clear(wgpu::Color {
                                r: c[0],
                                g: c[1],
                                b: c[2],
                                a: 1.0,
                            }),
                            None => wgpu::LoadOp::Load,
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            if let Some([x, y, w, h]) = viewport {
                pass.set_viewport(x, y, w, h, 0.0, 1.0);
            }
            pass.set_bind_group(0, &self.globals_bg, &[]);
            // 0 = opaque (alpha test), 1 = alpha blended, 2 = additive.
            for pass_kind in 0..3u8 {
                pass.set_pipeline(match pass_kind {
                    0 => &self.opaque,
                    1 => &self.blend,
                    _ => &self.additive,
                });
                for (mesh, inst) in draws {
                    if !inst.visible {
                        continue;
                    }
                    pass.set_vertex_buffer(0, mesh.vbuf.slice(..));
                    pass.set_index_buffer(mesh.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                    for (b, (_, bg)) in mesh.batches.iter().zip(&inst.bufs) {
                        let (material, _, alpha) = b.resolve(inst.part_fx.get(b.part as usize));
                        // Parts faded by a texmod (or a whole instance faded,
                        // MBTreeSetAlpha) blend while they are see-through.
                        let kind = if b.additive {
                            2
                        } else {
                            (b.blend || alpha < 1.0 || inst.tint.w < 0.999) as u8
                        };
                        if kind != pass_kind
                            || b.count == 0
                            || inst.hidden.get(b.part as usize).copied().unwrap_or(false)
                        {
                            continue;
                        }
                        let mat = inst.alt_tex.as_deref().unwrap_or(&mesh.materials[material]);
                        pass.set_bind_group(1, mat, &[]);
                        pass.set_bind_group(2, bg, &[]);
                        pass.draw_indexed(b.start..b.start + b.count, 0, 0..1);
                    }
                }
            }
            if let Some(layer) = sprites.filter(|l| !l.draws.is_empty()) {
                pass.set_vertex_buffer(0, layer.vbuf.slice(..));
                pass.set_index_buffer(layer.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                for d in &layer.draws {
                    let k = blend_index(d.blend) * 4 + (d.ztest as usize) * 2 + d.zwrite as usize;
                    pass.set_pipeline(&self.sprite[k]);
                    let mat = d
                        .texture
                        .and_then(|t| layer.materials.get(t))
                        .unwrap_or(&layer.white);
                    pass.set_bind_group(1, mat, &[]);
                    pass.draw_indexed(d.first * 6..(d.first + d.count) * 6, 0, 0..1);
                }
            }
        }
        self.queue.submit([enc.finish()]);
    }

    /// Render offscreen and read back as RGBA8.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_image(
        &self,
        width: u32,
        height: u32,
        globals: &Globals,
        clear: [f64; 3],
        draws: &[(&GpuMesh, &Instance)],
        sprites: Option<&SpriteLayer>,
        overlay: Option<&dyn Fn(&wgpu::TextureView)>,
    ) -> Result<Image> {
        let tex = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.color_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = tex.create_view(&Default::default());
        let depth = self.depth_view(width, height);
        self.render_viewport(&view, &depth, globals, clear, draws, None, sprites);
        if let Some(f) = overlay {
            f(&view);
        }

        let row = (width * 4).div_ceil(256) * 256;
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = self.device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([enc.finish()]);
        let slice = buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::Wait)?;
        rx.recv()?.context("map readback buffer")?;
        let data = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            rgba.extend_from_slice(&data[(y * row) as usize..(y * row + width * 4) as usize]);
        }
        Ok(Image {
            width,
            height,
            rgba,
        })
    }
}

pub fn globals(view_proj: Mat4, camera: Vec3, fog_color: [f32; 3], fog_density: f32) -> Globals {
    Globals {
        view_proj: view_proj.to_cols_array_2d(),
        light: [0.4, 1.0, 0.3, 0.45],
        fog: [fog_color[0], fog_color[1], fog_color[2], fog_density],
        camera_pos: [camera.x, camera.y, camera.z, 1.0],
    }
}

fn upload_texture(device: &wgpu::Device, queue: &wgpu::Queue, img: &Image) -> wgpu::TextureView {
    // Generate a simple box-filtered mip chain on the CPU.
    let mut levels = vec![img.clone()];
    while levels.last().unwrap().width > 1 || levels.last().unwrap().height > 1 {
        let p = levels.last().unwrap();
        let (w, h) = ((p.width / 2).max(1), (p.height / 2).max(1));
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(p.width - 1);
                        let sy = (y * 2 + dy).min(p.height - 1);
                        sum += p.rgba[((sy * p.width + sx) * 4 + c) as usize] as u32;
                    }
                    rgba[((y * w + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        levels.push(Image {
            width: w,
            height: h,
            rgba,
        });
    }
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: img.width,
            height: img.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: levels.len() as u32,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for (i, l) in levels.iter().enumerate() {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: i as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &l.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(l.width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: l.width,
                height: l.height,
                depth_or_array_layers: 1,
            },
        );
    }
    tex.create_view(&Default::default())
}
