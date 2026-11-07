//! Ctrl+Shift+P ile açılan komut paleti.
//!
//! Basit, bağımlılıksız bir alt-dizi (subsequence) bulanık eşleştirici
//! kullanır. Daha önce burada `nucleo::Nucleo` (binlerce öğelik listeler
//! için tasarlanmış, arka plan worker thread'leriyle **asenkron**
//! eşleştirme yapan bir motor) kullanılıyordu -- ama bizim listemiz
//! sabit ve küçük (~10 komut), ve `injector.push()` ile eklenen öğeler
//! birkaç `tick()` çağrısı geçmeden `snapshot()`'a yansımıyordu. Sonuç:
//! palet açılır açılmaz ya da hızlı yazarken sonuçlar "henüz hazır
//! değil" durumunda kalıyor, seçim/Enter çoğu zaman hiçbir şeye
//! isabet etmiyordu. Senkron bir alt-dizi eşleştirici bu belirsizliği
//! tamamen ortadan kaldırıyor: her tuş vuruşunda sonuç anında ve
//! deterministik.

#[derive(Debug, Clone)]
pub struct CommandPalette {
    commands: Vec<String>,
    pub query: String,
    pub selected: usize,
}

impl CommandPalette {
    pub fn new() -> Self {
        let commands = vec![
            "Yeni sekme".to_string(),
            "Yatay böl".to_string(),
            "Dikey böl".to_string(),
            "Temayı değiştir: lumen-dark".to_string(),
            "Temayı değiştir: lumen-light".to_string(),
            "Temayı değiştir: sunset".to_string(),
            "Temayı değiştir: glacier".to_string(),
            "Komut bloğunu katla/aç (fare altındaki)".to_string(),
        ];
        Self {
            commands,
            query: String::new(),
            selected: 0,
        }
    }

    pub fn reset_query(&mut self) {
        self.query.clear();
        self.selected = 0;
    }

    pub fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
        self.selected = 0;
    }

    /// Geriye dönük uyumluluk için bırakıldı. Eşleştirme artık tamamen
    /// senkron (`results()` içinde) yapıldığından bu metodun bir işlevi
    /// kalmadı; `app` yine de her frame çağırabilir, no-op'tur.
    pub fn tick(&mut self) {}

    /// Güncel sorguyla eşleşen komutları, en iyi skordan en kötüye doğru
    /// sıralı döner. Sorgu boşsa (palet yeni açıldığında olduğu gibi)
    /// tüm komutları listeler -- fuzzy eşleştiricinin boş-sorgu
    /// davranışına güvenmek yerine burada açıkça garanti ediliyor.
    pub fn results(&self, max: usize) -> Vec<String> {
        if self.query.is_empty() {
            return self.commands.iter().take(max).cloned().collect();
        }
        let mut scored: Vec<(&String, i32)> = self
            .commands
            .iter()
            .filter_map(|c| fuzzy_score(c, &self.query).map(|score| (c, score)))
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().take(max).map(|(s, _)| s.clone()).collect()
    }
}

/// Basit bir alt-dizi (subsequence) skorlayıcı: `query`'nin her karakteri
/// sırayla `candidate` içinde (büyük/küçük harf duyarsız) bulunmalı.
/// Ardışık eşleşmelere ve erken konumdaki eşleşmelere küçük bir bonus
/// verir. `query`'nin tüm karakterleri bulunamazsa `None`.
fn fuzzy_score(candidate: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let cand: Vec<char> = candidate.to_lowercase().chars().collect();
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let mut qi = 0;
    let mut score = 0i32;
    let mut last_match: Option<usize> = None;

    for (ci, &c) in cand.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            score += 10;
            if last_match == Some(ci.wrapping_sub(1)) {
                score += 8; // ardışık eşleşme bonusu
            }
            if ci == qi {
                score += 3; // erken konum bonusu
            }
            last_match = Some(ci);
            qi += 1;
        }
    }

    if qi == q.len() {
        Some(score)
    } else {
        None
    }
}
