//! GPU-часть: конвейеры, буферы, карта теней. Живёт в ресурсах egui-wgpu
//! и вызывается из paint callback (см. [`WorldPaint`]).

use crate::character::{CharVertex, CharacterSet, LODS, PointVertex};
use crate::grass::{CardVertex, GrassAssets, GrassUpload, Piece};
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
    pub view: [[f32; 4]; 4],
    /// Насыщенность, контраст, яркость, ступенчатость света.
    pub grade: [f32; 4],
    /// Оттенок картинки (rgb).
    pub tint: [f32; 4],
    /// Обводка, затенение, виньетка, дальность для глубины.
    pub post: [f32; 4],
    /// Вьюпорт в пикселях кадра: x, y, ширина, высота.
    pub screen: [f32; 4],
    /// x — ширина плавного сгиба у суставов солдат, воксели (0 — без сгиба).
    pub skin: [f32; 4],
    pub points: [GroundPoint; MAX_POINTS],
}

/// Экземпляр меша: матрица и оттенок (режимы — в `vs_voxel`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Instance {
    pub model: [[f32; 4]; 4],
    pub tint: [f32; 4],
}

/// Экземпляр своего солдата или оружия: с какой матрицы в буфере палок начинаются
/// его палки, признаки (бит 0 — оружие) и цвет команды (RGBA8).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SkinInstance {
    pub base: u32,
    pub flags: u32,
    pub team: u32,
    pub _pad: u32,
}

/// Клуб пыли или дыма: мягкий полупрозрачный круг, повёрнутый к камере.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Particle {
    pub pos: [f32; 3],
    /// Поперечник, м.
    pub size: f32,
    /// Цвет sRGB и непрозрачность.
    pub color: [f32; 4],
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
    /// Новая текстура следов боя (ширина, высота) — при смене карты.
    pub decal_size: Option<(u32, u32)>,
    /// Изменённые прямоугольники следов: (x, y, ширина, высота) и тексели по 2 байта.
    pub decal_rects: Vec<([u32; 4], Vec<u8>)>,
    /// Новые и изменённые модели предметов: (имя, меш).
    pub props: Vec<(String, Mesh)>,
    /// Трава карты — целиком, при смене карты, покрытия или предметов.
    pub grass: Option<GrassUpload>,
    /// Новые файлы травы (куски и атлас) — при запуске и когда их перевыгрузили.
    pub grass_assets: Option<GrassAssets>,
}

/// Всё, что нужно GPU для одного кадра. Собирает [`crate::scene::SceneBuilder`].
pub struct FrameData {
    pub globals: Globals,
    /// Экземпляры, сгруппированные по мешам.
    pub instances: Vec<Instance>,
    /// (меш, первый экземпляр, сколько).
    pub batches: Vec<(usize, u32, u32)>,
    /// Предметы: (модель, первый экземпляр, сколько) — экземпляры после обычных.
    pub prop_batches: Vec<(String, u32, u32)>,
    /// Свои солдаты и оружие: (меш, уровень детализации, первый экземпляр, сколько).
    pub char_batches: Vec<(usize, usize, u32, u32)>,
    pub skins: Vec<SkinInstance>,
    /// Матрицы палок всех солдат и оружия подряд (см. `SkinInstance::base`).
    pub bones: Vec<[[f32; 4]; 4]>,
    /// Пыль и дым, от дальних к ближним.
    pub particles: Vec<Particle>,
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
    decal_texture: wgpu::Texture,
    decal_sampler: wgpu::Sampler,
    /// Густота травы по клеткам (см. `GrassUpload::density`).
    grass_density: wgpu::Texture,
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
    /// Меши чанков карты: стены, земля, вода.
    chunks: HashMap<u32, (Option<GpuMesh>, GpuMesh, Option<GpuMesh>)>,
    batches: Vec<(usize, u32, u32)>,
    /// Модели предметов по имени (см. `props`).
    props: HashMap<String, GpuMesh>,
    prop_batches: Vec<(String, u32, u32)>,
    visible: Vec<u32>,
    /// Свои солдаты и оружие: тени — гранями, картинка — точками, как у RWR.
    character_shadow: wgpu::RenderPipeline,
    char_meshes: Vec<GpuMesh>,
    char_points: Vec<[GpuMesh; LODS]>,
    /// Силуэт сквозь стены, чёрная обводка, цветные точки.
    char_ghost: wgpu::RenderPipeline,
    char_outline: wgpu::RenderPipeline,
    char_color: wgpu::RenderPipeline,
    char_batches: Vec<(usize, usize, u32, u32)>,
    skin_layout: wgpu::BindGroupLayout,
    skin_bind: wgpu::BindGroup,
    skin_instances: wgpu::Buffer,
    skin_capacity: usize,
    bones: wgpu::Buffer,
    bones_capacity: usize,
    /// Обводка: проходы нормалей и глубины и полноэкранный умножающий проход.
    nd_voxel: wgpu::RenderPipeline,
    nd_char: wgpu::RenderPipeline,
    post: wgpu::RenderPipeline,
    water: wgpu::RenderPipeline,
    post_layout: wgpu::BindGroupLayout,
    /// Пыль и дым: прозрачные, рисуются последними.
    particles: wgpu::RenderPipeline,
    particle_buffer: wgpu::Buffer,
    particle_capacity: usize,
    particle_count: u32,
    nd: Option<NdTarget>,
    /// Трава карточками: конвейеры (картинка и тень, обе — с отсечением по альфе), атлас,
    /// меши кусков, куски карты и где в них какой вид и чанк.
    grass: GrassGpu,
    pub stats: RenderStats,
}

