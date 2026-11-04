//! "Katmanlı scrollback" kimlik özelliği.
//!
//! Varsayılan davranış klasik, kesintisiz bir scrollback'tir (hiçbir blok
//! zorunluluğu yoktur). Bu modül yalnızca arka planda, görünmez biçimde
//! OSC 133 (`\x1b]133;A\x07` prompt-start, `;B` input-start, `;C`
//! command-start, `;D;<exit>` command-end) sekanslarını izleyip her
//! komutun satır aralığını kaydeder. Kullanıcı bir kısayolla (örn.
//! Ctrl+Shift+Enter) imlecin üzerinde durduğu komutu "katlamayı" seçerse
//! `ui` katmanı bu bilgiyi kullanarak o aralığı tek satıra indirger.
//!
//! Bu nedenle `BlockTracker` render kararı vermez; sadece sınırları tutar.

use alacritty_terminal::index::Line;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockState {
    AwaitingPrompt,
    InPrompt,
    InInput,
    InOutput,
}

#[derive(Debug, Clone)]
pub struct Block {
    pub prompt_start: Line,
    pub command_start: Option<Line>,
    pub output_start: Option<Line>,
    pub output_end: Option<Line>,
    pub exit_code: Option<i32>,
    /// Kullanıcı bu bloğu katlamayı (fold) seçti mi? Varsayılan: false,
    /// yani klasik kesintisiz scrollback görünümü korunur.
    pub folded: bool,
}

#[derive(Default)]
pub struct BlockTracker {
    pub blocks: Vec<Block>,
    state: Option<BlockState>,
    osc_buf: Vec<u8>,
    in_osc: bool,
}

impl BlockTracker {
    /// Ham PTY çıktısını tarar; yalnızca OSC 133 sekanslarıyla ilgilenir,
    /// gerisini görmezden gelir (asıl parse işi zaten `vte::Processor`'da).
    pub fn scan(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match (self.in_osc, b) {
                (false, 0x1b) => {
                    self.osc_buf.clear();
                    self.osc_buf.push(b);
                }
                (false, b']') if self.osc_buf.last() == Some(&0x1b) => {
                    self.osc_buf.push(b);
                    self.in_osc = true;
                }
                (true, 0x07) | (true, 0x5c) => {
                    self.handle_osc(&self.osc_buf.clone());
                    self.in_osc = false;
                    self.osc_buf.clear();
                }
                (true, other) => self.osc_buf.push(other),
                _ => {}
            }
        }
    }

    fn handle_osc(&mut self, buf: &[u8]) {
        let s = String::from_utf8_lossy(buf);
        // beklenen biçim: "\x1b]133;<A|B|C|D>[;args]"
        let Some(rest) = s.strip_prefix("\u{1b}]133;") else {
            return;
        };
        let mut parts = rest.split(';');
        match parts.next() {
            Some("A") => {
                self.state = Some(BlockState::InPrompt);
                self.blocks.push(Block {
                    prompt_start: Line(0), // gerçek satır numarası `ui` tarafında Term'den okunur
                    command_start: None,
                    output_start: None,
                    output_end: None,
                    exit_code: None,
                    folded: false,
                });
            }
            Some("B") => self.state = Some(BlockState::InInput),
            Some("C") => self.state = Some(BlockState::InOutput),
            Some("D") => {
                let exit: Option<i32> = parts.next().and_then(|v| v.parse().ok());
                if let Some(last) = self.blocks.last_mut() {
                    last.exit_code = exit;
                }
                self.state = Some(BlockState::AwaitingPrompt);
            }
            _ => {}
        }
    }

    /// `ui` katmanının kısayolla katlama/genişletme yapması için.
    pub fn toggle_fold(&mut self, index: usize) {
        if let Some(b) = self.blocks.get_mut(index) {
            b.folded = !b.folded;
        }
    }
}
