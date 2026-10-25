//! GPU-часть: конвейеры, буферы, карта теней. Живёт в ресурсах egui-wgpu
//! и вызывается из paint callback (см. [`WorldPaint`]).

use crate::character::{CharVertex, CharacterSet};
use crate::mapmesh::ChunkMeshes;
use crate::models::{self, MESH_COUNT};
use crate::voxel::{Mesh, Vertex};
use bytemuck::{Pod, Zeroable};
use eframe::egui_wgpu::{self, CallbackResources, CallbackTrait, ScreenDescriptor};
use eframe::wgpu::{self, util::DeviceExt};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

pub const SHADOW_SIZE: u32 = 2048;
pub const MAX_POINTS: usize = 32;
/// Формат буфера глубины. В eframe ему соответствует `depth_buffer: 32`.
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GroundPoint {
    pub center_radius: [f32; 4],
    pub color: [f32; 4],
}

/// Совпадает со структурой `Globals` в `world.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    pub light_view_proj: [[f32; 4]; 4],
    pub sun_dir: [f32; 4],
    pub sun_color: [f32; 4],
    pub ambient_sky: [f32; 4],
    pub ambient_ground: [f32; 4],
    pub fog: [f32; 4],
    pub eye: [f32; 4],
    pub params: [f32; 4],
    pub points: [GroundPoint; MAX_POINTS],
}

/// Экземпляр меша: матрица и оттенок (режимы — в `vs_voxel`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Instance {
    pub model: [[f32; 4]; 4],
    pub tint: [f32; 4],
}

/// Изменения мешей карты, которые GPU ещё не забрал.
///
/// Это общая очередь, а не часть кадра: eframe может не нарисовать какой-то кадр
/// (например, самый первый), и тогда изменения, отправленные только с ним, потерялись бы.
#[derive(Default)]
pub struct PendingMap {
    /// Карта сменилась целиком — выбросить все чанки.
    pub reset: bool,
    pub chunks: Vec<(u32, ChunkMeshes)>,
    /// Покрытие земли целиком: (ширина, высота, клетки).
    pub ground: Option<(u32, u32, Vec<u8>)>,
}

/// Всё, что нужно GPU для одного кадра. Собирает [`crate::scene::SceneBuilder`].
pub struct FrameData {
    pub globals: Globals,
    /// Экземпляры, сгруппированные по мешам.
    pub instances: Vec<Instance>,
    /// (меш, первый экземпляр, сколько).
    pub batches: Vec<(usize, u32, u32)>,
    /// То же для своих солдат и оружия: (меш персонажа, первый экземпляр, сколько).
    pub char_batches: Vec<(usize, u32, u32)>,
    pub map: Arc<Mutex<PendingMap>>,
    /// Чанки, которые стоит рисовать в этом кадре.
    pub visible_chunks: Vec<u32>,
}

struct GpuMesh {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    count: u32,
}

impl GpuMesh {
    fn new(device: &wgpu::Device, mesh: &Mesh, label: &str) -> Self {
        Self::raw(device, bytemuck::cast_slice(&mesh.vertices), &mesh.indices, label)
    }

    fn raw(device: &wgpu::Device, vertices: &[u8], indices: &[u32], label: &str) -> Self {
        GpuMesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: vertices,
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: indices.len() as u32,
        }
    }

    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, instances: std::ops::Range<u32>) {
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.count, 0, instances);
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderStats {
    pub chunks: usize,
    pub chunks_drawn: usize,
    pub instances: usize,
    pub draw_calls: usize,
}

pub struct WorldRenderer {
    voxel: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    ground: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
    ground_layout: wgpu::BindGroupLayout,
    ground_texture: wgpu::Texture,
    ground_bind: wgpu::BindGroup,
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    shadow_bind: wgpu::BindGroup,
    shadow_view: wgpu::TextureView,
    meshes: Vec<GpuMesh>,
    instances: wgpu::Buffer,
    capacity: usize,
    identity: wgpu::Buffer,
    /// Чанк карты: стены (если есть) и земля.
    chunks: HashMap<u32, (Option<GpuMesh>, GpuMesh)>,
    batches: Vec<(usize, u32, u32)>,
    visible: Vec<u32>,
    /// Свои солдаты и оружие: конвейеры, меши, атлас цветов.
    character: wgpu::RenderPipeline,
    character_shadow: wgpu::RenderPipeline,
    char_meshes: Vec<GpuMesh>,
    char_bind: wgpu::BindGroup,
    char_batches: Vec<(usize, u32, u32)>,
    pub stats: RenderStats,
}