struct GrassGpu {
    pipeline: wgpu::RenderPipeline,
    shadow: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// Атлас и меши кусков — когда файлы травы прочитаны.
    bind: Option<wgpu::BindGroup>,
    meshes: Vec<GpuMesh>,
    pieces: Option<wgpu::Buffer>,
    /// Вид × чанков + чанк → (первый кусок, сколько).
    ranges: Vec<(u32, u32)>,
    chunks: usize,
}

const CARD_ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2];
const PIECE_ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![2 => Float32x4, 3 => Float32x2, 4 => Uint32x2];

fn grass_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<CardVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &CARD_ATTRS,
        }),
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<Piece>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &PIECE_ATTRS,
        }),
    ]
}

impl GrassGpu {
    #[allow(clippy::too_many_arguments)]
    fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        main_layout: &wgpu::BindGroupLayout,
        shadow_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        msaa: u32,
    ) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grass atlas"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let main = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grass"),
            bind_group_layouts: &[Some(main_layout), Some(&layout)],
            immediate_size: 0,
        });
        let shadow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grass shadow"),
            bind_group_layouts: &[Some(shadow_layout), Some(&layout)],
            immediate_size: 0,
        });
        let buffers = grass_layouts();
        let depth = |bias: wgpu::DepthBiasState| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias,
        };
        // Края стеблей — через альфу в покрытие MSAA (сглаженно), альфу кадра не трогаем.
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grass"),
            layout: Some(&main),
            vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_grass"), compilation_options: Default::default(), buffers: &buffers },
            primitive: wgpu::PrimitiveState { cull_mode: Some(wgpu::Face::Back), ..Default::default() },
            depth_stencil: Some(depth(Default::default())),
            multisample: wgpu::MultisampleState { count: msaa.max(1), mask: !0, alpha_to_coverage_enabled: msaa > 1 },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_grass"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: None, write_mask: wgpu::ColorWrites::COLOR })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grass shadow"),
            layout: Some(&shadow_pl),
            vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_grass_shadow"), compilation_options: Default::default(), buffers: &buffers },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth(wgpu::DepthBiasState { constant: 2, slope_scale: 2.0, clamp: 0.0 })),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState { module: shader, entry_point: Some("fs_grass_shadow"), compilation_options: Default::default(), targets: &[] }),
            multiview_mask: None,
            cache: None,
        });
        GrassGpu { pipeline, shadow, layout, bind: None, meshes: Vec::new(), pieces: None, ranges: Vec::new(), chunks: 0 }
    }

    /// Новые куски и атлас (при запуске и после перевыгрузки из Blender).
    fn set_assets(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, assets: &GrassAssets) {
        let layout = &self.layout;
        // Атлас с мип-уровнями (альфа подобрана для отсечения, см. `grass::atlas_mips`).
        // Цвета в нём — sRGB, но формат «как есть»: свет (`light`) сам переводит из sRGB.
        let (w, h, rgba) = &assets.atlas;
        let mips = crate::grass::atlas_mips(*w, *h, rgba);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("grass atlas"),
            size: wgpu::Extent3d { width: *w, height: *h, depth_or_array_layers: 1 },
            mip_level_count: mips.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, (lw, lh, texels)) in mips.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                texels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(lw * 4), rows_per_image: Some(*lh) },
                wgpu::Extent3d { width: *lw, height: *lh, depth_or_array_layers: 1 },
            );
        }
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("grass atlas"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grass atlas"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });
        self.meshes = assets
            .meshes
            .iter()
            .enumerate()
            .map(|(i, (v, idx))| GpuMesh::raw(device, bytemuck::cast_slice(v), idx, &format!("grass {i}")))
            .collect();
        self.bind = Some(bind);
    }

    fn upload(&mut self, device: &wgpu::Device, up: GrassUpload) {
        self.pieces = (!up.pieces.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("grass pieces"),
                contents: bytemuck::cast_slice(&up.pieces),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        self.ranges = up.ranges;
        self.chunks = up.chunks;
    }

    /// Куски видимых чанков: по видам, соседние диапазоны — одним вызовом.
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, visible: &[u32]) -> usize {
        let (Some(pieces), Some(bind)) = (&self.pieces, &self.bind) else { return 0 };
        pass.set_bind_group(1, bind, &[]);
        pass.set_vertex_buffer(1, pieces.slice(..));
        let mut sorted: Vec<u32> = visible.iter().copied().filter(|&c| (c as usize) < self.chunks).collect();
        sorted.sort_unstable();
        let mut calls = 0;
        for (kind, mesh) in self.meshes.iter().enumerate() {
            let mut run: Option<(u32, u32)> = None;
            for &c in &sorted {
                let (first, count) = self.ranges[kind * self.chunks + c as usize];
                if count == 0 {
                    continue;
                }
                run = match run {
                    Some((a, b)) if b == first => Some((a, first + count)),
                    Some((a, b)) => {
                        mesh.draw(pass, a..b);
                        calls += 1;
                        Some((first, first + count))
                    }
                    None => Some((first, first + count)),
                };
            }
            if let Some((a, b)) = run {
                mesh.draw(pass, a..b);
                calls += 1;
            }
        }
        calls
    }
}

