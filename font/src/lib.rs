//! lumenterm-font
//!
//! `cosmic-text` ile shaping (ligature, kompleks script, bidi) ve dahili
//! `SwashCache` ile rasterizasyon. Bu crate GPU'ya dokunmaz: yalnızca
//! glyph bitmap'leri (alfa maskesi ya da renkli emoji RGBA'sı) ve
//! metrikleri üretir; `renderer::atlas::GlyphAtlas` bunları GPU dokusuna
//! paketler.
//!
//! Monospace bir terminalde her hücre sabit genişlikte olduğundan, tam
//! paragraf shaping'i yerine karakter-başına rasterizasyon yapıyoruz
//! (performans için yeterli: glyph'ler atlas'ta cache'lenir, aynı
//! karakter ikinci kez asla yeniden rasterize edilmez). Ligature'lar
//! ileride çok-karakterli "cluster" rasterizasyonu ile eklenebilir.

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics as CtMetrics, Shaping, SwashCache, Weight,
};

pub struct FontManager {
    pub font_system: FontSystem,
    pub swash_cache: SwashCache,
    pub metrics: CtMetrics,
    primary_family: String,
    /// Birincil fontta bulunamayan karakterler için sırayla denenecek
    /// fallback aileleri (emoji, CJK vb.).
    pub fallback_chain: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    pub descent: f32,
}

/// Rasterize edilmiş tek bir glyph. `is_color` true ise `data` RGBA8
/// (emoji vb.), false ise tek kanallı (alfa/coverage) grayscale AA
/// maskesidir -- GPU tarafında ikisi de aynı atlas dokusuna (RGBA8)
/// paketlenir, mask glyph'ler paketlenirken R kanalına yazılıp shader'da
/// `fg_color` ile çarpılır.
pub struct RasterizedGlyph {
    pub width: u32,
    pub height: u32,
    /// Baseline'a göre sol üst köşe ofseti (piksel, swash `placement`).
    pub left: i32,
    pub top: i32,
    pub data: Vec<u8>,
    pub is_color: bool,
}

impl FontManager {
    pub fn new(primary_family: &str, size_px: f32, fallback_chain: Vec<String>) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let metrics = CtMetrics::new(size_px, size_px * 1.2);
        Self {
            font_system,
            swash_cache,
            metrics,
            primary_family: primary_family.to_string(),
            fallback_chain,
        }
    }

    /// Monospace hücre genişliğini birincil fontun ortalama ilerleme
    /// değerinden hesaplar. Gerçek bir monospace fontta tüm glyph'ler aynı
    /// `advance`'e sahip olduğundan 'M' yeterli bir örnektir.
    pub fn cell_metrics(&mut self) -> CellMetrics {
        let advance = self.measure_advance('M').unwrap_or(self.metrics.font_size * 0.6);
        CellMetrics {
            width: advance,
            height: self.metrics.line_height,
            descent: self.metrics.line_height * 0.2,
        }
    }

    fn measure_advance(&mut self, ch: char) -> Option<f32> {
        let attrs = Attrs::new().family(Family::Name(&self.primary_family));
        let mut buffer = Buffer::new(&mut self.font_system, self.metrics);
        buffer.set_text(&mut self.font_system, &ch.to_string(), attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer.layout_runs().next().and_then(|run| {
            run.glyphs.first().map(|g| g.w)
        })
    }

    /// Bir karakteri (opsiyonel bold/italic ile) rasterize eder. Fallback
    /// zinciri: birincil ailede glyph bulunamazsa sırayla
    /// `fallback_chain`'deki ailelere düşülür; o da yoksa cosmic-text'in
    /// kendi sistem fallback'ine (fontdb) bırakılır (attrs.family
    /// belirtilmeden tekrar denenir).
    pub fn rasterize_char(&mut self, ch: char, bold: bool, italic: bool) -> Option<RasterizedGlyph> {
        let mut families = vec![self.primary_family.clone()];
        families.extend(self.fallback_chain.iter().cloned());

        for family in &families {
            if let Some(g) = self.rasterize_with_family(ch, family, bold, italic) {
                return Some(g);
            }
        }
        // Son çare: aile belirtmeden sistem fallback'i.
        self.rasterize_with_family(ch, "", bold, italic)
    }

    fn rasterize_with_family(
        &mut self,
        ch: char,
        family: &str,
        bold: bool,
        italic: bool,
    ) -> Option<RasterizedGlyph> {
        let mut attrs = Attrs::new();
        if !family.is_empty() {
            attrs = attrs.family(Family::Name(family));
        }
        if bold {
            attrs = attrs.weight(Weight::BOLD);
        }
        if italic {
            attrs = attrs.style(cosmic_text::Style::Italic);
        }

        let mut buffer = Buffer::new(&mut self.font_system, self.metrics);
        buffer.set_text(&mut self.font_system, &ch.to_string(), attrs, Shaping::Advanced);
        buffer.shape_until_scroll(&mut self.font_system, false);

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                // Boşluk/kontrol karakterleri için outline yoktur; atlanır.
                if ch.is_whitespace() {
                    return None;
                }
                let physical = glyph.physical((0.0, 0.0), 1.0);
                let image = self
                    .swash_cache
                    .get_image(&mut self.font_system, physical.cache_key)
                    .as_ref()?;

                if image.placement.width == 0 || image.placement.height == 0 {
                    return None;
                }

                let is_color = matches!(image.content, cosmic_text::SwashContent::Color);
                return Some(RasterizedGlyph {
                    width: image.placement.width,
                    height: image.placement.height,
                    left: image.placement.left,
                    top: image.placement.top,
                    data: image.data.clone(),
                    is_color,
                });
            }
        }
        None
    }
}
