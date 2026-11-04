//! lumenterm-ui
//!
//! GPU üzerinde çizilen minimal chrome: sekme çubuğu, bölücü çizgiler,
//! split/pane ağacı, komut paleti overlay ve opsiyonel blok katmanı UI'ı.
//! Bu crate kendi `wgpu` çağrısı yapmaz; `renderer::CellInstance` benzeri
//! instance listeleri üretip `app` katmanına verir (chrome da aynı
//! instanced pipeline ile, sade dikdörtgenler + ileride ikon glyph'leri
//! olarak çizilir).

pub mod palette;
pub mod splits;

pub use palette::CommandPalette;
pub use splits::{PaneId, SplitDirection, SplitTree};

/// Tüm UI durumunu tutan üst nesne; `app` event loop'u girdi olaylarını
/// buraya yönlendirir, her frame `ui_instances()` çağırıp dönen
/// dikdörtgenleri terminal hücrelerinin üstüne (daha yüksek z-sırasıyla)
/// render eder.
pub struct UiState {
    pub splits: SplitTree,
    pub palette: CommandPalette,
    pub palette_open: bool,
    /// Faz 7 kimlik özelliği: "katlanmış" blokların gösterim durumu.
    pub blocks_overlay_visible: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            splits: SplitTree::new(),
            palette: CommandPalette::new(),
            palette_open: false,
            blocks_overlay_visible: false,
        }
    }
}

impl UiState {
    pub fn toggle_palette(&mut self) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            self.palette.reset_query();
        }
    }
}
