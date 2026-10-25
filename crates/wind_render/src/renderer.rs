//! GPU-часть: конвейеры, буферы, карта теней. Живёт в ресурсах egui-wgpu
//! и вызывается из paint callback (см. [`WorldPaint`]).

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
    /// (чанк, меш); `None` — чанк теперь пуст.
    pub chunks: Vec<(u32, Option<Mesh>)>,
}

/// Всё, что нужно GPU для одного кадра. Собирает [`crate::scene::SceneBuilder`].
pub struct FrameData {
    pub globals: Globals,
    /// Экземпляры, сгруппированные по мешам.
    pub instances: Vec<Instance>,
    /// (меш, первый экземпляр, сколько).
    pub batches: Vec<(usize, u32, u32)>,
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
        GpuMesh {
            vertices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&mesh.vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            indices: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&mesh.indices),
                usage: wgpu::BufferUsages::INDEX,
            }),
            count: mesh.indices.len() as u32,
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
    globals: wgpu::Buffer,
    bind: wgpu::BindGroup,
    shadow_bind: wgpu::BindGroup,
    shadow_view: wgpu::TextureView,
    meshes: Vec<GpuMesh>,
    instances: wgpu::Buffer,
    capacity: usize,
    identity: wgpu::Buffer,
    chunks: HashMap<u32, GpuMesh>,
    batches: Vec<(usize, u32, u32)>,
    visible: Vec<u32>,
    pub stats: RenderStats,
}

const MESH_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm8x4, 2 => Unorm8x4];
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
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, msaa: u32, meshes: &[Mesh]) -> Self {
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
            stats: RenderStats::default(),
        }
    }

    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, encoder: &mut wgpu::CommandEncoder, frame: &FrameData) {
        {
            let mut pending = frame.map.lock().unwrap_or_else(|e| e.into_inner());
            if std::mem::take(&mut pending.reset) {
                self.chunks.clear();
            }
            for (chunk, mesh) in pending.chunks.drain(..) {
                match mesh {
                    Some(m) => {
                        self.chunks.insert(chunk, GpuMesh::new(device, &m, "map chunk"));
                    }
                    None => {
                        self.chunks.remove(&chunk);
                    }
                }
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
        pass.set_pipeline(&self.voxel);
        self.draw_geometry(pass);
    }

    fn draw_geometry(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(1, self.identity.slice(..));
        for chunk in &self.visible {
            if let Some(mesh) = self.chunks.get(chunk) {
                mesh.draw(pass, 0..1);
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
pub fn install(render_state: &egui_wgpu::RenderState, msaa: u32, models_dir: Option<&Path>) -> Vec<String> {
    let (meshes, notes) = models::build_meshes(models_dir);
    debug_assert_eq!(meshes.len(), MESH_COUNT);
    let renderer = WorldRenderer::new(&render_state.device, render_state.target_format, msaa, &meshes);
    render_state.renderer.write().callback_resources.insert(renderer);
    notes
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