const MESH_ATTRS: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Snorm8x4, 2 => Unorm8x4];
const CHAR_ATTRS: [wgpu::VertexAttribute; 2] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint32];
const POINT_ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Uint8x4, 2 => Uint8x4];
const SKIN_ATTRS: [wgpu::VertexAttribute; 1] = wgpu::vertex_attr_array![3 => Uint32x4];

const SKIN_INSTANCES: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: size_of::<SkinInstance>() as u64,
    step_mode: wgpu::VertexStepMode::Instance,
    attributes: &SKIN_ATTRS,
};

fn point_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<PointVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &POINT_ATTRS,
        }),
        Some(SKIN_INSTANCES),
    ]
}

fn char_layouts() -> [Option<wgpu::VertexBufferLayout<'static>>; 2] {
    [
        Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<CharVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &CHAR_ATTRS,
        }),
        Some(SKIN_INSTANCES),
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
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // Следы боя: подпалины (r) и кровь (g), со сглаживанием.
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Густота травы по клеткам — подложка тона травы на земле.
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let ground_texture = layer_texture(device, "ground", wgpu::TextureFormat::R8Uint, 1, 1);
        let decal_texture = layer_texture(device, "decals", wgpu::TextureFormat::Rg8Unorm, 1, 1);
        let decal_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("decals"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let grass_density = layer_texture(device, "grass density", wgpu::TextureFormat::R8Unorm, 1, 1);
        let ground_bind = ground_bind(device, &ground_layout, &ground_texture, &decal_texture, &grass_density, &decal_sampler);

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
        // Матрицы палок солдат (группа 1): читает только вершинный шейдер.
        let skin_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bones"),
            entries: &[wgpu::BindGroupLayoutEntry {
                // Номер 4 — чтобы не совпасть с текстурами группы 1 у других конвейеров.
                binding: 4,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let char_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("character"),
            bind_group_layouts: &[Some(&main_layout), Some(&skin_layout)],
            immediate_size: 0,
        });
        let char_shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("character shadow"),
            bind_group_layouts: &[Some(&shadow_layout), Some(&skin_layout)],
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

        // Свои солдаты — точками, как у RWR. Три прохода: силуэт цвета команды там,
        // где солдата закрывает стена (глубина «дальше»), крупные чёрные точки (обводка)
        // и цветные точки поверх.
        let char_vertex = char_layouts();
        let point_vertex = point_layouts();
        let point_pipeline = |label: &str, vs: &str, fs: &str, compare: wgpu::CompareFunction, write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&char_pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers: &point_vertex },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(write),
                    depth_compare: Some(compare),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &color_target,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let char_ghost = point_pipeline("character ghost", "vs_point_ghost", "fs_point_ghost", wgpu::CompareFunction::Greater, false);
        let char_outline = point_pipeline("character outline", "vs_point_outline", "fs_point_outline", wgpu::CompareFunction::Less, true);
        let char_color = point_pipeline("character points", "vs_point", "fs_point", wgpu::CompareFunction::Less, true);
        let character_shadow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("character shadow"),
            layout: Some(&char_shadow_layout),
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
        // --- Обводка: нормали и глубина в свою текстуру, потом умножающий проход.
        let nd_target = [Some(wgpu::ColorTargetState { format: ND_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })];
        let nd_pipeline = |label: &str, fs: &str, buffers: &[Option<wgpu::VertexBufferLayout<'static>>], vs: &str, layout: &wgpu::PipelineLayout| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(vs), compilation_options: Default::default(), buffers },
                // Солдаты-точки — квадратики «на камеру», их не отсекаем.
                primitive: wgpu::PrimitiveState {
                    cull_mode: (!vs.starts_with("vs_point")).then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: Some(depth(true, Default::default())),
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &nd_target,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let nd_voxel = nd_pipeline("nd voxel", "fs_nd", &layouts, "vs_voxel", &main_pipeline_layout);
        let nd_char = nd_pipeline("nd character", "fs_point_nd", &point_vertex, "vs_point_nd", &char_pipeline_layout);
        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let post_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&main_layout), Some(&post_layout)],
            immediate_size: 0,
        });
        // Умножение: итог = нарисованное × множитель из шейдера.
        let multiply = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Dst, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let post = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post"),
            layout: Some(&post_pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_post"), compilation_options: Default::default(), buffers: &[] },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_post"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(multiply), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // Вода: прозрачная, глубину не пишет (видно то, что под ней), рисуется после обводки.
        let water = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("water"),
            layout: Some(&post_pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_water"), compilation_options: Default::default(), buffers: &layouts },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth(false, Default::default())),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_water"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // Пыль и дым: квадрат из 6 вершин на клуб, прозрачность, без записи глубины.
        const PARTICLE_ATTRS: [wgpu::VertexAttribute; 3] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x4];
        let particles = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("particles"),
            layout: Some(&main_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_particle"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Particle>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &PARTICLE_ATTRS,
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_particle"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let particle_capacity = 256;
        let grass = GrassGpu::new(device, &shader, &main_layout, &shadow_layout, color_format, msaa);

        let (char_meshes, char_points) = characters
            .map(|c| {
                c.meshes
                    .iter()
                    .map(|m| {
                        (
                            GpuMesh::raw(device, bytemuck::cast_slice(&m.vertices), &m.indices, "character part"),
                            m.lods.each_ref().map(|l| GpuMesh::raw(device, bytemuck::cast_slice(&l.points), &l.indices, "character points")),
                        )
                    })
                    .unzip()
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
        let skin_capacity = 1024;
        let bones_capacity = 16 * 1024;
        let bones = bones_buffer(device, bones_capacity);
        let skin_bind = skin_bind(device, &skin_layout, &bones);
        WorldRenderer {
            voxel,
            shadow,
            ground,
            terrain,
            ground_layout,
            ground_texture,
            decal_texture,
            decal_sampler,
            grass_density,
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
            props: HashMap::new(),
            prop_batches: Vec::new(),
            visible: Vec::new(),
            character_shadow,
            char_meshes,
            char_points,
            char_ghost,
            char_outline,
            char_color,
            char_batches: Vec::new(),
            skin_instances: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("skin instances"),
                size: (skin_capacity * size_of::<SkinInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            skin_capacity,
            skin_layout,
            skin_bind,
            bones,
            bones_capacity,
            nd_voxel,
            nd_char,
            post,
            water,
            post_layout,
            particles,
            particle_buffer: particle_buffer(device, particle_capacity),
            particle_capacity,
            particle_count: 0,
            nd: None,
            grass,
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
                let water = meshes.water.map(|m| GpuMesh::new(device, &m, "map water"));
                self.chunks.insert(chunk, (walls, terrain, water));
            }
            if let Some(assets) = pending.grass_assets.take() {
                self.grass.set_assets(device, queue, &assets);
            }
            if let Some(mut up) = pending.grass.take() {
                let (w, h, cells) = std::mem::take(&mut up.density);
                let size = self.grass_density.size();
                if size.width != w || size.height != h {
                    self.grass_density = layer_texture(device, "grass density", wgpu::TextureFormat::R8Unorm, w, h);
                    self.rebind_ground(device);
                }
                if w * h > 0 {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo { texture: &self.grass_density, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                        &cells,
                        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    );
                }
                self.grass.upload(device, up);
            }
            for (name, mesh) in pending.props.drain(..) {
                let gpu = GpuMesh::new(device, &mesh, &format!("prop {name}"));
                self.props.insert(name, gpu);
            }
            if let Some((w, h, cells)) = pending.ground.take() {
                let size = self.ground_texture.size();
                if size.width != w || size.height != h {
                    self.ground_texture = layer_texture(device, "ground", wgpu::TextureFormat::R8Uint, w, h);
                    self.rebind_ground(device);
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
            if let Some((w, h)) = pending.decal_size.take() {
                self.decal_texture = layer_texture(device, "decals", wgpu::TextureFormat::Rg8Unorm, w, h);
                self.rebind_ground(device);
            }
            for ([x, y, w, h], texels) in pending.decal_rects.drain(..) {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.decal_texture,
                        mip_level: 0,
                        origin: wgpu::Origin3d { x, y, z: 0 },
                        aspect: wgpu::TextureAspect::All,
                    },
                    &texels,
                    wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 2), rows_per_image: Some(h) },
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
        self.prop_batches.clone_from(&frame.prop_batches);
        self.char_batches.clone_from(&frame.char_batches);
        if frame.particles.len() > self.particle_capacity {
            self.particle_capacity = frame.particles.len().next_power_of_two();
            self.particle_buffer = particle_buffer(device, self.particle_capacity);
        }
        if !frame.particles.is_empty() {
            queue.write_buffer(&self.particle_buffer, 0, bytemuck::cast_slice(&frame.particles));
        }
        self.particle_count = frame.particles.len() as u32;
        if frame.skins.len() > self.skin_capacity {
            self.skin_capacity = frame.skins.len().next_power_of_two();
            self.skin_instances = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("skin instances"),
                size: (self.skin_capacity * size_of::<SkinInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !frame.skins.is_empty() {
            queue.write_buffer(&self.skin_instances, 0, bytemuck::cast_slice(&frame.skins));
        }
        if frame.bones.len() > self.bones_capacity {
            self.bones_capacity = frame.bones.len().next_power_of_two();
            self.bones = bones_buffer(device, self.bones_capacity);
            self.skin_bind = skin_bind(device, &self.skin_layout, &self.bones);
        }
        if !frame.bones.is_empty() {
            queue.write_buffer(&self.bones, 0, bytemuck::cast_slice(&frame.bones));
        }
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
        pass.set_bind_group(1, &self.skin_bind, &[]);
        pass.set_vertex_buffer(1, self.skin_instances.slice(..));
        for &(mesh, _, first, count) in &self.char_batches {
            self.char_meshes[mesh].draw(&mut pass, first..first + count);
        }
        pass.set_pipeline(&self.grass.shadow);
        self.grass.draw(&mut pass, &self.visible);
        drop(pass);

        // Проход нормалей и глубины — для обводки и затенения.
        let size = [frame.globals.screen[2].max(1.0) as u32, frame.globals.screen[3].max(1.0) as u32];
        if self.nd.as_ref().is_none_or(|t| t.size != size) {
            self.nd = Some(NdTarget::new(device, &self.post_layout, size));
        }
        let nd = self.nd.as_ref().unwrap();
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("normal-depth pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &nd.color,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
                depth_slice: None,
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &nd.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &self.bind, &[]);
        pass.set_pipeline(&self.nd_voxel);
        pass.set_vertex_buffer(1, self.identity.slice(..));
        for chunk in &self.visible {
            if let Some((_, terrain, _)) = self.chunks.get(chunk) {
                terrain.draw(&mut pass, 0..1);
            }
        }
        self.draw_geometry(&mut pass);
        pass.set_pipeline(&self.nd_char);
        self.draw_characters(&mut pass);
        drop(pass);

        self.stats = RenderStats {
            chunks: self.chunks.len(),
            chunks_drawn: self.visible.iter().filter(|c| self.chunks.contains_key(c)).count(),
            instances: frame.instances.len() + frame.skins.len(),
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
            if let Some((_, terrain, _)) = self.chunks.get(chunk) {
                terrain.draw(pass, 0..1);
            }
        }
        pass.set_pipeline(&self.voxel);
        self.draw_geometry(pass);
        // Силуэт — до самих солдат: в буфере глубины пока только земля и стены.
        for pipeline in [&self.char_ghost, &self.char_outline, &self.char_color] {
            pass.set_pipeline(pipeline);
            self.draw_characters(pass);
        }
        // Трава — после солдат: силуэт сквозь стены не должен вспыхивать за каждым пучком,
        // а солдата в траве она закрывает по глубине.
        pass.set_pipeline(&self.grass.pipeline);
        self.grass.draw(pass, &self.visible);
        if let Some(nd) = &self.nd {
            pass.set_pipeline(&self.post);
            pass.set_bind_group(1, &nd.bind, &[]);
            pass.draw(0..3, 0..1);
        }
        // Вода — после обводки (контуры под водой — под ней) и до дыма. Глубину под водой
        // берёт из прохода нормалей и глубины.
        if let Some(nd) = &self.nd {
            pass.set_pipeline(&self.water);
            pass.set_bind_group(1, &nd.bind, &[]);
            pass.set_vertex_buffer(1, self.identity.slice(..));
            for chunk in &self.visible {
                if let Some((_, _, Some(water))) = self.chunks.get(chunk) {
                    water.draw(pass, 0..1);
                }
            }
        }
        // Пыль и дым — после обводки: контуры стен не должны просвечивать сквозь дым.
        if self.particle_count > 0 {
            pass.set_pipeline(&self.particles);
            pass.set_vertex_buffer(0, self.particle_buffer.slice(..));
            pass.draw(0..6, 0..self.particle_count);
        }
    }

    fn rebind_ground(&mut self, device: &wgpu::Device) {
        self.ground_bind = ground_bind(device, &self.ground_layout, &self.ground_texture, &self.decal_texture, &self.grass_density, &self.decal_sampler);
    }

    fn draw_characters(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_bind_group(1, &self.skin_bind, &[]);
        pass.set_vertex_buffer(1, self.skin_instances.slice(..));
        for &(mesh, lod, first, count) in &self.char_batches {
            if count > 0 {
                self.char_points[mesh][lod].draw(pass, first..first + count);
            }
        }
    }

    /// Стены и экземпляры: и в проходе теней, и в основном. Земля тени не отбрасывает.
    fn draw_geometry(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_vertex_buffer(1, self.identity.slice(..));
        for chunk in &self.visible {
            if let Some((Some(walls), _, _)) = self.chunks.get(chunk) {
                walls.draw(pass, 0..1);
            }
        }
        pass.set_vertex_buffer(1, self.instances.slice(..));
        for &(mesh, first, count) in &self.batches {
            if count > 0 {
                self.meshes[mesh].draw(pass, first..first + count);
            }
        }
        for (name, first, count) in &self.prop_batches {
            if let Some(mesh) = self.props.get(name) {
                mesh.draw(pass, *first..first + count);
            }
        }
    }
}

fn particle_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("particles"),
        size: (capacity * size_of::<Particle>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn bones_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("bones"),
        size: (capacity * 64) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn skin_bind(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, bones: &wgpu::Buffer) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bones"),
        layout,
        entries: &[wgpu::BindGroupEntry { binding: 4, resource: bones.as_entire_binding() }],
    })
}

fn layer_texture(device: &wgpu::Device, label: &str, format: wgpu::TextureFormat, w: u32, h: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn ground_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    ground: &wgpu::Texture,
    decals: &wgpu::Texture,
    grass: &wgpu::Texture,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    let ground = ground.create_view(&wgpu::TextureViewDescriptor::default());
    let decals = decals.create_view(&wgpu::TextureViewDescriptor::default());
    let grass = grass.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ground"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&ground) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&decals) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(&grass) },
        ],
    })
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
            let points: usize = set.meshes.iter().map(|m| m.lods[0].points.len() / 4).sum();
            notes.push(format!("Солдаты: {} моделей, {} видов оружия, точек {points}", set.models.len(), set.weapons.len()));
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

/// Формат текстуры нормалей и глубины: xyz — нормаль в пространстве камеры, w — расстояние.
const ND_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// Текстура нормалей и глубины размером с вьюпорт.
struct NdTarget {
    size: [u32; 2],
    color: wgpu::TextureView,
    depth: wgpu::TextureView,
    bind: wgpu::BindGroup,
}

impl NdTarget {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, size: [u32; 2]) -> Self {
        let make = |label: &str, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let color = make("normal-depth", ND_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let depth = make("normal-depth depth", DEPTH_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&color) }],
        });
        NdTarget { size, color, depth, bind }
    }
}
