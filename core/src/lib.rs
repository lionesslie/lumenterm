//! lumenterm-core
//!
//! PTY yönetimi + `alacritty_terminal` üzerinden VTE/grid durumu.
//! Bu crate render veya pencere katmanına dokunmaz: yalnızca
//! "terminal durumu" (grid, scrollback, cursor, seçim) ve PTY I/O sağlar.
//!
//! Faz 1 kapsamı: çalışan bir PTY + grid güncellemesi.
//! Faz 7 (kimlik özelliği) için: OSC 133 prompt marking -> `BlockTracker`.

mod blocks;
mod event;
mod graphics;
mod pty_io;
mod sixel;

pub use blocks::{Block, BlockTracker};
pub use event::{CoreEvent, EventProxy};
pub use graphics::{DecodedImage, GraphicsStore, ImagePlacement};

use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config as TermConfig, Term};
use alacritty_terminal::Grid;
use crossbeam_channel::{Receiver, Sender};
use parking_lot::Mutex;
use portable_pty::{CommandBuilder, NativePtySystem, PtyPair, PtySize, PtySystem};
use std::io::{Read, Write};
use std::sync::Arc;
use std::thread;

/// Terminal hücre boyutları (kolon/satır) + piksel boyutu (font metrikleri
/// hazır olduğunda `renderer`/`font` tarafından doldurulur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermDimensions {
    pub columns: usize,
    pub lines: usize,
}

/// Platforma göre varsayılan shell. `portable-pty` PTY/ConPTY farkını
/// zaten soyutluyor; burada yalnızca "hangi süreci başlatalım" kararı
/// veriliyor.
#[cfg(target_os = "windows")]
fn default_shell() -> String {
    // Modern Windows'ta PowerShell daha iyi bir varsayılan deneyim verir
    // (renk/unicode desteği `cmd.exe`'den daha sağlam); PATH'te yoksa
    // ConPTY'nin her zaman bulunan `cmd.exe`'sine düşülür.
    if which_on_path("pwsh.exe") {
        "pwsh.exe".to_string()
    } else if which_on_path("powershell.exe") {
        "powershell.exe".to_string()
    } else {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
}

#[cfg(target_os = "windows")]
fn which_on_path(exe: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(exe).is_file())
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn default_shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.columns
    }
}

/// Tek bir terminal oturumu: PTY + alacritty_terminal::Term durumu.
///
/// `Term` kilitli (`Mutex`) olarak tutulur çünkü PTY okuma işi ayrı bir
/// arka plan thread'inde çalışır (bkz. `pty_io::spawn_reader`); render
/// döngüsü her frame'de kısa süreliğine kilidi alıp "dirty" hücreleri okur.
pub struct Session {
    pub term: Arc<Mutex<Term<EventProxy>>>,
    pub blocks: Arc<Mutex<BlockTracker>>,
    pub graphics: Arc<Mutex<GraphicsStore>>,
    pty_writer: Box<dyn Write + Send>,
    _pty_pair: PtyPair,
    pub events_rx: Receiver<CoreEvent>,
}

impl Session {
    /// Yeni bir shell başlatır: PTY açar, kullanıcı shell'ini `$SHELL`
    /// içinden (yoksa `/bin/bash`) çalıştırır ve arka planda bir okuma
    /// thread'i kurar.
    pub fn spawn(dims: TermDimensions, cell_px: (u16, u16)) -> anyhow::Result<Self> {
        let pty_system = NativePtySystem::default();
        let pty_pair = pty_system.openpty(PtySize {
            rows: dims.lines as u16,
            cols: dims.columns as u16,
            pixel_width: cell_px.0 * dims.columns as u16,
            pixel_height: cell_px.1 * dims.lines as u16,
        })?;

        let mut cmd = CommandBuilder::new(default_shell());
        cmd.env("TERM", "xterm-256color");
        // OSC 133 shell entegrasyonu: kullanıcının shell rc dosyasına
        // eklenecek küçük bir script ile prompt start/end işaretlenir.
        // Burada sadece ortam değişkeni bırakıyoruz; gerçek entegrasyon
        // `assets/shell-integration/*.sh` dosyalarından source edilir.
        cmd.env("LUMENTERM_SHELL_INTEGRATION", "1");
        let _child = pty_pair.slave.spawn_command(cmd)?;

        let pty_writer = pty_pair.master.take_writer()?;
        let mut pty_reader = pty_pair.master.try_clone_reader()?;

        let (events_tx, events_rx): (Sender<CoreEvent>, Receiver<CoreEvent>) =
            crossbeam_channel::unbounded();

        let proxy = EventProxy::new(events_tx.clone());
        let term_config = TermConfig::default();
        let term = Term::new(term_config, &dims, proxy);
        let term = Arc::new(Mutex::new(term));
        let blocks = Arc::new(Mutex::new(BlockTracker::default()));
        let graphics = Arc::new(Mutex::new(GraphicsStore::default()));
        let cell_px_f = (cell_px.0 as f32, cell_px.1 as f32);

        pty_io::spawn_reader(
            term.clone(),
            blocks.clone(),
            graphics.clone(),
            cell_px_f,
            events_tx,
            move |buf| pty_reader.read(buf),
        );

        Ok(Self {
            term,
            blocks,
            graphics,
            pty_writer,
            _pty_pair: pty_pair,
            events_rx,
        })
    }

    /// Kullanıcı girdisini (klavye -> escape sequence çevrimi `app` katmanında
    /// yapılır) PTY'ye yazar.
    pub fn write_input(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.pty_writer.write_all(bytes)?;
        self.pty_writer.flush()
    }

    pub fn resize(&self, dims: TermDimensions, cell_px: (u16, u16)) {
        let mut term = self.term.lock();
        term.resize(dims);
        drop(term);
        let _ = self._pty_pair.master.resize(PtySize {
            rows: dims.lines as u16,
            cols: dims.columns as u16,
            pixel_width: cell_px.0 * dims.columns as u16,
            pixel_height: cell_px.1 * dims.lines as u16,
        });
    }
}
