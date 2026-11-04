use alacritty_terminal::event::{Event as AlacEvent, EventListener};
use crossbeam_channel::Sender;

/// `app`/`renderer` katmanlarına taşınan, `alacritty_terminal`'in kendi
/// `Event` tipinden bağımsızlaştırılmış olaylar. Böylece üst katmanlar
/// alacritty'nin iç tipine doğrudan bağlı kalmaz.
#[derive(Debug, Clone)]
pub enum CoreEvent {
    /// Grid değişti, bir sonraki frame'de yeniden çizilmeli.
    Wakeup,
    /// OSC 52 ile clipboard'a yazılacak metin.
    ClipboardStore(String),
    /// Pencere başlığı değişti (OSC 0/2).
    TitleChanged(String),
    /// PTY/çocuk süreç sonlandı.
    ChildExited,
    /// Ham alacritty event'i (ileride ihtiyaç halinde kullanılabilsin diye
    /// debug amaçlı taşınıyor).
    Other(String),
}

/// `alacritty_terminal::Term` bir `EventListener` ister; biz bunu bir
/// mpsc kanalına köprüleyerek render thread'inin `Term` içini kilitlemeden
/// olayları tüketmesini sağlıyoruz.
#[derive(Clone)]
pub struct EventProxy {
    tx: Sender<CoreEvent>,
}

impl EventProxy {
    pub fn new(tx: Sender<CoreEvent>) -> Self {
        Self { tx }
    }
}

impl EventListener for EventProxy {
    fn send_event(&self, event: AlacEvent) {
        let mapped = match event {
            AlacEvent::Wakeup => CoreEvent::Wakeup,
            AlacEvent::ClipboardStore(_, text) => CoreEvent::ClipboardStore(text),
            AlacEvent::Title(title) => CoreEvent::TitleChanged(title),
            AlacEvent::ChildExit(_) => CoreEvent::ChildExited,
            other => CoreEvent::Other(format!("{other:?}")),
        };
        let _ = self.tx.send(mapped);
    }
}