const MESH_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm8x4, 2 => Unorm8x4];
const CHAR_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm8x4, 2 => Uint16x4];

fn char_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    let [_, instances] = vertex_layouts();
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<CharVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &CHAR_ATTRS,
        }),
        instances,
    ]
}
const INSTANCE_ATTRS: [wgpu::VertexAttribute; 5] =
    wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];

fn vertex_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &MESH_ATTRS,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<Instance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &INSTANCE_ATTRS,
        }),
    ]
}

impl WorldRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color_format: wgpu::TextureFormat,
        msaa: u32,
        meshes: &[Mesh],
        characters: Option<&CharacterSet>,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("world.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("world.wgsl").into()),
        });

        let uniform_entry = |visibility| wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let main_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("world globals"),
            entries: &[
                uniform_entry(wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        // В проходе теней карту теней читать нельзя — она туда пишется. Отдельный набор без неё.
        let shadow_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow globals"),
            entries: &[uniform_entry(wgpu::ShaderStages::VERTEX)],
        });

        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shadow_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow map"),
            size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("world globals"),
            layout: &main_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_sampler) },
            ],
        });
        let shadow_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow globals"),
            layout: &shadow_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals.as_entire_binding() }],
        });

        // Покрытие земли: номер материала на клетку, читается в шейдере без фильтрации.
        let ground_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ground"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let (ground_texture, ground_bind) = ground_texture(device, &ground_layout, 1, 1);

        let main_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("world"),
            bind_group_layouts: &[Some(&main_layout)],
            immediate_size: 0,
        });
        let shadow_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow"),
            bind_group_layouts: &[Some(&shadow_layout)],
            immediate_size: 0,
        });

        let depth = |write: bool, bias: wgpu::DepthBiasState| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias,
        };
        let color_target = [Some(wgpu::ColorTargetState {
            format: color_format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let multisample = wgpu::MultisampleState { count: msaa.max(1), mask: !0, alpha_to_coverage_enabled: false };
        let layouts = vertex_layouts();

        let voxel = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("voxel"),
            layout: Some(&main_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_voxel"),
                compilation_options: Default::default(),
                buffers: &layouts,
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(depth(true, Default::default())),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_voxel"),
                compilation_options: Default::default(),
                targets: &color_target,
            }),
            multiview_mask: None,
            cache: None,
        });
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                compilation_options: Default::default(),
                buffers: &layouts,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth(true, wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 })),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let ground = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ground"),
            layout: Some(&main_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ground"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth(true, Default::default())),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ground"),
                compilation_options: Default::default(),
                targets: &color_target,
            }),
            multiview_mask: None,
            cache: None,
        });

        let terrain_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("terrain"),
            bind_group_layouts: &[Some(&main_layout), Some(&ground_layout)],
            immediate_size: 0,
        });
        let terrain = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("terrain"),
            layout: Some(&terrain_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_voxel"),
                compilation_options: Default::default(),
                buffers: &layouts,
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(depth(true, Default::default())),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_terrain"),
                compilation_options: Default::default(),
                targets: &color_target,
            }),
            multiview_mask: None,
            cache: None,
        });

        // Свои солдаты: цвет каждого вокселя — из 3D-атласа (группа 2).
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("character atlas"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let (atlas_size, atlas_data) = match characters {
            Some(c) => (c.atlas_size, c.atlas.clone()),
            None => ([1, 1, 1], vec![255; 4]),
        };
        let atlas = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("character atlas"),
                size: wgpu::Extent3d { width: atlas_size[0], height: atlas_size[1], depth_or_array_layers: atlas_size[2] },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &atlas_data,
        );
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
        let char_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("character atlas"),
            layout: &atlas_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&atlas_view) }],
        });
        let char_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("character"),
            bind_group_layouts: &[Some(&main_layout), None, Some(&atlas_layout)],
            immediate_size: 0,
        });
        let char_vertex = char_layouts();
        let character = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("character"),
            layout: Some(&char_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_char"),
                compilation_options: Default::default(),
                buffers: &char_vertex,
            },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(depth(true, Default::default())),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_char"),
                compilation_options: Default::default(),
                targets: &color_target,
            }),
            multiview_mask: None,
            cache: None,
        });
        let character_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("character shadow"),
            layout: Some(&shadow_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_char_shadow"),
                compilation_options: Default::default(),
                buffers: &char_vertex,
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth(true, wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 })),
            multisample: wgpu::MultisampleState::default(),
            fragment: None,
            multiview_mask: None,
            cache: None,
        });
        let char_meshes = characters
            .map(|c| {
                c.meshes
                    .iter()
                    .map(|m| GpuMesh::raw(device, bytemuck::cast_slice(&m.vertices), &m.indices, "character part"))
                    .collect()
            })
            .unwrap_or_default();

        let identity = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("identity instance"),
            contents: bytemuck::bytes_of(&Instance {
                model: glam::Mat4::IDENTITY.to_cols_array_2d(),
                tint: [1.0, 1.0, 1.0, 0.0],
            }),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let capacity = 1024;
        WorldRenderer {
            voxel,
            shadow,
            ground,
            terrain,
            ground_layout,
            ground_texture,
            ground_bind,
            globals,
            bind,
            shadow_bind,
            shadow_view,
            meshes: meshes.iter().enumerate().map(|(i, m)| GpuMesh::new(device, m, &format!("mesh {i}"))).collect(),
            instances: instance_buffer(device, capacity),
            capacity,
            identity,
            chunks: HashMap::new(),
            batches: Vec::new(),
            visible: Vec::new(),
            character,
            character_shadow,
            char_meshes,
            char_bind,
            char_batches: Vec::new(),
            stats: RenderStats::default(),
        }
    }

    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &FrameData) {
        {
            let mut pending = frame.map.lock().unwrap_or_else(|e| e.into_inner());
            if std::mem::take(&mut pending.reset) {
                self.chunks.clear();
            }
            for (chunk, meshes) in pending.chunks.drain(..) {
                let walls = meshes.walls.map(|m| GpuMesh::new(device, &m, "map walls"));
                let terrain = GpuMesh::new(device, &meshes.terrain, "map terrain");
                self.chunks.insert(chunk, (walls, terrain));
            }
            if let Some((w, h, cells)) = pending.ground.take() {
                let size = self.ground_texture.size();
                if size.width != w || size.height != h {
                    (self.ground_texture, self.ground_bind) = ground_texture(device, &self.ground_layout, w, h);
                }
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.ground_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &cells,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                    wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                );
            }
        }
        queue.write_buffer(&self.globals, 0, bytemuck::bytes_of(&frame.globals));
        if frame.instances.len() > self.capacity {
            self.capacity = frame.instances.len().next_power_of_two();
            self.instances = instance_buffer(device, self.capacity);
        }
        if !frame.instances.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&frame.instances));
        }
        self.batches.clone_from(&frame.batches);
        self.char_batches.clone_from(&frame.char_batches);
        self.visible.clone_from(&frame.visible_chunks);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &self.shadow_view,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.shadow);
        pass.set_bind_group(0, &self.shadow_bind, &[]);
        self.draw_geometry(&mut pass);
        pass.set_pipeline(&self.character_shadow);
        self.draw_characters(&mut pass);
        drop(pass);

        self.stats = RenderStats {
            chunks: self.chunks.len(),
            chunks_drawn: self.visible.iter().filter(|c| self.chunks.contains_key(c)).count(),
            instances: frame.instances.len(),
            draw_calls: 0,
        };
        self.stats.draw_calls = (self.stats.chunks_drawn + self.batches.len()) * 2 + 1;
    }

    fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_pipeline(&self.ground);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.terrain);
        pass.set_bind_group(1, &self.ground_bind, &[]);
        pass.set_vertex_buffer(1, self.identity.slice(..));
        for chunk in &self.visible {
            if let Some((_, terrain)) = self.chunks.get(chunk) {
                terrain.draw(pass, 0..1);
            }
        }
        pass.set_pipeline(&self.voxel);
        self.draw_geometry(pass);
        pass.set_pipeline(&self.character);
        pass.set_bind_group(2, &self.char_bind, &[]);
        self.draw_characters(pass);
    }

    fn draw_characters(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for &(mesh, first, count) in &self.char_batches {
            if count > 0 {
                self.char_meshes[mesh].draw(pass, first..first + count);
            }
        }
    }

    /// Стены и экземпляры: и в проходе теней, и в основном. Земля тени не отбрасывает.
    fn draw_geometry(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(1, self.identity.slice(..));
        for chunk in &self.visible {
            if let Some((Some(walls), _)) = self.chunks.get(chunk) {
                walls.draw(pass, 0..1);
            }
        }
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for &(mesh, first, count) in &self.batches {
            if count > 0 {
                self.meshes[mesh].draw(pass, first..first + count);
            }
        }
    }
}

