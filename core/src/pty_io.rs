use crate::blocks::BlockTracker;
use crate::event::{CoreEvent, EventProxy};
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::Processor;
use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread;

/// PTY'den sürekli okuyan ve okunan baytları `vte::Processor` üzerinden
/// `Term`'e besleyen arka plan thread'ini başlatır.
///
/// Bu fonksiyon render/UI thread'ini asla bloklamaz: `Term` kısa süreli
/// kilitlerle güncellenir, ağır iş (okuma + parse) tamamen bu thread'de
/// kalır. `renderer`, her frame'de yalnızca "dirty" olup olmadığına bakar
/// (bkz. `CoreEvent::Wakeup`).
pub fn spawn_reader<R>(
    term: Arc<Mutex<Term<EventProxy>>>,
    blocks: Arc<Mutex<BlockTracker>>,
    events_tx: Sender<CoreEvent>,
    mut read_fn: R,
) where
    R: FnMut(&mut [u8]) -> std::io::Result<usize> + Send + 'static,
{
    thread::Builder::new()
        .name("lumenterm-pty-reader".into())
        .spawn(move || {
            let mut processor = Processor::new();
            let mut buf = [0u8; 32 * 1024];

            loop {
                match read_fn(&mut buf) {
                    Ok(0) => {
                        let _ = events_tx.send(CoreEvent::ChildExited);
                        break;
                    }
                    Ok(n) => {
                        let mut term_guard = term.lock();
                        // OSC 133 (prompt start/end) sekanslarını burada
                        // ayrıca yakalamak için Processor'a verilen her
                        // baytı blocks tracker'a da besliyoruz; gerçek
                        // OSC ayrıştırması `blocks::scan_osc133` içinde.
                        blocks.lock().scan(&buf[..n]);
                        for byte in &buf[..n] {
                            processor.advance(&mut *term_guard, *byte);
                        }
                        drop(term_guard);
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
