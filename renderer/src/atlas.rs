//! Glyph atlas: `lumenterm_font::RasterizedGlyph` çıktısını tek bir büyük
//! RGBA8 `wgpu::Texture`'a paketleyen basit bir "shelf" (satır bazlı)
//! packer. Hem grayscale AA maskeleri (R kanalına yazılıp shader'da
//! `fg_color` ile çarpılır) hem de renkli emoji glyph'leri (doğrudan RGBA)
//! aynı dokuda tutulur; `AtlasEntry::color_glyph` hangisi olduğunu söyler.
//!
//! Basitlik için atlas dolunca taşma durumu şimdilik panic yerine
//! sessizce "pack edilemedi" (`None`) döner -- `app` o glyph'i o frame
//! çizmeden geçer. Gerçek bir LRU tahliye stratejisi (nadir kullanılan
//! glyph'leri atıp yer açma) ilerideki bir fazda eklenebilir; pratikte
//! 2048x2048 bir atlas, tipik bir terminal oturumunun kullandığı
//! ASCII+yaygın-unicode glyph setinin çok üzerindedir.

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
    /// Hücre içindeki piksel ofseti (baseline'a göre sol/üst), glyph'in
    /// quad'ını doğru konuma yerleştirmek için `app` tarafında kullanılır.
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
    cache: HashMap<GlyphKey, AtlasEntry>,
    // Shelf packer durumu.
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
}

impl GlyphAtlas {
    pub fn new(device: &wgpu::Device) -> Self {
        let size = (2048u32, 2048u32);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("glyph-atlas"),
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
            label: Some("glyph-atlas-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
            size,
            cache: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            shelf_height: 0,
        }
    }

    pub fn get(&self, key: &GlyphKey) -> Option<AtlasEntry> {
        self.cache.get(key).copied()
    }

    /// Glyph atlas'ta yoksa paketler, varsa cache'ten döner.
    pub fn get_or_insert(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        key: GlyphKey,
        glyph: &RasterizedGlyph,
    ) -> Option<AtlasEntry> {
        if let Some(entry) = self.cache.get(&key) {
            return Some(*entry);
        }

        let (w, h) = (glyph.width, glyph.height);
        if w == 0 || h == 0 {
            return None;
        }

        // Satıra sığmıyorsa yeni satıra geç.
        if self.cursor_x + w + 1 > self.size.0 {
            self.cursor_x = 1;
            self.cursor_y += self.shelf_height + 1;
            self.shelf_height = 0;
        }
        if self.cursor_y + h + 1 > self.size.1 {
            log::warn!("Glyph atlas dolu, '{}' karakteri bu frame çizilemeyecek", key.ch);
            return None;
        }

        let rgba = self.to_rgba(glyph);
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: self.cursor_x,
                    y: self.cursor_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(w * 4),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        let entry = AtlasEntry {
            uv_min: [
                self.cursor_x as f32 / self.size.0 as f32,
                self.cursor_y as f32 / self.size.1 as f32,
            ],
            uv_max: [
                (self.cursor_x + w) as f32 / self.size.0 as f32,
                (self.cursor_y + h) as f32 / self.size.1 as f32,
            ],
            left: glyph.left,
            top: glyph.top,
            width: w,
            height: h,
            color_glyph: glyph.is_color,
        };

        self.cursor_x += w + 1;
        self.shelf_height = self.shelf_height.max(h);
        self.cache.insert(key, entry);
        let _ = device; // ileride mip/format değişiminde kullanılabilir
        Some(entry)
    }

    /// `swash`/cosmic-text çıktısını her zaman RGBA8'e normalize eder:
    /// mask (tek kanal) glyph'ler R=coverage, G=B=0, A=coverage olarak
    /// yazılır (shader `has_glyph && !color_glyph` durumunda yalnızca R'yi
    /// alfa olarak kullanıp `fg_color` ile çarpar); renkli glyph'ler
    /// olduğu gibi kopyalanır.
    fn to_rgba(&self, glyph: &RasterizedGlyph) -> Vec<u8> {
        if glyph.is_color {
            debug_assert_eq!(glyph.data.len(), (glyph.width * glyph.height * 4) as usize);
            glyph.data.clone()
        } else {
            let mut out = Vec::with_capacity((glyph.width * glyph.height * 4) as usize);
            for &a in &glyph.data {
                out.extend_from_slice(&[a, a, a, a]);
            }
            out
        }
    }
}
