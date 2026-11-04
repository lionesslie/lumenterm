//! LumenTerm — Faz 2 giriş noktası.
//!
//! Faz 1'in üzerine: gerçek glyph render'ı (cosmic-text/swash -> GPU
//! atlas), yumuşak cursor animasyonu + blink, fare ile seçim/kopyalama,
//! fare tekerleği ile scrollback kaydırma. Damage tracking Faz 1'deki
//! gibi korunur: `about_to_wait` yalnızca PTY'den gerçekten bir şey
//! geldiğinde ya da cursor animasyonu/blink'i ilerletmek gerektiğinde
//! redraw ister.

mod input;

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use lumenterm_config::Config;
use lumenterm_core::{CoreEvent, Session, TermDimensions};
use lumenterm_font::FontManager;
use lumenterm_renderer::{CellInstance, GlyphKey, GpuState};
use lumenterm_ui::UiState;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

/// Fare pozisyonunu (line, col) hücre koordinatına çevirir.
fn pixel_to_cell(pos: (f64, f64), pad: f32, cell_w: f32, cell_h: f32, dims: TermDimensions) -> (usize, usize) {
    let col = (((pos.0 as f32 - pad) / cell_w).floor().max(0.0)) as usize;
    let line = (((pos.1 as f32 - pad) / cell_h).floor().max(0.0)) as usize;
    (
        line.min(dims.lines.saturating_sub(1)),
        col.min(dims.columns.saturating_sub(1)),
    )
}

/// Fare ile yapılan metin seçimi, görünür ekran (line, col) uzayında.
/// Not: basitlik için kaydırma (scroll) seçimi temizler -- scrollback'te
/// seçim Faz 4+'da ele alınabilir.
#[derive(Debug, Clone, Copy)]
struct Selection {
    anchor: (usize, usize),
    cursor: (usize, usize),
}

impl Selection {
    fn normalized(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.cursor {
            (self.anchor, self.cursor)
        } else {
            (self.cursor, self.anchor)
        }
    }

    fn contains(&self, line: usize, col: usize) -> bool {
        let (start, end) = self.normalized();
        if line < start.0 || line > end.0 {
            return false;
        }
        if start.0 == end.0 {
            return col >= start.1 && col <= end.1;
        }
        if line == start.0 {
            return col >= start.1;
        }
        if line == end.0 {
            return col <= end.1;
        }
        true
    }
}

/// Cursor'un hücreden hücreye yumuşak geçişi için basit ease-out lerp.
struct CursorAnim {
    from: (f32, f32),
    to: (f32, f32),
    start: Instant,
}

impl CursorAnim {
    fn new(pos: (f32, f32)) -> Self {
        Self { from: pos, to: pos, start: Instant::now() }
    }

    fn retarget(&mut self, target: (f32, f32), duration_ms: u64) {
        if (self.to.0 - target.0).abs() > 0.5 || (self.to.1 - target.1).abs() > 0.5 {
            self.from = self.current(duration_ms);
            self.to = target;
            self.start = Instant::now();
        }
    }

    fn current(&self, duration_ms: u64) -> (f32, f32) {
        if duration_ms == 0 {
            return self.to;
        }
        let elapsed_ms = self.start.elapsed().as_secs_f32() * 1000.0;
        let t = (elapsed_ms / duration_ms as f32).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t) * (1.0 - t); // ease-out quad
        (
            self.from.0 + (self.to.0 - self.from.0) * eased,
            self.from.1 + (self.to.1 - self.from.1) * eased,
        )
    }

    fn is_animating(&self, duration_ms: u64) -> bool {
        self.start.elapsed().as_millis() < duration_ms as u128
    }
}

struct App {
    config: Config,
    config_watcher: Option<lumenterm_config::ConfigWatcher>,
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    session: Option<Session>,
    font: FontManager,
    ui: UiState,
    dims: TermDimensions,
    cell_w: f32,
    cell_h: f32,
    descent: f32,
    dirty: bool,
    modifiers: ModifiersState,
    mouse_pos: (f64, f64),
    mouse_down: bool,
    selection: Option<Selection>,
    cursor_anim: CursorAnim,
    blink_on: bool,
    next_blink: Instant,
}

