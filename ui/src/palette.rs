//! Ctrl+Shift+P ile açılan komut paleti + scrollback fuzzy arama.
//! `nucleo` ile bulanık eşleştirme yapılır; sonuçlar skora göre sıralanır.

use nucleo::pattern::{CaseMatching, Normalization};
use nucleo::{Config, Nucleo};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub enum PaletteItemKind {
    Command,
    ScrollbackLine { line: usize },
}

#[derive(Debug, Clone)]
pub struct PaletteItem {
    pub label: String,
    pub kind: PaletteItemKind,
}

pub struct CommandPalette {
    matcher: Nucleo<PaletteItem>,
    pub query: String,
    pub selected: usize,
}

impl CommandPalette {
    pub fn new() -> Self {
        let matcher = Nucleo::new(Config::DEFAULT, Arc::new(|| {}), None, 1);
        let mut palette = Self {
            matcher,
            query: String::new(),
            selected: 0,
        };
        palette.seed_builtin_commands();
        palette
    }

    fn seed_builtin_commands(&mut self) {
        let injector = self.matcher.injector();
        for label in [
            "Yeni sekme",
            "Yatay böl",
            "Dikey böl",
            "Temayı değiştir: lumen-dark",
            "Temayı değiştir: lumen-light",
            "Temayı değiştir: sunset",
            "Temayı değiştir: glacier",
            "Komut bloğunu katla/aç",
            "Scrollback'te ara",
            "Config dosyasını aç",
        ] {
            injector.push(
                PaletteItem {
                    label: label.to_string(),
                    kind: PaletteItemKind::Command,
                },
                |item, cols| cols[0] = item.label.as_str().into(),
            );
        }
    }

    pub fn reset_query(&mut self) {
        self.query.clear();
        self.selected = 0;
        self.matcher.pattern.reparse(
            0,
            "",
            CaseMatching::Smart,
            Normalization::Smart,
            false,
        );
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.matcher.pattern.reparse(
            0,
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            true,
        );
    }

    /// Her frame (ya da her tuş basımında) çağrılır; `nucleo` arka planda
    /// asenkron eşleştirme yapar, burada yalnızca bir adım ilerletilir.
    pub fn tick(&mut self) {
        self.matcher.tick(10);
    }

    pub fn results(&self, max: usize) -> Vec<String> {
        let snapshot = self.matcher.snapshot();
        snapshot
            .matched_items(0..snapshot.matched_item_count().min(max as u32))
            .map(|item| item.data.label.clone())
            .collect()
    }

    /// Scrollback içeriği değiştiğinde (`core::Session` tarafından) çağrılıp
    /// aranabilir satırları besler.
    pub fn index_scrollback_line(&mut self, line_no: usize, text: String) {
        let injector = self.matcher.injector();
        injector.push(
            PaletteItem {
                label: text.clone(),
                kind: PaletteItemKind::ScrollbackLine { line: line_no },
            },
            move |item, cols| cols[0] = item.label.as_str().into(),
        );
    }
}
