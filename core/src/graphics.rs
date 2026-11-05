//! Resim gösterme: Kitty graphics protokolü (APC `ESC _ G ... ESC \`) ana
//! hedef, DEC Sixel (DCS `ESC P ... q ... ESC \`) fallback olarak.
//!
//! Bu modül yalnızca **protokolü çözer** (byte akışından piksel verisine);
//! GPU'ya yükleme/compositing `renderer`/`app` katmanındadır (bkz.
//! `renderer::atlas` -- görseller glyph atlas'ıyla aynı paketleme
//! mekanizmasını paylaşır).
//!
//! Kapsam dışı bırakılanlar (bilinçli, bir sonraki iterasyon için TODO):
//! - Kitty'nin PNG yükü (`f=100`): yalnızca ham RGB(`f=24`)/RGBA(`f=32`)
//!   destekleniyor; PNG decode bir `image`/`png` crate bağımlılığı
//!   gerektirir.
//! - Kitty'nin sorgu (`a=q`), silme (`a=d`), var olan görseli yeniden
//!   konumlandırma (`a=p`) aksiyonları: yalnızca "aktar ve hemen göster"
//!   (`a=T`, yoksa varsayılan) destekleniyor.
//! - Sixel içinde animasyon/çoklu frame.

use std::collections::HashMap;

pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Görselin terminal grid'indeki yerleşimi. `row`/`col` *mutlak* grid
/// satırıdır (alacritty'nin `Line` ile aynı uzayda, scrollback dahil);
/// `app` bunu ekrana çizerken `display_offset` ile ekran-uzayına çevirir
/// (tıpkı metin hücreleri gibi).
#[derive(Debug, Clone, Copy)]
pub struct ImagePlacement {
    pub image_id: u32,
    pub row: i32,
    pub col: usize,
    pub cols: usize,
    pub rows: usize,
}

#[derive(Default)]
pub struct GraphicsStore {
    pub images: HashMap<u32, DecodedImage>,
    pub placements: Vec<ImagePlacement>,
    next_sixel_id: u32,
    // Ham tarayıcı durumu.
    after_esc: bool,
    in_apc: bool,
    in_dcs: bool,
    pending_st: bool,
    buf: Vec<u8>,
    // Kitty çok parçalı transferler (m=1) için id -> biriken ham veri.
    partial: HashMap<u32, Vec<u8>>,
}

impl GraphicsStore {
    /// PTY'den gelen ham baytları tarar. `cursor`: APC/DCS tamamlandığı
    /// anda (bu chunk için) bilinen en güncel imleç konumu (satır, sütun)
    /// -- yeni görsel oraya yerleştirilir. `cell_px`: görselin kaç hücre
    /// kaplayacağını hesaplamak için hücre piksel boyutu. Gerçek tarama
    /// durum makinesi `scan_into` serbest fonksiyonundadır (bu metod ona
    /// delege eder); ayrım, ST-sonlanma dispatch'inin tek bir yerde,
    /// `in_apc`/`in_dcs` bayrakları henüz true iken yapılmasını sağlamak
    /// içindir.
    pub fn scan(&mut self, bytes: &[u8], cursor: (i32, usize), cell_px: (f32, f32)) {
        scan_into(self, bytes, cursor, cell_px);
    }

