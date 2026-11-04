//! Klavye olaylarını terminale yazılacak bayt dizilerine çevirir.
//! Faz 2: temel ASCII + kontrol tuşları + Ctrl/Alt modifier kombinasyonları.
//! Kitty keyboard protocol / modifyOtherKeys gibi gelişmiş modlar ileride
//! (Faz 4+) eklenebilir; şu an xterm'in "legacy" moduna karşılık gelir.

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

pub fn key_to_bytes(event: &KeyEvent, mods: ModifiersState) -> Option<Vec<u8>> {
    // Ctrl+<harf> -> kontrol karakteri (Ctrl+A = 0x01 .. Ctrl+Z = 0x1a).
    if mods.control_key() {
        if let Key::Character(s) = &event.logical_key {
            let ch = s.chars().next()?;
            let upper = ch.to_ascii_uppercase();
            if upper.is_ascii_alphabetic() {
                let byte = (upper as u8) - b'A' + 1;
                return Some(vec![byte]);
            }
        }
    }

    let base = match &event.logical_key {
        Key::Character(s) => Some(s.as_bytes().to_vec()),
        Key::Named(NamedKey::Enter) => Some(vec![b'\r']),
        Key::Named(NamedKey::Backspace) => Some(vec![0x7f]),
        Key::Named(NamedKey::Tab) => Some(vec![b'\t']),
        Key::Named(NamedKey::Escape) => Some(vec![0x1b]),
        Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".to_vec()),
        Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".to_vec()),
        Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".to_vec()),
        Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".to_vec()),
        Key::Named(NamedKey::Home) => Some(b"\x1b[H".to_vec()),
        Key::Named(NamedKey::End) => Some(b"\x1b[F".to_vec()),
        Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".to_vec()),
        Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".to_vec()),
        Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".to_vec()),
        Key::Named(NamedKey::Space) => Some(vec![b' ']),
        _ => None,
    }?;

    // Alt+<tuş> -> ESC öneki (xterm metaSendsEscape davranışı).
    if mods.alt_key() {
        let mut out = vec![0x1b];
        out.extend(base);
        return Some(out);
    }

    Some(base)
}

pub fn ctrl_shift_held(mods: ModifiersState) -> bool {
    mods.control_key() && mods.shift_key()
}