impl App {
    fn new() -> Self {
        let config_path = Config::default_path();
        let config = Config::load(&config_path).unwrap_or_default();
        let config_watcher = lumenterm_config::ConfigWatcher::new(config_path).ok();

        let mut font = FontManager::new(&config.font.family, config.font.size, config.font.fallback.clone());
        let metrics = font.cell_metrics();

        let pad = config.window.padding as f32;
        Self {
            config,
            config_watcher,
            window: None,
            gpu: None,
            session: None,
            font,
            ui: UiState::default(),
            dims: TermDimensions { columns: 100, lines: 32 },
            cell_w: metrics.width,
            cell_h: metrics.height,
            descent: metrics.descent,
            dirty: true,
            modifiers: ModifiersState::empty(),
            mouse_pos: (0.0, 0.0),
            mouse_down: false,
            selection: None,
            cursor_anim: CursorAnim::new((pad, pad)),
            blink_on: true,
            next_blink: Instant::now(),
        }
    }

    fn recompute_dims(&mut self, width: u32, height: u32) {
        let pad = self.config.window.padding as f32;
        let cols = (((width as f32 - 2.0 * pad) / self.cell_w).floor().max(1.0)) as usize;
        let lines = (((height as f32 - 2.0 * pad) / self.cell_h).floor().max(1.0)) as usize;
        self.dims = TermDimensions { columns: cols, lines };
    }

    /// Seçili hücrelerin metnini satır satır çıkarır (kopyalama için).
    fn extract_selection_text(&self, sel: &Selection) -> String {
        let Some(session) = &self.session else { return String::new() };
        let term = session.term.lock();
        let grid = term.grid();
        let display_offset = grid.display_offset() as i32;
        let (start, end) = sel.normalized();

        let mut out = String::new();
        for line in start.0..=end.0 {
            let (col_start, col_end) = if start.0 == end.0 {
                (start.1, end.1)
            } else if line == start.0 {
                (start.1, self.dims.columns.saturating_sub(1))
            } else if line == end.0 {
                (0, end.1)
            } else {
                (0, self.dims.columns.saturating_sub(1))
            };
            let grid_line = line as i32 - display_offset;
            let mut row = String::new();
            for col in col_start..=col_end {
                let point = Point::new(Line(grid_line), Column(col));
                row.push(grid[point].c);
            }
            out.push_str(row.trim_end());
            if line != end.0 {
                out.push('\n');
            }
        }
        out
    }

    /// Grid'i düz renk arka plan dikdörtgenleri + atlas'tan örneklenen
    /// glyph quad'larına çevirir. Cursor hariç (ayrıca eklenir, animasyon
    /// gerektiği için).
    fn build_instances(&mut self) -> (Vec<CellInstance>, Option<(f32, f32)>) {
        let Some(session) = &self.session else { return (Vec::new(), None) };
        let Some(gpu) = &mut self.gpu else { return (Vec::new(), None) };

        let term = session.term.lock();
        let grid = term.grid();
        let display_offset = grid.display_offset() as i32;
        let pad = self.config.window.padding as f32;
        let bg_default = hex_to_rgba(&self.config.theme.background);
        let fg_default = hex_to_rgba(&self.config.theme.foreground);

        let mut instances = Vec::with_capacity(self.dims.columns * self.dims.lines);

        for line in 0..self.dims.lines {
            for col in 0..self.dims.columns {
                let grid_line = line as i32 - display_offset;
                let point = Point::new(Line(grid_line), Column(col));
                let cell = &grid[point];

                let mut cell_bg = cell_color(cell.bg, &self.config, bg_default, true);
                let mut cell_fg = cell_color(cell.fg, &self.config, fg_default, false);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut cell_bg, &mut cell_fg);
                }
                if let Some(sel) = &self.selection {
                    if sel.contains(line, col) {
                        cell_bg = blend_highlight(cell_bg);
                    }
                }

                let cx = pad + col as f32 * self.cell_w;
                let cy = pad + line as f32 * self.cell_h;

                instances.push(CellInstance {
                    pos: [cx, cy],
                    size: [self.cell_w, self.cell_h],
                    bg_color: cell_bg,
                    fg_color: cell_fg,
                    atlas_uv_min: [0.0, 0.0],
                    atlas_uv_max: [0.0, 0.0],
                    has_glyph: 0.0,
                    color_glyph: 0.0,
                });

                let ch = cell.c;
                if ch != ' ' && ch != '\0' && !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    let bold = cell.flags.contains(Flags::BOLD);
                    let italic = cell.flags.contains(Flags::ITALIC);
                    if let Some(raster) = self.font.rasterize_char(ch, bold, italic) {
                        let key = GlyphKey { ch, bold, italic };
                        if let Some(entry) = gpu.glyph_entry(key, &raster) {
                            let baseline_y = cy + (self.cell_h - self.descent);
                            let gx = cx + entry.left as f32;
                            let gy = baseline_y - entry.top as f32;
                            instances.push(CellInstance {
                                pos: [gx, gy],
                                size: [entry.width as f32, entry.height as f32],
                                bg_color: [0.0; 4],
                                fg_color: cell_fg,
                                atlas_uv_min: entry.uv_min,
                                atlas_uv_max: entry.uv_max,
                                has_glyph: 1.0,
                                color_glyph: if entry.color_glyph { 1.0 } else { 0.0 },
                            });
                        }
                    }
                }
            }
        }

        // Cursor yalnızca scrollback'e kaydırılmamışken (display_offset==0)
        // görünür konumdadır.
        let cursor_target = if display_offset == 0 {
            let cp = grid.cursor.point;
            let (cl, cc) = (cp.line.0, cp.column.0);
            if cl >= 0 && (cl as usize) < self.dims.lines && cc < self.dims.columns {
                Some((pad + cc as f32 * self.cell_w, pad + cl as f32 * self.cell_h))
            } else {
                None
            }
        } else {
            None
        };

        (instances, cursor_target)
    }
}

