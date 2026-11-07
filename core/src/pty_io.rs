use crate::blocks::BlockTracker;
use crate::event::{CoreEvent, EventProxy};
use crate::graphics::GraphicsStore;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;

/// PTY'den sürekli okuyan ve okunan baytları `vte::Processor` üzerinden
/// `Term`'e besleyen arka plan thread'ini başlatır. Aynı baytlar OSC 133
/// (blok sınırları) ve Kitty/Sixel grafik protokolü için de ayrıca
/// taranır (bkz. `blocks::BlockTracker`, `graphics::GraphicsStore`).
///
/// Bu fonksiyon render/UI thread'ini asla bloklamaz: `Term` kısa süreli
/// kilitlerle güncellenir, ağır iş (okuma + parse) tamamen bu thread'de
/// kalır. `renderer`, her frame'de yalnızca "dirty" olup olmadığına bakar
/// (bkz. `CoreEvent::Wakeup`).
pub fn spawn_reader<R>(
    term: Arc<Mutex<Term<EventProxy>>>,
    blocks: Arc<Mutex<BlockTracker>>,
    graphics: Arc<Mutex<GraphicsStore>>,
    cell_px: (f32, f32),
    events_tx: Sender<CoreEvent>,
    mut read_fn: R,
) where
    R: FnMut(&mut [u8]) -> std::io::Result<usize> + Send + 'static,
{
    thread::Builder::new()
        .name("lumenterm-pty-reader".into())
        .spawn(move || {
            // `vte` 0.13.1'den itibaren `Processor<T: Timeout = StdSyncHandler>`
            // bir generic parametre aldı. Varsayılan tip (`StdSyncHandler`)
            // yalnızca *tip pozisyonunda* otomatik uygulanıyor; `Processor::new()`
            // bir fonksiyon çağrısı olduğundan Rust bunu kendiliğinden çıkaramıyor
            // ("type annotations needed" hatası) -- bu yüzden burada açıkça
            // belirtiyoruz. `StdSyncHandler`, senkronize çıktı (DEC mode 2026)
            // zaman aşımını senkron/basit şekilde ele alan varsayılan handler'dır;
            // bizim kullanım şeklimiz için tam olarak doğru seçim.
            let mut processor = Processor::<StdSyncHandler>::new();
            let mut buf = [0u8; 32 * 1024];

            loop {
                match read_fn(&mut buf) {
                    Ok(0) => {
                        let _ = events_tx.send(CoreEvent::ChildExited);
                        break;
                    }
                    Ok(n) => {
                        let mut term_guard = term.lock();
                        for byte in &buf[..n] {
                            processor.advance(&mut *term_guard, *byte);
                        }
                        // İmlecin bu chunk sonrasındaki konumu: yeni
                        // görseller buraya yerleştirilir (tam byte-hassas
                        // değil ama pratikte yeterli bir yaklaşıklık).
                        let cursor_point = term_guard.grid().cursor.point;
                        let cursor = (cursor_point.line.0, cursor_point.column.0);
                        drop(term_guard);

                        blocks.lock().scan(&buf[..n], cursor.0);
                        graphics.lock().scan(&buf[..n], cursor, cell_px);

                        let _ = events_tx.send(CoreEvent::Wakeup);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        continue;
                    }
                    Err(_) => {
                        let _ = events_tx.send(CoreEvent::ChildExited);
                        break;
                    }
                }
            }
        })
        .expect("pty reader thread spawn edilemedi");
}