fn ground_texture(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, w: u32, h: u32) -> (wgpu::Texture, wgpu::BindGroup) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("ground"),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Uint,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ground"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
    });
    (texture, bind)
}

fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("instances"),
        size: (capacity * size_of::<Instance>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Создаёт отрисовщик мира и кладёт его в ресурсы egui-wgpu.
/// `msaa` должен совпадать с `NativeOptions::multisampling`, а в `NativeOptions`
/// должен быть `depth_buffer: 32`. Возвращает заметки о загруженных моделях.
pub fn install(render_state: &egui_wgpu::RenderState, msaa: u32, data_dir: Option<&Path>) -> Installed {
    let (meshes, mut notes) = models::build_meshes(data_dir.map(|d| d.join("models/soldier")).as_deref());
    debug_assert_eq!(meshes.len(), MESH_COUNT);
    let characters = match data_dir.map(CharacterSet::load) {
        Some(Ok(set)) => {
            notes.push(format!(
                "Солдаты: {} моделей, {} видов оружия, атлас {}×{}×{}",
                set.models.len(),
                set.weapons.len(),
                set.atlas_size[0],
                set.atlas_size[1],
                set.atlas_size[2]
            ));
            Some(Arc::new(set))
        }
        Some(Err(e)) => {
            notes.push(format!("{e} — рисую встроенных солдат"));
            None
        }
        None => None,
    };
    let renderer = WorldRenderer::new(
        &render_state.device,
        &render_state.queue,
        render_state.target_format,
        msaa,
        &meshes,
        characters.as_deref(),
    );
    render_state.renderer.write().callback_resources.insert(renderer);
    Installed { notes, characters }
}

/// Что получилось при установке отрисовщика.
pub struct Installed {
    pub notes: Vec<String>,
    /// Свои солдаты из `data/factions.ron`. Их нужно отдать каждому [`crate::SceneBuilder`].
    pub characters: Option<Arc<CharacterSet>>,
}

/// Статистика последнего кадра.
pub fn stats(render_state: &egui_wgpu::RenderState) -> RenderStats {
    render_state
        .renderer
        .read()
        .callback_resources
        .get::<WorldRenderer>()
        .map(|r| r.stats.clone())
        .unwrap_or_default()
}

/// Paint callback: в `prepare` загружает кадр и рисует тени, в `paint` — мир.
pub struct WorldPaint(pub Arc<FrameData>);

impl CallbackTrait for WorldPaint {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(r) = resources.get_mut::<WorldRenderer>() {
            r.prepare(device, queue, encoder, &self.0);
        }
        Vec::new()
    }

    fn paint(&self, _info: eframe::egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &CallbackResources) {
        if let Some(r) = resources.get::<WorldRenderer>() {
            r.paint(pass);
        }
    }
}
