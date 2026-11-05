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

use std::time::Instant;

/// Tüm UI durumunu tutan üst nesne; `app` event loop'u girdi olaylarını
/// buraya yönlendirir, her frame `build_frame` çağırıp dönen
/// dikdörtgenleri terminal hücrelerinin üstüne (daha yüksek z-sırasıyla)
/// render eder.
pub struct UiState {
    pub splits: SplitTree,
    pub palette: CommandPalette,
    pub palette_open: bool,
    /// Palet en son ne zaman açıldı -- `app` bunu kısa bir fade-in
    /// animasyonu (bkz. Faz 5 "yumuşak animasyonlar") için kullanır.
    pub palette_opened_at: Option<Instant>,
    /// Faz 5 kimlik özelliğinin (katlanabilir komut blokları) ileride
    /// eklenebilecek ayrı bir "tüm katlı bloklar" kenar çubuğu/overlay'i
    /// için ayrılmış bayrak. Şu an `app`, her bloğu doğrudan kendi
    /// satırında bir "fold chip" olarak çizdiğinden (bkz.
    /// `core::BlockTracker`, `app::compute_rows`), bu alan henüz
    /// kullanılmıyor.
    pub blocks_overlay_visible: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            splits: SplitTree::new(),
            palette: CommandPalette::new(),
            palette_open: false,
            palette_opened_at: None,
            blocks_overlay_visible: false,
        }
    }
}

impl UiState {
    pub fn toggle_palette(&mut self) {
        self.palette_open = !self.palette_open;
        if self.palette_open {
            self.palette.reset_query();
            self.palette_opened_at = Some(Instant::now());
        }
    }
}
