//! Glyph **ve** görsel (Kitty/Sixel) atlas'ı: ikisi de aynı büyük RGBA8
//! `wgpu::Texture`'a, aynı basit "shelf" (satır bazlı) packer ile
//! paketlenir. Glyph'ler karaktere, görseller `image_id`'ye göre
//! cache'lenir -- aynı karakter/görsel ikinci kez asla yeniden
//! yüklenmez.
//!
//! Atlas boyutu (4096x4096), hem binlerce ASCII+yaygın-unicode glyph hem
//! de birkaç orta boy terminal görseli için yeterlidir. Atlas dolunca ya
//! da tek bir görsel atlas'tan büyükse (örn. çok yüksek çözünürlüklü bir
//! ekran görüntüsü), o öğe o frame'de sessizce atlanır ve bir uyarı
//! loglanır -- gerçek bir LRU tahliye/çoklu-atlas stratejisi ileride
//! eklenebilir, pratikte tipik kullanım için yeterli.

use lumenterm_font::RasterizedGlyph;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub ch: char,
    pub bold: bool,
    pub italic: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct AtlasEntry {
    pub uv_min: [f32; 2],
    pub uv_max: [f32; 2],
    /// Hücre içindeki piksel ofseti (baseline'a göre sol/üst); yalnızca
    /// glyph'ler için anlamlı, görseller için 0.
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
    pub color_glyph: bool,
}

pub struct GlyphAtlas {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    size: (u32, u32),
    glyph_cache: HashMap<GlyphKey, AtlasEntry>,
    image_cache: HashMap<u32, AtlasEntry>,
    // Shelf packer durumu (glyph ve görseller aynı shelf alanını paylaşır).
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let size = (4096u32, 4096u32);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph-image-atlas"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
            size,
            glyph_cache: HashMap::new(),
            image_cache: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            shelf_height: 0,
        }
    }

    pub fn get(&self, key: &GlyphKey) -> Option<AtlasEntry> {
        self.glyph_cache.get(key).copied()
    }

    pub fn get_image(&self, image_id: u32) -> Option<AtlasEntry> {
        self.image_cache.get(&image_id).copied()
    }

    /// Glyph atlas'ta yoksa paketler, varsa cache'ten döner.
    pub fn get_or_insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: GlyphKey,
        glyph: &RasterizedGlyph,
    ) -> Option<AtlasEntry> {
        if let Some(entry) = self.glyph_cache.get(&key) {
            return Some(*entry);
        }
        let rgba = to_rgba_mask_or_color(glyph);
        let origin = self.alloc_and_upload(device, queue, glyph.width, glyph.height, &rgba, &key.ch.to_string())?;
        let entry = AtlasEntry {
            uv_min: [origin.0 as f32 / self.size.0 as f32, origin.1 as f32 / self.size.1 as f32],
            uv_max: [
                (origin.0 + glyph.width) as f32 / self.size.0 as f32,
                (origin.1 + glyph.height) as f32 / self.size.1 as f32,
            ],
            left: glyph.left,
            top: glyph.top,
            width: glyph.width,
            height: glyph.height,
            color_glyph: glyph.is_color,
        };
        self.glyph_cache.insert(key, entry);
        Some(entry)
    }

    /// Bir Kitty/Sixel görselini (zaten RGBA8 olarak çözülmüş) atlas'a
    /// paketler. Aynı `image_id` ikinci kez çağrılırsa cache'ten döner --
    /// Kitty protokolünde bir görsel bir kez aktarılıp birden çok kez
    /// yerleştirilebildiğinden bu önemlidir.
    pub fn insert_image(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image_id: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Option<AtlasEntry> {
        if let Some(entry) = self.image_cache.get(&image_id) {
            return Some(*entry);
        }
        let origin = self.alloc_and_upload(device, queue, width, height, rgba, &format!("image#{image_id}"))?;
        let entry = AtlasEntry {
            uv_min: [origin.0 as f32 / self.size.0 as f32, origin.1 as f32 / self.size.1 as f32],
            uv_max: [(origin.0 + width) as f32 / self.size.0 as f32, (origin.1 + height) as f32 / self.size.1 as f32],
            left: 0,
            top: 0,
            width,
            height,
            color_glyph: true,
        };
        self.image_cache.insert(image_id, entry);
        Some(entry)
    }

    /// Ortak shelf-pack + GPU'ya yükleme mantığı. `rgba.len()` tam olarak
    /// `w*h*4` olmalıdır.
    fn alloc_and_upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        w: u32,
        h: u32,
        rgba: &[u8],
        debug_label: &str,
    ) -> Option<(u32, u32)> {
        if w == 0 || h == 0 || rgba.len() as u64 != (w as u64) * (h as u64) * 4 {
            return None;
        }
        if w + 2 > self.size.0 || h + 2 > self.size.1 {
            log::warn!("'{debug_label}' atlas'a sığmayacak kadar büyük ({w}x{h}), atlanıyor");
            return None;
        }

        if self.cursor_x + w + 1 > self.size.0 {
            self.cursor_x = 1;
            self.cursor_y += self.shelf_height + 1;
            self.shelf_height = 0;
        }
        if self.cursor_y + h + 1 > self.size.1 {
            log::warn!("Atlas dolu, '{debug_label}' bu frame paketlenemeyecek");
            return None;
        }

        let origin = (self.cursor_x, self.cursor_y);
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: origin.0, y: origin.1, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );

        self.cursor_x += w + 1;
        self.shelf_height = self.shelf_height.max(h);
        let _ = device; // ileride mip/format değişiminde kullanılabilir
        Some(origin)
    }
}

/// `swash`/cosmic-text çıktısını her zaman RGBA8'e normalize eder: mask
/// (tek kanal) glyph'ler R=G=B=A=coverage olarak yazılır (shader
/// `has_glyph && !color_glyph` durumunda yalnızca R'yi alfa olarak
/// kullanıp `fg_color` ile çarpar); renkli glyph'ler olduğu gibi
/// kopyalanır.
fn to_rgba_mask_or_color(glyph: &RasterizedGlyph) -> Vec<u8> {
    if glyph.is_color {
        glyph.data.clone()
    } else {
        let mut out = Vec::with_capacity((glyph.width * glyph.height * 4) as usize);
        for &a in &glyph.data {
            out.extend_from_slice(&[a, a, a, a]);
        }
        out
    }
}
