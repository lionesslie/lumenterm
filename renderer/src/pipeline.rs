use bytemuck::{Pod, Zeroable};

/// Tek bir çizim biriminin (hücre arka planı, glyph veya cursor/seçim
/// vurgusu) GPU instance verisi. Aynı instance tipi üç amaçla da
/// kullanılır: `has_glyph=0` düz dolgu dikdörtgen (arka plan, seçim,
/// cursor), `has_glyph=1` atlas'tan örneklenen bir glyph.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CellInstance {
    /// Ekran-uzayı pozisyonu (piksel, sol-üst köşe).
    pub pos: [f32; 2],
    /// Dikdörtgen boyutu (piksel).
    pub size: [f32; 2],
    pub bg_color: [f32; 4],
    pub fg_color: [f32; 4],
    /// Glyph atlas UV aralığı; `has_glyph=0` iken kullanılmaz.
    pub atlas_uv_min: [f32; 2],
    pub atlas_uv_max: [f32; 2],
    /// 0 = düz renk dikdörtgen, 1 = atlas'tan örnekle.
    pub has_glyph: f32,
    /// 1 = atlas içeriği doğrudan RGBA (renkli emoji), 0 = tek kanal
    /// coverage maskesi (`fg_color` ile çarpılır). `has_glyph=0` iken
    /// anlamsız.
    pub color_glyph: f32,
    /// Faz 5 chrome cilası: düz renk dikdörtgenler (`has_glyph=0`) için
    /// piksel cinsinden köşe yarıçapı (SDF ile fragment shader'da
    /// hesaplanır). 0 = tamamen keskin köşe (karakter hücreleri gibi);
    /// glyph/görsel instance'larında (`has_glyph=1`) kullanılmaz.
    pub corner_radius: f32,
}

const SHADER_SRC: &str = r#"
struct Globals {
    screen_size: vec2<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;
@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct Instance {
    @location(0) pos: vec2<f32>,
    @location(1) size: vec2<f32>,
    @location(2) bg_color: vec4<f32>,
    @location(3) fg_color: vec4<f32>,
    @location(4) atlas_uv_min: vec2<f32>,
    @location(5) atlas_uv_max: vec2<f32>,
    @location(6) has_glyph: f32,
    @location(7) color_glyph: f32,
    @location(8) corner_radius: f32,
};

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) bg_color: vec4<f32>,
    @location(1) fg_color: vec4<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) has_glyph: f32,
    @location(4) color_glyph: f32,
    // Enstans içi yerel pozisyon (0..1, dikdörtgenin sol-üstünden
    // itibaren) + piksel boyutu + köşe yarıçapı: yuvarlatılmış köşe
    // SDF'i fragment shader'da bunlardan hesaplanır.
    @location(5) local01: vec2<f32>,
    @location(6) size_px: vec2<f32>,
    @location(7) corner_radius: f32,
};

// Birim quad (0..1) * instance.size + instance.pos, NDC'ye çevrilir.
@vertex
fn vs_main(
    @builtin(vertex_index) vidx: u32,
    inst: Instance,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let corner = corners[vidx];
    let px = inst.pos + corner * inst.size;
    let ndc = vec2<f32>(
        (px.x / globals.screen_size.x) * 2.0 - 1.0,
        1.0 - (px.y / globals.screen_size.y) * 2.0,
    );
    var out: VsOut;
    out.clip_pos = vec4<f32>(ndc, 0.0, 1.0);
    out.bg_color = inst.bg_color;
    out.fg_color = inst.fg_color;
    out.uv = mix(inst.atlas_uv_min, inst.atlas_uv_max, corner);
    out.has_glyph = inst.has_glyph;
    out.color_glyph = inst.color_glyph;
    out.local01 = corner;
    out.size_px = inst.size;
    out.corner_radius = inst.corner_radius;
    return out;
}

// Standart bir yuvarlatılmış-dikdörtgen SDF'i (Inigo Quilez'in bilinen
// formülü): d<0 içeride, d>0 dışarıda; 1 piksellik yumuşak kenar.
fn rounded_rect_alpha(local01: vec2<f32>, size_px: vec2<f32>, radius_px: f32) -> f32 {
    if (radius_px <= 0.0) {
        return 1.0;
    }
    let half = size_px * 0.5;
    let r = min(radius_px, min(half.x, half.y));
    let p = (local01 - vec2<f32>(0.5, 0.5)) * size_px;
    let q = abs(p) - (half - vec2<f32>(r, r));
    let d = length(max(q, vec2<f32>(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
    return clamp(0.5 - d, 0.0, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.has_glyph < 0.5) {
        var color = in.bg_color;
        color.a = color.a * rounded_rect_alpha(in.local01, in.size_px, in.corner_radius);
        return color;
    }
    let sample = textureSample(atlas_tex, atlas_sampler, in.uv);
    if (in.color_glyph > 0.5) {
        return sample;
    }
    // Mask glyph: R kanalı = coverage (grayscale AA), fg_color ile çarp.
    return vec4<f32>(in.fg_color.rgb, in.fg_color.a * sample.r);
}
"#;

pub struct CellPipeline {
    pipeline: wgpu::RenderPipeline,
    globals_buf: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    pub atlas_bind_group_layout: wgpu::BindGroupLayout,
    instance_buf: wgpu::Buffer,
    instance_capacity: usize,
}

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
struct Globals {
    screen_size: [f32; 2],
    _pad: [f32; 2],
}

impl CellPipeline {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cell-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SRC.into()),
        });

        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bind-group"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buf.as_entire_binding(),
            }],
        });

        let atlas_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("atlas-layout"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cell-pipeline-layout"),
            bind_group_layouts: &[&globals_layout, &atlas_bind_group_layout],
            push_constant_ranges: &[],
        });

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CellInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x2, // pos
                1 => Float32x2, // size
                2 => Float32x4, // bg_color
                3 => Float32x4, // fg_color
                4 => Float32x2, // atlas_uv_min
                5 => Float32x2, // atlas_uv_max
                6 => Float32,   // has_glyph
                7 => Float32,   // color_glyph
                8 => Float32,   // corner_radius
            ],
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cell-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[instance_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let instance_capacity = 8192;
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instance-buf"),
            size: (instance_capacity * std::mem::size_of::<CellInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            globals_buf,
            globals_bind_group,
            atlas_bind_group_layout,
            instance_buf,
            instance_capacity,
        }
    }

    pub fn create_atlas_bind_group(
        &self,
        device: &wgpu::Device,
        view: &wgpu::TextureView,
        sampler: &wgpu::Sampler,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas-bind-group"),
            layout: &self.atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        })
    }

    pub fn upload_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &[CellInstance],
    ) {
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len().next_power_of_two();
            self.instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("instance-buf"),
                size: (self.instance_capacity * std::mem::size_of::<CellInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !instances.is_empty() {
            queue.write_buffer(&self.instance_buf, 0, bytemuck::cast_slice(instances));
        }
    }

    pub fn update_screen_size(&self, queue: &wgpu::Queue, width: f32, height: f32) {
        let globals = Globals {
            screen_size: [width, height],
            _pad: [0.0, 0.0],
        };
        queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
    }

    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        atlas_bind_group: &'a wgpu::BindGroup,
        instance_count: u32,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.globals_bind_group, &[]);
        pass.set_bind_group(1, atlas_bind_group, &[]);
        pass.set_vertex_buffer(0, self.instance_buf.slice(..));
        pass.draw(0..6, 0..instance_count);
    }
}