fn hex_to_rgba(hex: &str) -> [f32; 4] {
    let hex = hex.trim_start_matches('#');
    if hex.len() < 6 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0) as f32 / 255.0;
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0) as f32 / 255.0;
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0) as f32 / 255.0;
    [r, g, b, 1.0]
}

/// Bir cursor hücresinin arka plan rengini biraz aydınlatarak seçim
/// vurgusu üretir (basit bir alfa-üstü-blend yerine sabit bir ton).
fn blend_highlight(base: [f32; 4]) -> [f32; 4] {
    [
        (base[0] + 0.35).min(1.0),
        (base[1] + 0.35).min(1.0),
        (base[2] + 0.45).min(1.0),
        base[3],
    ]
}

fn cell_color(
    color: alacritty_terminal::vte::ansi::Color,
    config: &Config,
    default: [f32; 4],
    is_bg: bool,
) -> [f32; 4] {
    use alacritty_terminal::vte::ansi::{Color, NamedColor};
    match color {
        Color::Named(NamedColor::Background) if is_bg => default,
        Color::Named(NamedColor::Foreground) if !is_bg => default,
        Color::Named(named) => config
            .theme
            .palette
            .get(named as usize % 16)
            .map(|h| hex_to_rgba(h))
            .unwrap_or(default),
        Color::Spec(rgb) => [rgb.r as f32 / 255.0, rgb.g as f32 / 255.0, rgb.b as f32 / 255.0, 1.0],
        Color::Indexed(i) => config
            .theme
            .palette
            .get(i as usize % 16)
            .map(|h| hex_to_rgba(h))
            .unwrap_or(default),
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title("LumenTerm")
            .with_decorations(self.config.window.decorations)
            .with_inner_size(winit::dpi::LogicalSize::new(1100.0, 700.0));
        let window = Arc::new(event_loop.create_window(attrs).expect("pencere oluşturulamadı"));

        let size = window.inner_size();
        self.recompute_dims(size.width, size.height);

        let gpu = pollster::block_on(GpuState::new(window.clone(), size.width, size.height))
            .expect("wgpu/Vulkan başlatılamadı");
        gpu.pipeline
            .update_screen_size(&gpu.queue, size.width as f32, size.height as f32);

        let session = Session::spawn(self.dims, (self.cell_w as u16, self.cell_h as u16))
            .expect("PTY/shell başlatılamadı");

        let pad = self.config.window.padding as f32;
        self.cursor_anim = CursorAnim::new((pad, pad));
        self.next_blink = Instant::now() + Duration::from_millis(self.config.cursor.blink_interval_ms);

        self.window = Some(window);
        self.gpu = Some(gpu);
        self.session = Some(session);
        self.dirty = true;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                    gpu.pipeline
                        .update_screen_size(&gpu.queue, size.width as f32, size.height as f32);
                }
                self.recompute_dims(size.width, size.height);
                if let Some(session) = &self.session {
                    session.resize(self.dims, (self.cell_w as u16, self.cell_h as u16));
                }
                self.dirty = true;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    if matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyP))
                        && input::ctrl_shift_held(self.modifiers)
                    {
                        self.ui.toggle_palette();
                        self.dirty = true;
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                        return;
                    }
                    if let Some(bytes) = input::key_to_bytes(&event, self.modifiers) {
                        if let Some(session) = &mut self.session {
                            let _ = session.write_input(&bytes);
                        }
                        // Kullanıcı yazarken scrollback'ten çıkıp en alta dön.
                        if let Some(session) = &self.session {
                            session.term.lock().scroll_display(Scroll::Bottom);
                        }
                        self.selection = None;
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x, position.y);
                if self.mouse_down {
                    let pad = self.config.window.padding as f32;
                    let cell = pixel_to_cell(self.mouse_pos, pad, self.cell_w, self.cell_h, self.dims);
                    if let Some(sel) = &mut self.selection {
                        sel.cursor = cell;
                        self.dirty = true;
                        if let Some(w) = &self.window {
                            w.request_redraw();
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let pad = self.config.window.padding as f32;
                    match state {
                        ElementState::Pressed => {
                            let cell = pixel_to_cell(self.mouse_pos, pad, self.cell_w, self.cell_h, self.dims);
                            self.selection = Some(Selection { anchor: cell, cursor: cell });
                            self.mouse_down = true;
                            self.dirty = true;
                        }
                        ElementState::Released => {
                            self.mouse_down = false;
                            if let Some(sel) = self.selection {
                                if sel.anchor == sel.cursor {
                                    self.selection = None;
                                } else {
                                    let text = self.extract_selection_text(&sel);
                                    if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                        let _ = clipboard.set_text(text);
                                    }
                                }
                            }
                            self.dirty = true;
                        }
                    }
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / self.cell_h as f64).round() as i32,
                };
                if lines != 0 {
                    if let Some(session) = &self.session {
                        session.term.lock().scroll_display(Scroll::Delta(lines));
                    }
                    self.selection = None;
                    self.dirty = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                if !self.dirty {
                    return;
                }
                let (mut instances, cursor_target) = self.build_instances();

                if let Some(target) = cursor_target {
                    self.cursor_anim.retarget(target, self.config.cursor.animation_ms);
                }

                if self.blink_on && cursor_target.is_some() {
                    let pos = self.cursor_anim.current(self.config.cursor.animation_ms);
                    let cursor_color = hex_to_rgba(&self.config.theme.cursor);
                    instances.push(CellInstance {
                        pos: [pos.0, pos.1],
                        size: [self.cell_w, self.cell_h],
                        bg_color: [cursor_color[0], cursor_color[1], cursor_color[2], 0.55],
                        fg_color: [0.0; 4],
                        atlas_uv_min: [0.0, 0.0],
                        atlas_uv_max: [0.0, 0.0],
                        has_glyph: 0.0,
                        color_glyph: 0.0,
                    });
                }

                if let Some(gpu) = &mut self.gpu {
                    let bg = hex_to_rgba(&self.config.theme.background);
                    let clear = wgpu::Color {
                        r: bg[0] as f64,
                        g: bg[1] as f64,
                        b: bg[2] as f64,
                        a: self.config.theme.opacity as f64,
                    };
                    if let Err(e) = gpu.render(&instances, clear) {
                        log::warn!("render hatası: {e:?}");
                    }
                }
                self.dirty = false;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let mut need_redraw = false;

        // PTY/çekirdek olaylarını tüket (damage tracking: yalnızca
        // gerçekten bir şey değiştiyse redraw isteriz).
        if let Some(session) = &self.session {
            while let Ok(event) = session.events_rx.try_recv() {
                match event {
                    CoreEvent::Wakeup => need_redraw = true,
                    CoreEvent::ClipboardStore(text) => {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(text);
                        }
                    }
                    CoreEvent::TitleChanged(title) => {
                        if let Some(window) = &self.window {
                            window.set_title(&title);
                        }
                    }
                    CoreEvent::ChildExited => {
                        event_loop.exit();
                    }
                    CoreEvent::Other(_) => {}
                }
            }
        }

        // Config hot-reload.
        if let Some(watcher) = &self.config_watcher {
            if let Some(new_config) = watcher.poll() {
                log::info!("Config yeniden yüklendi (hot reload)");
                self.config = new_config;
                need_redraw = true;
            }
        }

        // Cursor blink zamanlaması.
        let now = Instant::now();
        let mut next_wake: Option<Instant> = None;
        if self.config.cursor.blink {
            if now >= self.next_blink {
                self.blink_on = !self.blink_on;
                self.next_blink = now + Duration::from_millis(self.config.cursor.blink_interval_ms);
                need_redraw = true;
            }
            next_wake = Some(self.next_blink);
        }

        // Cursor konum animasyonu sürüyorsa yaklaşık 60fps ile ilerlet.
        if self.cursor_anim.is_animating(self.config.cursor.animation_ms) {
            need_redraw = true;
            let frame_wake = now + Duration::from_millis(16);
            next_wake = Some(next_wake.map_or(frame_wake, |w| w.min(frame_wake)));
        }

        if need_redraw {
            self.dirty = true;
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }

        event_loop.set_control_flow(match next_wake {
            Some(instant) => ControlFlow::WaitUntil(instant),
            None => ControlFlow::Wait,
        });
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new();
    event_loop.run_app(&mut app)?;
    Ok(())
}
