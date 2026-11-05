//! LumenTerm — Faz 5 giriş noktası.
//!
//! Faz 1-4'ün üzerine: "katmanlı scrollback" kimlik özelliği -- OSC 133
//! ile işaretlenen komut blokları (`Ctrl+Shift+Enter` ile fare altındaki
//! bloğu katla/genişlet) ve chrome cilası (yuvarlatılmış köşeler, aktif
//! pane odak çizgisi, komut paletinde yumuşak açılış animasyonu) eklendi.

mod input;

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use lumenterm_config::Config;
use lumenterm_core::{CoreEvent, Session, TermDimensions};
use lumenterm_font::FontManager;
use lumenterm_renderer::{CellInstance, GlyphKey, GpuState};
use lumenterm_ui::{PaneId, SplitDirection, UiState};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{Window, WindowAttributes, WindowId};

const DIVIDER_PX: f32 = 2.0;
const PANE_INNER_PAD: f32 = 4.0;

/// Fare pozisyonunu, bir pane'in dikdörtgenine göre (line, col) hücre
/// koordinatına çevirir.
fn pixel_to_cell(pos: (f64, f64), rect_x: f32, rect_y: f32, cell_w: f32, cell_h: f32, dims: TermDimensions) -> (usize, usize) {
    let col = (((pos.0 as f32 - rect_x - PANE_INNER_PAD) / cell_w).floor().max(0.0)) as usize;
    let line = (((pos.1 as f32 - rect_y - PANE_INNER_PAD) / cell_h).floor().max(0.0)) as usize;
    (line.min(dims.lines.saturating_sub(1)), col.min(dims.columns.saturating_sub(1)))
}

#[derive(Debug, Clone, Copy)]
struct Selection {
    anchor: (usize, usize),
    cursor: (usize, usize),
}

impl Selection {
    fn normalized(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.cursor { (self.anchor, self.cursor) } else { (self.cursor, self.anchor) }
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
        let eased = 1.0 - (1.0 - t) * (1.0 - t);
        (self.from.0 + (self.to.0 - self.from.0) * eased, self.from.1 + (self.to.1 - self.from.1) * eased)
    }

    fn is_animating(&self, duration_ms: u64) -> bool {
        self.start.elapsed().as_millis() < duration_ms as u128
    }
}

/// Tek bir pane'e özgü çalışma zamanı durumu. `font`/GPU gibi paylaşılan
/// kaynaklar `App`'te kalır; burada yalnızca pane'e özgü olanlar var.
struct PaneRuntime {
    session: Session,
    cursor_anim: CursorAnim,
    selection: Option<Selection>,
    last_dims: TermDimensions,
}

struct App {
    config: Config,
    config_watcher: Option<lumenterm_config::ConfigWatcher>,
    window: Option<Arc<Window>>,
    gpu: Option<GpuState>,
    font: FontManager,
    panes: HashMap<PaneId, PaneRuntime>,
    ui: UiState,
    cell_w: f32,
    cell_h: f32,
    descent: f32,
    dirty: bool,
    modifiers: ModifiersState,
    mouse_pos: (f64, f64),
    mouse_down: bool,
    selecting_pane: Option<PaneId>,
    resizing_divider: Option<u64>,
    // Son `RedrawRequested`'te hesaplanan layout; fare hit-testi için
    // (girdi olayları render'dan önce gelebileceğinden bir önceki
    // frame'in layout'u kullanılır -- pencere ilk açıldığında boş olabilir).
    last_panes: Vec<lumenterm_ui::splits::PaneRect>,
    last_dividers: Vec<lumenterm_ui::splits::DividerRect>,
    last_splits: Vec<lumenterm_ui::splits::SplitRect>,
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

