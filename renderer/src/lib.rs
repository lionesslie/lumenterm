//! lumenterm-renderer
//!
//! wgpu üzerinden **Vulkan öncelikli** GPU render backend'i. Tüm hücreler
//! (arka plan rengi + glyph) tek bir instanced draw call ile çizilir.
//! Damage tracking: yalnızca "dirty" işaretlenen hücreler için instance
//! buffer güncellenir; hiçbir şey değişmediyse frame tamamen atlanır
//! (boşta CPU/GPU ~0).
//!
//! **Platform/backend politikası**: Linux'ta (projenin birincil hedefi)
//! yalnızca Vulkan denenir — bulunamazsa açık bir hatayla durulur, sessiz
//! bir GL/yazılım fallback'ine asla izin verilmez. Windows'ta da önce
//! Vulkan denenir (modern NVIDIA/AMD/Intel sürücülerinin hemen hepsi
//! Vulkan ICD'si sağlar); yalnızca gerçekten bir Vulkan adaptörü
//! bulunamazsa, DX12'ye **loglanan, açık** bir fallback yapılır. Bu,
//! projenin "Vulkan" kimliğini korurken bozuk/eksik Vulkan sürücüsü olan
//! Windows makinelerinde de uygulamanın açılabilmesini sağlar.

mod atlas;
mod pipeline;

pub use atlas::{AtlasEntry, GlyphAtlas, GlyphKey};
pub use pipeline::CellInstance;

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use std::sync::Arc;

/// Bu platformda sırayla denenecek backend'ler. Linux'ta tek seçenek
/// Vulkan'dır (kasıtlı: proje gereksinimleri gereği buradan taviz
/// verilmez). Windows'ta Vulkan birincil, DX12 yalnızca fallback'tir.
#[cfg(target_os = "windows")]
const CANDIDATE_BACKENDS: &[wgpu::Backends] = &[wgpu::Backends::VULKAN, wgpu::Backends::DX12];
#[cfg(not(target_os = "windows"))]
const CANDIDATE_BACKENDS: &[wgpu::Backends] = &[wgpu::Backends::VULKAN];

pub struct GpuState {
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub pipeline: pipeline::CellPipeline,
    pub atlas: GlyphAtlas,
    atlas_bind_group: wgpu::BindGroup,
    pub size: (u32, u32),
}

impl GpuState {
    pub async fn new<W>(window: Arc<W>, width: u32, height: u32) -> anyhow::Result<Self>
    where
        W: HasWindowHandle + HasDisplayHandle + Send + Sync + 'static,
    {
        // Aday backend'leri sırayla dene: her biri kendi wgpu::Instance +
        // Surface çiftini gerektirir (Surface bir Instance'a bağlıdır),
        // bu yüzden Vulkan adaptörü bulunamazsa o Instance/Surface'ı
        // bırakıp bir sonraki aday için yeniden kuruyoruz.
        let mut chosen: Option<(wgpu::Instance, wgpu::Surface<'static>, wgpu::Adapter)> = None;
        for &backends in CANDIDATE_BACKENDS {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends,
                flags: wgpu::InstanceFlags::default(),
                dx12_shader_compiler: Default::default(),
                gles_minor_version: Default::default(),
            });
            let Ok(surface) = instance.create_surface(window.clone()) else {
                continue;
            };
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await;
            if let Some(adapter) = adapter {
                if backends != wgpu::Backends::VULKAN {
                    log::warn!(
                        "Vulkan adaptörü bulunamadı; {:?} backend'ine düşüldü. \
                         En iyi deneyim için güncel GPU sürücülerinizi (Vulkan ICD) kurmanız önerilir.",
                        backends
                    );
                }
                chosen = Some((instance, surface, adapter));
                break;
            }
        }

        let (instance, surface, adapter) = chosen.ok_or_else(|| {
            if cfg!(target_os = "windows") {
                anyhow::anyhow!(
                    "Ne Vulkan ne de DX12 üzerinden bir GPU adaptörü bulunamadı. \
                     GPU sürücülerinizin güncel olduğundan emin olun."
                )
            } else {
                anyhow::anyhow!(
                    "Uygun bir Vulkan adaptörü bulunamadı. Vulkan sürücülerinizin \
                     (mesa-vulkan-drivers / nvidia-vulkan vb.) kurulu olduğundan emin olun."
                )
            }
        })?;

        log::info!(
            "GPU adaptörü: {} (backend: {:?})",
            adapter.get_info().name,
            adapter.get_info().backend
        );

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("lumenterm-device"),
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::Performance,
                },
                None,
            )
            .await?;

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: width.max(1),
            height: height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let pipeline = pipeline::CellPipeline::new(&device, format);
        let atlas = GlyphAtlas::new(&device);
        let atlas_bind_group =
            pipeline.create_atlas_bind_group(&device, &atlas.view, &atlas.sampler);

        Ok(Self {
            instance,
            surface,
            device,
            queue,
            config,
            pipeline,
            atlas,
            atlas_bind_group,
            size: (width, height),
        })
    }

    /// `app` katmanının her yeni karakter için çağırdığı kısayol: glyph
    /// atlas'ta yoksa paketler, UV koordinatlarını döndürür.
    pub fn glyph_entry(
        &mut self,
        key: GlyphKey,
        glyph: &lumenterm_font::RasterizedGlyph,
    ) -> Option<AtlasEntry> {
        self.atlas
            .get_or_insert(&self.device, &self.queue, key, glyph)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.device, &self.config);
        self.size = (width, height);
    }

    /// Yalnızca `dirty` true olduğunda çağrılmalı (damage tracking). `app`
    /// event loop'u bunu `RedrawRequested` veya grid değişim event'inde
    /// tetikler; aksi halde hiçbir GPU işi yapılmaz.
    pub fn render(&mut self, instances: &[CellInstance], clear_color: wgpu::Color) -> anyhow::Result<()> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lumenterm-encoder"),
            });

        self.pipeline
            .upload_instances(&self.device, &self.queue, instances);

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lumenterm-cell-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            self.pipeline
                .draw(&mut pass, &self.atlas_bind_group, instances.len() as u32);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}