    fn handle_apc(&mut self, payload: &[u8], cursor: (i32, usize), cell_px: (f32, f32)) {
        if payload.first() != Some(&b'G') {
            return;
        }
        let text = String::from_utf8_lossy(&payload[1..]).into_owned();
        let (header, data_b64) = match text.find(';') {
            Some(idx) => (&text[..idx], &text[idx + 1..]),
            None => (text.as_str(), ""),
        };
        let mut kv: HashMap<&str, &str> = HashMap::new();
        for pair in header.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                kv.insert(k, v);
            }
        }
        let format = kv.get("f").and_then(|v| v.parse::<u32>().ok()).unwrap_or(32);
        let image_id = kv.get("i").and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
        let width = kv.get("s").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let height = kv.get("v").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
        let more = kv.get("m").map(|v| *v == "1").unwrap_or(false);

        let chunk = base64_decode(data_b64.as_bytes());
        self.partial.entry(image_id).or_default().extend_from_slice(&chunk);
        if more {
            return; // sonraki APC parçası bekleniyor
        }
        let raw = self.partial.remove(&image_id).unwrap_or_default();
        if width == 0 || height == 0 {
            return;
        }

        let rgba = match format {
            32 => {
                if raw.len() as u64 != (width as u64) * (height as u64) * 4 {
                    return;
                }
                raw
            }
            24 => rgb_to_rgba(&raw, width, height),
            100 => {
                log::warn!("Kitty PNG (f=100) formatı henüz desteklenmiyor, görsel atlanıyor");
                return;
            }
            other => {
                log::warn!("Bilinmeyen Kitty görsel formatı f={other}, atlanıyor");
                return;
            }
        };
        if rgba.is_empty() {
            return;
        }

        self.images.insert(image_id, DecodedImage { width, height, rgba });
        let cols = ((width as f32) / cell_px.0).ceil().max(1.0) as usize;
        let rows = ((height as f32) / cell_px.1).ceil().max(1.0) as usize;
        self.push_placement(ImagePlacement {
            image_id,
            row: cursor.0,
            col: cursor.1,
            cols,
            rows,
        });
    }

    /// Basit bir büyüme sınırı: gerçek bir LRU tahliye stratejisi yerine
    /// (ileride eklenebilir), en eski yerleşimi/görseli atarak sınırsız
    /// bellek büyümesini önler. Pratikte (ör. bir ilerleme çubuğunu
    /// tekrar tekrar basan bir araç) bu yeterlidir.
    fn push_placement(&mut self, placement: ImagePlacement) {
        const MAX_PLACEMENTS: usize = 64;
        self.placements.push(placement);
        if self.placements.len() > MAX_PLACEMENTS {
            self.placements.remove(0);
        }
        const MAX_IMAGES: usize = 128;
        if self.images.len() > MAX_IMAGES {
            if let Some(&oldest_id) = self.images.keys().next() {
                self.images.remove(&oldest_id);
            }
        }
    }

    fn handle_dcs(&mut self, payload: &[u8], cursor: (i32, usize), cell_px: (f32, f32)) {
        // DCS girişi genelde "Pan;Pad;Ph;Pv q<sixel-data>" biçimindedir;
        // yalnızca 'q' ile başlayan gövdeyle ilgileniyoruz.
        let Some(q_pos) = payload.iter().position(|&b| b == b'q') else { return };
        let body = &payload[q_pos + 1..];
        let Some(image) = crate::sixel::decode(body) else { return };

        self.next_sixel_id += 1;
        let image_id = 0x5349_0000 | (self.next_sixel_id & 0xffff); // sixel id'lerini kitty id uzayından ayır
        let cols = ((image.width as f32) / cell_px.0).ceil().max(1.0) as usize;
        let rows = ((image.height as f32) / cell_px.1).ceil().max(1.0) as usize;
        self.images.insert(image_id, DecodedImage { width: image.width, height: image.height, rgba: image.rgba });
        self.push_placement(ImagePlacement {
            image_id,
            row: cursor.0,
            col: cursor.1,
            cols,
            rows,
        });
    }
}

/// `scan`'in basitleştirilmiş, doğru ST-dispatch mantığına sahip asıl
/// implementasyonu. Yukarıdaki `scan` metodu yerine bu kullanılır --
/// bkz. `pty_io.rs`'teki çağrı.
pub fn scan_into(store: &mut GraphicsStore, bytes: &[u8], cursor: (i32, usize), cell_px: (f32, f32)) {
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if store.in_apc || store.in_dcs {
            if store.pending_st && b == b'\\' {
                store.pending_st = false;
                let was_apc = store.in_apc;
                store.in_apc = false;
                store.in_dcs = false;
                let payload = std::mem::take(&mut store.buf);
                if was_apc {
                    store.handle_apc(&payload, cursor, cell_px);
                } else {
                    store.handle_dcs(&payload, cursor, cell_px);
                }
                i += 1;
                continue;
            }
            if store.pending_st {
                // ESC'den sonra '\' gelmedi: ESC'i veriye geri koy, devam et.
                store.buf.push(0x1b);
                store.pending_st = false;
            }
            if b == 0x1b {
                store.pending_st = true;
            } else {
                store.buf.push(b);
            }
            i += 1;
            continue;
        }

        if store.after_esc {
            store.after_esc = false;
            match b {
                b'_' => {
                    store.in_apc = true;
                    store.buf.clear();
                }
                b'P' => {
                    store.in_dcs = true;
                    store.buf.clear();
                }
                _ => {}
            }
            i += 1;
            continue;
        }

        if b == 0x1b {
            store.after_esc = true;
        }
        i += 1;
    }
}

fn rgb_to_rgba(rgb: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize) * (height as usize) * 3;
    if rgb.len() != expected {
        return Vec::new();
    }
    let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for px in rgb.chunks_exact(3) {
        out.extend_from_slice(&[px[0], px[1], px[2], 255]);
    }
    out
}

/// Harici bir bağımlılık eklememek için elle yazılmış standart base64
/// çözücü (RFC 4648, dolgu `=` destekli).
fn base64_decode(input: &[u8]) -> Vec<u8> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0u32;
    for &c in input {
        if c == b'=' || c == b'\n' || c == b'\r' {
            continue;
        }
        let Some(v) = val(c) else { continue };
        buf = (buf << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    out
}