        Self {
            config,
            config_watcher,
            window: None,
            gpu: None,
            font,
            panes: HashMap::new(),
            ui: UiState::default(),
            cell_w: metrics.width,
            cell_h: metrics.height,
            descent: metrics.descent,
            dirty: true,
            modifiers: ModifiersState::empty(),
            mouse_pos: (0.0, 0.0),
            mouse_down: false,
            selecting_pane: None,
            resizing_divider: None,
            last_panes: Vec::new(),
            last_dividers: Vec::new(),
            last_splits: Vec::new(),
            blink_on: true,
            next_blink: Instant::now(),
        }
    }

    fn usable_area(&self) -> (f32, f32, f32, f32) {
        let pad = self.config.window.padding as f32;
        let (w, h) = self.gpu.as_ref().map(|g| g.size).unwrap_or((1100, 700));
        (pad, pad, w as f32 - 2.0 * pad, h as f32 - 2.0 * pad)
    }

    fn relayout(&mut self) {
        let (x, y, w, h) = self.usable_area();
        let (panes, dividers, splits) = self.ui.splits.layout(x, y, w, h, DIVIDER_PX);
        for rect in &panes {
            let dims = self.dims_for_rect(rect.w, rect.h);
            if let Some(runtime) = self.panes.get_mut(&rect.pane) {
                if runtime.last_dims != dims {
                    runtime.session.resize(dims, (self.cell_w as u16, self.cell_h as u16));
                    runtime.last_dims = dims;
                }
            }
        }
        self.last_panes = panes;
        self.last_dividers = dividers;
        self.last_splits = splits;
    }

    fn dims_for_rect(&self, w: f32, h: f32) -> TermDimensions {
        let cols = (((w - 2.0 * PANE_INNER_PAD) / self.cell_w).floor().max(1.0)) as usize;
        let lines = (((h - 2.0 * PANE_INNER_PAD) / self.cell_h).floor().max(1.0)) as usize;
        TermDimensions { columns: cols, lines }
    }

    fn spawn_pane(&mut self, pane_id: PaneId, w: f32, h: f32) {
        let dims = self.dims_for_rect(w, h);
        if let Ok(session) = Session::spawn(dims, (self.cell_w as u16, self.cell_h as u16)) {
            self.panes.insert(
                pane_id,
                PaneRuntime {
                    session,
                    cursor_anim: CursorAnim::new((0.0, 0.0)),
                    selection: None,
                    last_dims: dims,
                },
            );
        } else {
            log::error!("Pane {pane_id} için PTY/shell başlatılamadı");
        }
    }

    fn split_and_spawn(&mut self, direction: SplitDirection) {
        let new_id = self.ui.splits.split_active(direction);
        self.relayout();
        if let Some(rect) = self.last_panes.iter().find(|r| r.pane == new_id) {
            let (w, h) = (rect.w, rect.h);
            self.spawn_pane(new_id, w, h);
        }
        self.relayout();
        self.dirty = true;
    }

    fn close_active_pane(&mut self) {
        let active = self.ui.splits.active;
        if let Some(_promoted) = self.ui.splits.close_pane(active) {
            self.panes.remove(&active);
            self.relayout();
            self.dirty = true;
        }
    }

    /// Fare, hangi pane üzerindeyse o pane'de farenin bulunduğu ekran
    /// satırının karşılık geldiği bloğu bulup katlama durumunu çevirir
    /// (Faz 5 kimlik özelliği). Satır bir "fold chip" ise doğrudan o
    /// bloğu, normal bir grid satırıysa o satırı içeren bloğu hedefler.
    fn toggle_fold_under_mouse(&mut self) {
        let Some(rect) = self.last_panes.iter().find(|r| hit_rect(r, self.mouse_pos)).copied() else { return };
        let Some(runtime) = self.panes.get(&rect.pane) else { return };
        let dims = runtime.last_dims;
        let (screen_line, _col) = pixel_to_cell(self.mouse_pos, rect.x, rect.y, self.cell_w, self.cell_h, dims);

        let display_offset = runtime.session.term.lock().grid().display_offset() as i32;
        let mut blocks = runtime.session.blocks.lock();
        let rows = compute_rows(dims, display_offset, &blocks.blocks, self.config.blocks.enabled);
        if let Some(row) = rows.get(screen_line).copied() {
            match row {
                RowSource::FoldChip { block_index } => blocks.toggle_fold(block_index),
                RowSource::GridLine(grid_line) => {
                    if let Some(idx) = blocks.block_at_line(grid_line) {
                        blocks.toggle_fold(idx);
                    }
                }
            }
        }
        drop(blocks);

        self.dirty = true;
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn cycle_focus(&mut self) {
        let panes = self.ui.splits.panes();
        if panes.len() < 2 {
            return;
        }
        let idx = panes.iter().position(|p| *p == self.ui.splits.active).unwrap_or(0);
        self.ui.splits.active = panes[(idx + 1) % panes.len()];
        self.dirty = true;
    }

    /// Seçili hücrelerin metnini çıkarır (kopyalama için).
    fn extract_selection_text(&self, session: &Session, sel: &Selection, dims: TermDimensions) -> String {
        let term = session.term.lock();
        let grid = term.grid();
        let display_offset = grid.display_offset() as i32;
        let (start, end) = sel.normalized();

        let mut out = String::new();
        for line in start.0..=end.0 {
            let (col_start, col_end) = if start.0 == end.0 {
                (start.1, end.1)
            } else if line == start.0 {
                (start.1, dims.columns.saturating_sub(1))
            } else if line == end.0 {
                (0, end.1)
            } else {
                (0, dims.columns.saturating_sub(1))
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

    /// Tüm pane'leri + ayraçları + (varsa) komut paletini tek bir
    /// instance listesine çevirir. Görseller listenin başında (metnin
    /// altında, zaten o hücrelerde metin çizilmiyor), hücreler ortada,
    /// ayraçlar ve palet en üstte.
    fn build_frame(&mut self) -> Vec<CellInstance> {
        let Some(gpu) = self.gpu.as_mut() else { return Vec::new() };
        let bg_default = hex_to_rgba(&self.config.theme.background);
        let fg_default = hex_to_rgba(&self.config.theme.foreground);
        let divider_color = {
            let mut c = bg_default;
            c[0] = (c[0] + 0.12).min(1.0);
            c[1] = (c[1] + 0.12).min(1.0);
            c[2] = (c[2] + 0.12).min(1.0);
            c
        };

        let mut image_instances = Vec::new();
        let mut cell_instances = Vec::new();

        let panes = self.last_panes.clone();
        for rect in &panes {
            let Some(runtime) = self.panes.get_mut(&rect.pane) else { continue };
            let dims = runtime.last_dims;
            let origin_x = rect.x + PANE_INNER_PAD;
            let origin_y = rect.y + PANE_INNER_PAD;

            let term = runtime.session.term.lock();
            let grid = term.grid();
            let display_offset = grid.display_offset() as i32;

            // Faz 5: katmanlı scrollback. `rows` -- bu pane'in bu frame'de
            // çizeceği her ekran satırının kaynağını listeler: ya normal
            // bir grid satırı, ya da katlı bir bloğun tek-satırlık özeti
            // ("chip"). Bloklar kapalıysa (`config.blocks.enabled=false`)
            // ya da hiç katlı blok yoksa, bu tam olarak eski
            // `grid_line = screen_line - display_offset` davranışına denk
            // gelir -- kimse fark etmez.
            let blocks_guard = runtime.session.blocks.lock();
            let rows = compute_rows(dims, display_offset, &blocks_guard.blocks, self.config.blocks.enabled);
            let blocks_snapshot: Vec<lumenterm_core::Block> = blocks_guard.blocks.clone();
            drop(blocks_guard);

            let mut line_to_screen: HashMap<i32, usize> = HashMap::new();
            for (screen_row, src) in rows.iter().enumerate() {
                if let RowSource::GridLine(gl) = src {
                    line_to_screen.insert(*gl, screen_row);
                }
            }

            // Bu pane'de görünür alanda olan görsel yerleşimlerini bul
            // (mutlak grid satırı -- bkz. `line_to_screen` haritası).
            let graphics = runtime.session.graphics.lock();
            let mut covered: HashSet<(i32, usize)> = HashSet::new();
            let mut pane_images: Vec<(u32, u32, u32, usize, usize, usize, usize)> = Vec::new(); // id,w,h,screen_row,col,cols,rows
            for placement in &graphics.placements {
                let Some(&screen_row) = line_to_screen.get(&placement.row) else { continue };
                let Some(img) = graphics.images.get(&placement.image_id) else { continue };
                for r in 0..placement.rows {
                    let gl = placement.row + r as i32;
                    for c in 0..placement.cols {
                        let sc = placement.col + c;
                        if sc < dims.columns {
                            covered.insert((gl, sc));
                        }
                    }
                }
                pane_images.push((placement.image_id, img.width, img.height, screen_row, placement.col, placement.cols, placement.rows));
            }
            drop(graphics);

            let chip_bg = {
                let mut c = bg_default;
                c[0] = (c[0] + 0.08).min(1.0);
                c[1] = (c[1] + 0.08).min(1.0);
                c[2] = (c[2] + 0.10).min(1.0);
                c
            };
            let accent = hex_to_rgba(&self.config.theme.cursor);

            for (screen_row, row_source) in rows.iter().enumerate() {
                let line = screen_row;
                match *row_source {
                    RowSource::FoldChip { block_index } => {
                        let block = &blocks_snapshot[block_index];
                        let cy = origin_y + line as f32 * self.cell_h;
                        cell_instances.push(CellInstance {
                            pos: [origin_x, cy],
                            size: [dims.columns as f32 * self.cell_w, self.cell_h],
                            bg_color: chip_bg,
                            fg_color: [0.0; 4],
                            atlas_uv_min: [0.0, 0.0],
                            atlas_uv_max: [0.0, 0.0],
                            has_glyph: 0.0,
                            color_glyph: 0.0,
                            corner_radius: 6.0,
                        });
                        let summary = fold_chip_summary(grid, block, dims);
                        push_text(&mut cell_instances, gpu, &mut self.font, &summary, origin_x + 6.0, cy, self.cell_w, self.cell_h, self.descent, accent);
                    }
                    RowSource::GridLine(grid_line) => {
                        for col in 0..dims.columns {
                            if covered.contains(&(grid_line, col)) {
                                continue;
                            }
                            let point = Point::new(Line(grid_line), Column(col));
                            let cell = &grid[point];

                            let mut cell_bg = cell_color(cell.bg, &self.config, bg_default, true);
                            let mut cell_fg = cell_color(cell.fg, &self.config, fg_default, false);
                            if cell.flags.contains(Flags::INVERSE) {
                                std::mem::swap(&mut cell_bg, &mut cell_fg);
                            }
                            if let Some(sel) = &runtime.selection {
                                if sel.contains(line, col) {
                                    cell_bg = blend_highlight(cell_bg);
                                }
                            }

                            let cx = origin_x + col as f32 * self.cell_w;
                            let cy = origin_y + line as f32 * self.cell_h;

                            cell_instances.push(CellInstance {
                                pos: [cx, cy],
                                size: [self.cell_w, self.cell_h],
                                bg_color: cell_bg,
                                fg_color: cell_fg,
                                atlas_uv_min: [0.0, 0.0],
                                atlas_uv_max: [0.0, 0.0],
                                has_glyph: 0.0,
                                color_glyph: 0.0,
                                corner_radius: 0.0,
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
                                        cell_instances.push(CellInstance {
                                            pos: [gx, gy],
                                            size: [entry.width as f32, entry.height as f32],
                                            bg_color: [0.0; 4],
                                            fg_color: cell_fg,
                                            atlas_uv_min: entry.uv_min,
                                            atlas_uv_max: entry.uv_max,
                                            has_glyph: 1.0,
                                            color_glyph: if entry.color_glyph { 1.0 } else { 0.0 },
                                            corner_radius: 0.0,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Görselleri paketle + çiz.
            for (image_id, iw, ih, screen_row, col, cols, rows) in pane_images {
                let graphics = runtime.session.graphics.lock();
                let Some(img) = graphics.images.get(&image_id) else { continue };
                let rgba_ptr = img.rgba.as_slice();
                if let Some(entry) = gpu.image_entry(image_id, iw, ih, rgba_ptr) {
                    let ix = origin_x + col as f32 * self.cell_w;
                    let iy = origin_y + screen_row as f32 * self.cell_h;
                    image_instances.push(CellInstance {
                        pos: [ix, iy],
                        size: [cols as f32 * self.cell_w, rows as f32 * self.cell_h],
                        bg_color: [0.0; 4],
                        fg_color: [0.0; 4],
                        atlas_uv_min: entry.uv_min,
                        atlas_uv_max: entry.uv_max,
                        has_glyph: 1.0,
                        color_glyph: 1.0,
                        corner_radius: 0.0,
                    });
                }
            }

            // Cursor (katlı bloklar varsa, imlecin satırı yukarı kaymış
            // olabilir -- `line_to_screen` ile gerçek ekran satırını bul).
            let cursor_target = if display_offset == 0 {
                let cp = grid.cursor.point;
                let (cl, cc) = (cp.line.0, cp.column.0);
                line_to_screen.get(&cl).and_then(|&screen_line| {
                    if cc < dims.columns {
                        Some((origin_x + cc as f32 * self.cell_w, origin_y + screen_line as f32 * self.cell_h))
                    } else {
                        None
                    }
                })
            } else {
                None
            };
            drop(term);

            if let Some(target) = cursor_target {
                runtime.cursor_anim.retarget(target, self.config.cursor.animation_ms);
                if self.blink_on && rect.pane == self.ui.splits.active {
                    let pos = runtime.cursor_anim.current(self.config.cursor.animation_ms);
                    let cursor_color = hex_to_rgba(&self.config.theme.cursor);
                    cell_instances.push(CellInstance {
                        pos: [pos.0, pos.1],
                        size: [self.cell_w, self.cell_h],
                        bg_color: [cursor_color[0], cursor_color[1], cursor_color[2], 0.55],
                        fg_color: [0.0; 4],
                        atlas_uv_min: [0.0, 0.0],
                        atlas_uv_max: [0.0, 0.0],
                        has_glyph: 0.0,
                        color_glyph: 0.0,
                        corner_radius: 3.0,
                    });
                }
            }

            // Chrome cilası: aktif pane'in üstünde ince, vurgu renkli bir
            // odak çizgisi -- geleneksel bir pencere kenarlığı olmadan
            // "hangi pane aktif" sorusunu yanıtlar.
            if rect.pane == self.ui.splits.active {
                cell_instances.push(CellInstance {
                    pos: [rect.x, rect.y],
                    size: [rect.w, 2.0],
                    bg_color: [accent[0], accent[1], accent[2], 0.9],
                    fg_color: [0.0; 4],
                    atlas_uv_min: [0.0, 0.0],
                    atlas_uv_max: [0.0, 0.0],
                    has_glyph: 0.0,
                    color_glyph: 0.0,
                    corner_radius: 1.0,
                });
            }
        }

        // Ayraçlar.
        for d in &self.last_dividers {
            cell_instances.push(CellInstance {
                pos: [d.x, d.y],
                size: [d.w, d.h],
                bg_color: divider_color,
                fg_color: [0.0; 4],
                atlas_uv_min: [0.0, 0.0],
                atlas_uv_max: [0.0, 0.0],
                has_glyph: 0.0,
                color_glyph: 0.0,
                corner_radius: 0.0,
            });
        }

        let mut out = Vec::with_capacity(image_instances.len() + cell_instances.len());
        out.extend(image_instances);
        out.extend(cell_instances);

        if self.ui.palette_open {
            push_palette(
                &self.config,
                self.cell_w,
                self.cell_h,
                self.descent,
                gpu,
                &mut self.font,
                &mut self.ui.palette,
                self.ui.palette_opened_at,
                &mut out,
            );
        }

        out
    }
}

/// Komut paletini ekranın üst-orta kısmında bir panel olarak çizer: arka
/// plan + sorgu satırı + sonuç listesi. Aynı `CellInstance` pipeline'ını
/// (glyph atlas dahil) kullanır, GPU tarafında ekstra hiçbir şeye ihtiyaç
/// duymaz. Serbest bir fonksiyon: `App::build_frame` içinde zaten canlı
/// olan `gpu: &mut GpuState` ödüncünü, `&mut self` gerektiren bir metod
/// çağrısıyla çakıştırmamak için (field-disjoint ödünç yerine tüm
/// `self`'i isteyen bir metod, aktif `gpu` alanı ödüncüyle çakışırdı).
fn push_palette(
    config: &Config,
    cell_w: f32,
    cell_h: f32,
    descent: f32,
    gpu: &mut GpuState,
    font: &mut FontManager,
    palette: &mut lumenterm_ui::CommandPalette,
    palette_opened_at: Option<std::time::Instant>,
    out: &mut Vec<CellInstance>,
) {
    palette.tick();
    let window_w = gpu.size.0 as f32;
    let pad = config.window.padding as f32;
    let usable_w = window_w - 2.0 * pad;
    let panel_w = (usable_w * 0.6).clamp(300.0, 900.0);
    let panel_x = (window_w - panel_w) / 2.0;
    let panel_y = 60.0;
    let row_h = cell_h * 1.3;
    let results = palette.results(10);
    let panel_h = row_h * (results.len() as f32 + 1.5);

    // Yumuşak açılış: panel 120ms içinde hafifçe yukarıdan kayarak ve
    // saydamlıktan belirerek açılır -- "minimal chrome ama yumuşak
    // animasyonlar" hedefinin küçük bir yansıması.
    const FADE_MS: f32 = 120.0;
    let (fade_t, slide_y) = match palette_opened_at {
        Some(start) => {
            let elapsed = start.elapsed().as_secs_f32() * 1000.0;
            let t = (elapsed / FADE_MS).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t) * (1.0 - t);
            (eased, (1.0 - eased) * 10.0)
        }
        None => (1.0, 0.0),
    };
    let panel_y = panel_y + slide_y;

    let panel_bg = hex_to_rgba(&config.theme.background);
    let mut border_bg = panel_bg;
    border_bg[0] = (border_bg[0] + 0.1).min(1.0);
    border_bg[1] = (border_bg[1] + 0.1).min(1.0);
    border_bg[2] = (border_bg[2] + 0.1).min(1.0);

    out.push(CellInstance {
        pos: [panel_x, panel_y],
        size: [panel_w, panel_h],
        bg_color: [border_bg[0], border_bg[1], border_bg[2], 0.98 * fade_t],
        fg_color: [0.0; 4],
        atlas_uv_min: [0.0, 0.0],
        atlas_uv_max: [0.0, 0.0],
        has_glyph: 0.0,
        color_glyph: 0.0,
        corner_radius: config.window.corner_radius,
    });

    let fg = hex_to_rgba(&config.theme.foreground);
    let accent = hex_to_rgba(&config.theme.cursor);
    let query_display = if palette.query.is_empty() { "Ara...".to_string() } else { palette.query.clone() };
    push_text(out, gpu, font, &query_display, panel_x + 12.0, panel_y + 8.0, cell_w, cell_h, descent, fg);

    for (i, label) in results.iter().enumerate() {
        let y = panel_y + row_h * (1.3 + i as f32);
        let color = if i == palette.selected { accent } else { fg };
        if i == palette.selected {
            out.push(CellInstance {
                pos: [panel_x + 4.0, y - 2.0],
                size: [panel_w - 8.0, row_h],
                bg_color: [color[0], color[1], color[2], 0.15 * fade_t],
                fg_color: [0.0; 4],
                atlas_uv_min: [0.0, 0.0],
                atlas_uv_max: [0.0, 0.0],
                has_glyph: 0.0,
                color_glyph: 0.0,
                corner_radius: (config.window.corner_radius * 0.6).min(row_h * 0.4),
            });
        }
        push_text(out, gpu, font, label, panel_x + 12.0, y, cell_w, cell_h, descent, color);
    }
}

/// Bir metin satırını, hücre-bazlı değil serbest piksel pozisyonunda
/// (palet gibi UI yüzeyleri için) çizen yardımcı -- aynı glyph atlas'ı
/// paylaşır.
fn push_text(
    out: &mut Vec<CellInstance>,
    gpu: &mut GpuState,
    font: &mut FontManager,
    text: &str,
    x: f32,
    y: f32,
    cell_w: f32,
    cell_h: f32,
    descent: f32,
    color: [f32; 4],
) {
    let mut cx = x;
    for ch in text.chars() {
        if ch != ' ' {
            if let Some(raster) = font.rasterize_char(ch, false, false) {
                let key = GlyphKey { ch, bold: false, italic: false };
                if let Some(entry) = gpu.glyph_entry(key, &raster) {
                    let baseline_y = y + (cell_h - descent);
                    let gx = cx + entry.left as f32;
                    let gy = baseline_y - entry.top as f32;
                    out.push(CellInstance {
                        pos: [gx, gy],
                        size: [entry.width as f32, entry.height as f32],
                        bg_color: [0.0; 4],
                        fg_color: color,
                        atlas_uv_min: entry.uv_min,
                        atlas_uv_max: entry.uv_max,
                        has_glyph: 1.0,
                        color_glyph: if entry.color_glyph { 1.0 } else { 0.0 },
                        corner_radius: 0.0,
                    });
                }
            }
        }
        cx += cell_w;
    }
}

/// Bir pane'in ekranda çizeceği tek bir satırın kaynağı -- Faz 5'in
/// katmanlı scrollback'i için (bkz. `compute_rows`).
#[derive(Debug, Clone, Copy)]
enum RowSource {
    /// Normal durum: bu ekran satırı, şu mutlak grid satırını gösterir.
    GridLine(i32),
    /// Katlı bir bloğun (komut girişi + çıktısı) yerine geçen tek
    /// satırlık özet; `block_index`, o pane'in `blocks_snapshot`'ındaki
    /// indeks.
    FoldChip { block_index: usize },
}

/// Bir pane için, ekranın her satırının hangi grid satırından (ya da
/// katlı bir bloğun özetinden) geleceğini hesaplar. Blok katmanı kapalıysa
/// ya da hiçbir blok katlı değilse, bu tam olarak klasik
/// `grid_line = screen_line - display_offset` eşlemesine denk gelir.
///
/// Güvenlik notu: katlama, aynı ekran satırı bütçesiyle normalde
/// görünmeyecek kadar "ileri" (yeni) içeriğe ulaşmayı sağlar, ama
/// `alacritty_terminal`'in `Grid` indeksleyicisi aktif ekranın dışına
/// çıkan bir `Line` için panikleyebilir. Bu yüzden `grid_line` hiçbir
/// zaman katlama olmadan ulaşılabilecek en büyük değerin (`dims.lines-1-
/// display_offset`) ötesine geçmez -- katlama bu pencere *içinde*
/// sıkıştırma yapar, henüz ekstra geçmişi pencereye çekmez (bkz. README,
/// "Faz 5'te alınan tasarım kararları").
fn compute_rows(dims: TermDimensions, display_offset: i32, blocks: &[lumenterm_core::Block], enabled: bool) -> Vec<RowSource> {
    let mut out = Vec::with_capacity(dims.lines);
    let mut grid_line = -display_offset;
    let max_grid_line = dims.lines as i32 - 1 - display_offset;

    while out.len() < dims.lines && grid_line <= max_grid_line {
        if enabled {
            let hidden = blocks.iter().enumerate().find_map(|(idx, b)| b.hidden_range().map(|r| (idx, r)));
            if let Some((idx, (start, end))) = hidden {
                if grid_line >= start && grid_line <= end {
                    if grid_line == start {
                        out.push(RowSource::FoldChip { block_index: idx });
                    }
                    grid_line = end + 1;
                    continue;
                }
            }
        }
        out.push(RowSource::GridLine(grid_line));
        grid_line += 1;
    }
    out
}

/// Katlı bir blok için tek satırlık özet metni üretir: komutun kendisini
/// (grid'den, `command_start`..`output_start` satır aralığından okuyarak)
/// ve kaç satır çıktı + çıkış koduyla birlikte gösterir.
fn fold_chip_summary(grid: &alacritty_terminal::Grid<alacritty_terminal::term::cell::Cell>, block: &lumenterm_core::Block, dims: TermDimensions) -> String {
    let command_line = block.command_start.unwrap_or(block.prompt_start);
    let mut command = String::new();
    for col in 0..dims.columns {
        let point = Point::new(Line(command_line), Column(col));
        command.push(grid[point].c);
    }
    let command = command.trim_end();
    let command = if command.is_empty() { "(komut)" } else { command };

    let output_lines = match (block.output_start, block.output_end) {
        (Some(s), Some(e)) if e >= s => (e - s + 1).max(0),
        _ => 0,
    };
    let exit = block.exit_code.map(|c| format!("exit {c}")).unwrap_or_else(|| "...".to_string());
    format!("▸ {command}  ·  {output_lines} satır çıktı  ·  {exit}  (Ctrl+Shift+Enter ile genişlet)")
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

fn blend_highlight(base: [f32; 4]) -> [f32; 4] {
    [(base[0] + 0.35).min(1.0), (base[1] + 0.35).min(1.0), (base[2] + 0.45).min(1.0), base[3]]
}

fn cell_color(color: alacritty_terminal::vte::ansi::Color, config: &Config, default: [f32; 4], is_bg: bool) -> [f32; 4] {
    use alacritty_terminal::vte::ansi::{Color, NamedColor};
    match color {
        Color::Named(NamedColor::Background) if is_bg => default,
        Color::Named(NamedColor::Foreground) if !is_bg => default,
        Color::Named(named) => config.theme.palette.get(named as usize % 16).map(|h| hex_to_rgba(h)).unwrap_or(default),
        Color::Spec(rgb) => [rgb.r as f32 / 255.0, rgb.g as f32 / 255.0, rgb.b as f32 / 255.0, 1.0],
        Color::Indexed(i) => config.theme.palette.get(i as usize % 16).map(|h| hex_to_rgba(h)).unwrap_or(default),
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
        let gpu = pollster::block_on(GpuState::new(window.clone(), size.width, size.height)).expect("wgpu/Vulkan başlatılamadı");
        gpu.pipeline.update_screen_size(&gpu.queue, size.width as f32, size.height as f32);
        self.gpu = Some(gpu);
        self.window = Some(window);

        self.relayout();
        let root_rect = self.last_panes.first().cloned();
        if let Some(rect) = root_rect {
            self.spawn_pane(rect.pane, rect.w, rect.h);
        }
        self.relayout();

        self.next_blink = Instant::now() + Duration::from_millis(self.config.cursor.blink_interval_ms);
        self.dirty = true;
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::ModifiersChanged(mods) => self.modifiers = mods.state(),
            WindowEvent::Resized(size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.resize(size.width, size.height);
                    gpu.pipeline.update_screen_size(&gpu.queue, size.width as f32, size.height as f32);
                }
                self.relayout();
                self.dirty = true;
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }

                if self.ui.palette_open {
                    self.handle_palette_key(&event);
                    self.dirty = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }

                if matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyP)) && input::ctrl_shift_held(self.modifiers) {
                    self.ui.toggle_palette();
                    self.dirty = true;
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                    return;
                }
                if matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyO)) && input::ctrl_shift_held(self.modifiers) {
                    self.split_and_spawn(SplitDirection::Horizontal);
                    return;
                }
                if matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyU)) && input::ctrl_shift_held(self.modifiers) {
                    self.split_and_spawn(SplitDirection::Vertical);
                    return;
                }
                if matches!(event.physical_key, PhysicalKey::Code(KeyCode::KeyW)) && input::ctrl_shift_held(self.modifiers) {
                    self.close_active_pane();
                    return;
                }
                if matches!(event.logical_key, Key::Named(NamedKey::Tab)) && self.modifiers.alt_key() {
                    self.cycle_focus();
                    return;
                }
                // Faz 5 kimlik özelliği: farenin üzerinde durduğu bloğu
                // katla/genişlet. Varsayılan kısayol `Ctrl+Shift+Enter`
                // (bkz. `BlocksConfig::fold_shortcut` -- şu an yalnızca
                // bu sabit kısayol olarak uygulanıyor, config'teki metin
                // henüz ayrıştırılmıyor).
                if matches!(event.logical_key, Key::Named(NamedKey::Enter)) && input::ctrl_shift_held(self.modifiers) {
                    self.toggle_fold_under_mouse();
                    return;
                }

                if let Some(bytes) = input::key_to_bytes(&event, self.modifiers) {
                    let active = self.ui.splits.active;
                    if let Some(runtime) = self.panes.get_mut(&active) {
                        let _ = runtime.session.write_input(&bytes);
                        runtime.session.term.lock().scroll_display(Scroll::Bottom);
                        runtime.selection = None;
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse_pos = (position.x, position.y);
                if let Some(split_id) = self.resizing_divider {
                    if let Some(sr) = self.last_splits.iter().find(|s| s.split == split_id).copied() {
                        let ratio = match sr.direction {
                            SplitDirection::Horizontal => ((self.mouse_pos.0 as f32 - sr.x) / sr.w).clamp(0.0, 1.0),
                            SplitDirection::Vertical => ((self.mouse_pos.1 as f32 - sr.y) / sr.h).clamp(0.0, 1.0),
                        };
                        self.ui.splits.set_ratio(split_id, ratio);
                        self.relayout();
                        self.dirty = true;
                    }
                } else if self.mouse_down {
                    if let Some(pane_id) = self.selecting_pane {
                        if let Some(rect) = self.last_panes.iter().find(|r| r.pane == pane_id).copied() {
                            if let Some(runtime) = self.panes.get_mut(&pane_id) {
                                let cell = pixel_to_cell(self.mouse_pos, rect.x, rect.y, self.cell_w, self.cell_h, runtime.last_dims);
                                if let Some(sel) = &mut runtime.selection {
                                    sel.cursor = cell;
                                    self.dirty = true;
                                }
                            }
                        }
                    }
                }
                if self.dirty {
                    if let Some(w) = &self.window {
                        w.request_redraw();
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if button != MouseButton::Left {
                    return;
                }
                match state {
                    ElementState::Pressed => {
                        if let Some(d) = self.last_dividers.iter().find(|d| hit_divider(d, self.mouse_pos)).copied() {
                            self.resizing_divider = Some(d.split);
                        } else if let Some(rect) = self.last_panes.iter().find(|r| hit_rect(r, self.mouse_pos)).copied() {
                            self.ui.splits.active = rect.pane;
                            if let Some(runtime) = self.panes.get_mut(&rect.pane) {
                                let cell = pixel_to_cell(self.mouse_pos, rect.x, rect.y, self.cell_w, self.cell_h, runtime.last_dims);
                                runtime.selection = Some(Selection { anchor: cell, cursor: cell });
                            }
                            self.selecting_pane = Some(rect.pane);
                            self.mouse_down = true;
                        }
                        self.dirty = true;
                    }
                    ElementState::Released => {
                        self.mouse_down = false;
                        self.resizing_divider = None;
                        if let Some(pane_id) = self.selecting_pane.take() {
                            if let Some(runtime) = self.panes.get(&pane_id) {
                                if let Some(sel) = runtime.selection {
                                    if sel.anchor == sel.cursor {
                                        // tek tık, seçim yok
                                    } else {
                                        let text = self.extract_selection_text(&runtime.session, &sel, runtime.last_dims);
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(text);
                                        }
                                    }
                                }
                            }
                            if let Some(runtime) = self.panes.get_mut(&pane_id) {
                                if runtime.selection.map(|s| s.anchor == s.cursor).unwrap_or(false) {
                                    runtime.selection = None;
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
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y.round() as i32,
                    MouseScrollDelta::PixelDelta(p) => (p.y / self.cell_h as f64).round() as i32,
                };
                if lines != 0 {
                    if let Some(rect) = self.last_panes.iter().find(|r| hit_rect(r, self.mouse_pos)).copied() {
                        if let Some(runtime) = self.panes.get_mut(&rect.pane) {
                            runtime.session.term.lock().scroll_display(Scroll::Delta(lines));
                            runtime.selection = None;
                        }
                    }
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
                self.relayout();
                let instances = self.build_frame();
                if let Some(gpu) = &mut self.gpu {
                    let bg = hex_to_rgba(&self.config.theme.background);
                    let clear = wgpu::Color { r: bg[0] as f64, g: bg[1] as f64, b: bg[2] as f64, a: self.config.theme.opacity as f64 };
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
        let mut dead_panes = Vec::new();

        for (pane_id, runtime) in self.panes.iter() {
            while let Ok(event) = runtime.session.events_rx.try_recv() {
                match event {
                    CoreEvent::Wakeup => need_redraw = true,
                    CoreEvent::ClipboardStore(text) => {
                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                            let _ = clipboard.set_text(text);
                        }
                    }
                    CoreEvent::TitleChanged(title) => {
                        if *pane_id == self.ui.splits.active {
                            if let Some(window) = &self.window {
                                window.set_title(&title);
                            }
                        }
                    }
                    CoreEvent::ChildExited => dead_panes.push(*pane_id),
                    CoreEvent::Other(_) => {}
                }
            }
        }

        for pane_id in dead_panes {
            if self.ui.splits.close_pane(pane_id).is_some() {
                self.panes.remove(&pane_id);
                need_redraw = true;
            } else {
                // Son pane kapandı: uygulamadan çık.
                event_loop.exit();
                return;
            }
        }

        if let Some(watcher) = &self.config_watcher {
            if let Some(new_config) = watcher.poll() {
                log::info!("Config yeniden yüklendi (hot reload)");
                self.config = new_config;
                need_redraw = true;
            }
        }

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

        let cursor_animating = self.panes.values().any(|r| r.cursor_anim.is_animating(self.config.cursor.animation_ms));
        let palette_animating = self.ui.palette_open
            && self.ui.palette_opened_at.map(|t| t.elapsed().as_millis() < 120).unwrap_or(false);
        if cursor_animating || palette_animating {
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

fn hit_rect(r: &lumenterm_ui::splits::PaneRect, pos: (f64, f64)) -> bool {
    let (x, y) = (pos.0 as f32, pos.1 as f32);
    x >= r.x && x <= r.x + r.w && y >= r.y && y <= r.y + r.h
}

fn hit_divider(d: &lumenterm_ui::splits::DividerRect, pos: (f64, f64)) -> bool {
    const TOLERANCE: f32 = 4.0;
    let (x, y) = (pos.0 as f32, pos.1 as f32);
    x >= d.x - TOLERANCE && x <= d.x + d.w + TOLERANCE && y >= d.y - TOLERANCE && y <= d.y + d.h + TOLERANCE
}

impl App {
    fn handle_palette_key(&mut self, event: &winit::event::KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => self.ui.palette_open = false,
            Key::Named(NamedKey::Enter) => {
                // Faz 4 kapsamında yalnızca birkaç yerleşik komut
                // uygulanıyor (tema değiştirme); geri kalanı (scrollback
                // satırına atlama vb.) ileride eklenebilir.
                let results = self.ui.palette.results(10);
                if let Some(label) = results.get(self.ui.palette.selected) {
                    self.execute_palette_command(label.clone());
                }
                self.ui.palette_open = false;
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.ui.palette.selected = (self.ui.palette.selected + 1).min(9);
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.ui.palette.selected = self.ui.palette.selected.saturating_sub(1);
            }
            Key::Named(NamedKey::Backspace) => {
                let mut q = self.ui.palette.query.clone();
                q.pop();
                self.ui.palette.set_query(&q);
            }
            Key::Character(s) => {
                let mut q = self.ui.palette.query.clone();
                q.push_str(s);
                self.ui.palette.set_query(&q);
            }
            _ => {}
        }
    }

    fn execute_palette_command(&mut self, label: String) {
        if let Some(theme_file) = label.strip_prefix("Temayı değiştir: ") {
            let path = lumenterm_config::Config::default_path()
                .parent()
                .map(|p| p.join("themes").join(format!("{theme_file}.toml")));
            log::info!("Tema değişimi istendi: {theme_file} (dosya: {path:?}) -- şu an yalnızca config.toml'ü elle düzenleyip hot-reload ile uygulayın");
        } else if label == "Yatay böl" {
            self.split_and_spawn(SplitDirection::Horizontal);
        } else if label == "Dikey böl" {
            self.split_and_spawn(SplitDirection::Vertical);
        } else if label == "Yeni sekme" {
            self.split_and_spawn(SplitDirection::Horizontal);
        }
        self.dirty = true;
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
