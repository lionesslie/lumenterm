//! "Katmanlı scrollback" kimlik özelliği.
//!
//! Varsayılan davranış klasik, kesintisiz bir scrollback'tir (hiçbir blok
//! zorunluluğu yoktur, hiçbir şey katlanmamışken ekranda normal bir
//! terminal akışından farksızdır). Bu modül arka planda, görünmez biçimde
//! OSC 133 (`\x1b]133;A\x07` prompt-start, `;B` input-start, `;C`
//! command-start, `;D;<exit>` command-end) sekanslarını izleyip her
//! komutun **mutlak grid satır aralığını** kaydeder (Kitty/Sixel görsel
//! yerleşimleriyle aynı koordinat uzayı: `core::graphics::ImagePlacement`
//! ile tutarlı). Kullanıcı bir kısayolla (varsayılan `Ctrl+Shift+Enter`,
//! bkz. `lumenterm_config::BlocksConfig::fold_shortcut`) imlecin/farenin
//! üzerinde durduğu bloğu "katlar" -- `render` tarafı (`app`) bu durumda
//! bloğun komut+çıktı satırlarını tek bir özet satıra indirger.

#[derive(Debug, Clone)]
pub struct Block {
    /// Prompt'un başladığı mutlak grid satırı (her zaman görünür kalır,
    /// katlansa bile -- kullanıcı en azından "burada bir komut vardı"yı
    /// görsün diye).
    pub prompt_start: i32,
    /// Kullanıcının komutu yazmaya başladığı satır.
    pub command_start: Option<i32>,
    /// Komut çalışmaya başladığı (çıktının başladığı) satır.
    pub output_start: Option<i32>,
    /// Komutun bittiği (bir sonraki prompt'tan hemen önceki) satır.
    pub output_end: Option<i32>,
    pub exit_code: Option<i32>,
    /// Kullanıcı bu bloğu katlamayı (fold) seçti mi? Varsayılan: false,
    /// yani klasik kesintisiz scrollback görünümü korunur.
    pub folded: bool,
}

impl Block {
    /// Katlandığında *gizlenecek* satır aralığı (dahil-dahil), yani
    /// prompt satırının kendisi hariç -- prompt her zaman görünür kalır,
    /// yalnızca komut girişi + çıktı tek bir özet satırına iner. Henüz
    /// `output_end` bilinmiyorsa (komut hâlâ çalışıyor) katlanamaz.
    pub fn hidden_range(&self) -> Option<(i32, i32)> {
        if !self.folded {
            return None;
        }
        let end = self.output_end?;
        let start = self.prompt_start + 1;
        if end >= start {
            Some((start, end))
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct BlockTracker {
    pub blocks: Vec<Block>,
    osc_buf: Vec<u8>,
    in_osc: bool,
    after_esc: bool,
}

impl BlockTracker {
    /// Ham PTY çıktısını tarar; yalnızca OSC 133 sekanslarıyla ilgilenir
    /// (gerisi zaten `vte::Processor` tarafından ayrıca işleniyor).
    /// `cursor_line`: bu chunk işlendikten sonraki imleç satırı (mutlak
    /// grid uzayında) -- yeni/güncellenen blok sınırları buna göre
    /// damgalanır (bkz. `core::graphics` ile aynı yaklaşım).
    pub fn scan(&mut self, bytes: &[u8], cursor_line: i32) {
        for &b in bytes {
            if self.in_osc {
                if b == 0x07 || b == 0x5c {
                    let payload = std::mem::take(&mut self.osc_buf);
                    self.handle_osc(&payload, cursor_line);
                    self.in_osc = false;
                } else {
                    self.osc_buf.push(b);
                }
                continue;
            }
            if self.after_esc {
                self.after_esc = false;
                if b == b']' {
                    self.in_osc = true;
                    self.osc_buf.clear();
                }
                continue;
            }
            if b == 0x1b {
                self.after_esc = true;
            }
        }
    }

    fn handle_osc(&mut self, buf: &[u8], cursor_line: i32) {
        let s = String::from_utf8_lossy(buf);
        let Some(rest) = s.strip_prefix("133;") else { return };
        let mut parts = rest.split(';');
        match parts.next() {
            Some("A") => {
                self.blocks.push(Block {
                    prompt_start: cursor_line,
                    command_start: None,
                    output_start: None,
                    output_end: None,
                    exit_code: None,
                    folded: false,
                });
                const MAX_BLOCKS: usize = 500;
                if self.blocks.len() > MAX_BLOCKS {
                    self.blocks.remove(0);
                }
            }
            Some("B") => {
                if let Some(last) = self.blocks.last_mut() {
                    last.command_start = Some(cursor_line);
                }
            }
            Some("C") => {
                if let Some(last) = self.blocks.last_mut() {
                    last.output_start = Some(cursor_line);
                }
            }
            Some("D") => {
                let exit: Option<i32> = parts.next().and_then(|v| v.parse().ok());
                if let Some(last) = self.blocks.last_mut() {
                    last.output_end = Some(cursor_line);
                    last.exit_code = exit;
                }
            }
            _ => {}
        }
    }

    /// Verilen mutlak grid satırını içeren bloğun index'ini döner (prompt
    /// satırından, o bloğun `output_end`'ine kadar -- katlı olsun ya da
    /// olmasın, "bu satır hangi bloğa ait" sorusu için).
    pub fn block_at_line(&self, line: i32) -> Option<usize> {
        self.blocks.iter().rposition(|b| {
            let end = b.output_end.unwrap_or(i32::MAX);
            line >= b.prompt_start && line <= end
        })
    }

    pub fn toggle_fold(&mut self, index: usize) {
        if let Some(b) = self.blocks.get_mut(index) {
            // Henüz tamamlanmamış (hâlâ çalışan) bir komut katlanamaz.
            if b.output_end.is_some() {
                b.folded = !b.folded;
            }
        }
    }
}
