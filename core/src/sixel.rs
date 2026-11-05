//! Minimal bir DEC Sixel çözücü. Sixel verisi, her biri 6 dikey pikseli
//! temsil eden karakterlerden (0x3F-0x7E, değer = karakter - 0x3F, bit N
//! = o sütunun N. satırı boyalı) oluşur; `$` satır başı, `-` yeni satır
//! (6 piksel aşağı), `#` renk kaydı tanımlar/seçer, `!Pn<char>` tekrar
//! sayısıdır. `"` ile başlayan raster attribute (en-boy oranı/boyut)
//! parametreleri şimdilik yalnızca atlanıyor -- boyut, çizilen piksellerin
//! kapladığı alandan dinamik olarak hesaplanıyor.
//!
//! Kapsam dışı bırakılanlar (bilinçli, ileride eklenebilir): sixel
//! paylete/registere dayalı "GRI" grid komutları, animasyon (çoklu
//! frame), gerçek VT340 varsayılan 16 renkli paleti (burada yalnızca
//! açıkça `#` ile tanımlanan renkler kullanılıyor; tanımlanmamış bir
//! register için düz beyaz varsayılıyor).

pub struct SixelImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub fn decode(data: &[u8]) -> Option<SixelImage> {
    let mut colors: std::collections::HashMap<i64, [u8; 4]> = std::collections::HashMap::new();
    colors.insert(0, [0, 0, 0, 0]); // register 0: tanımlanmamış/arka plan -> saydam

    let mut x: i64 = 0;
    let mut y: i64 = 0;
    let mut current_color: i64 = 0;
    let mut max_x: i64 = 0;
    let mut max_y: i64 = 0;
    let mut pixels: Vec<(i64, i64, i64)> = Vec::new();

    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        match b {
            b'#' => {
                i += 1;
                let (num, len) = read_number(&data[i..]);
                i += len;
                let mut params = vec![num.unwrap_or(0)];
                while i < data.len() && data[i] == b';' {
                    i += 1;
                    let (n, l) = read_number(&data[i..]);
                    params.push(n.unwrap_or(0));
                    i += l;
                }
                let reg = params[0];
                if params.len() >= 5 {
                    let (sys, p1, p2, p3) = (params[1], params[2], params[3], params[4]);
                    let rgb = if sys == 1 {
                        hls_to_rgb(p1, p2, p3)
                    } else {
                        [
                            (p1.clamp(0, 100) * 255 / 100) as u8,
                            (p2.clamp(0, 100) * 255 / 100) as u8,
                            (p3.clamp(0, 100) * 255 / 100) as u8,
                        ]
                    };
                    colors.insert(reg, [rgb[0], rgb[1], rgb[2], 255]);
                }
                current_color = reg;
            }
            b'!' => {
                i += 1;
                let (num, len) = read_number(&data[i..]);
                i += len;
                let count = num.unwrap_or(1).max(1).min(100_000);
                if i < data.len() {
                    let ch = data[i];
                    i += 1;
                    if (0x3F..=0x7E).contains(&ch) {
                        let bits = ch - 0x3F;
                        for _ in 0..count {
                            emit_column(bits, x, y, current_color, &mut pixels);
                            x += 1;
                        }
                        max_x = max_x.max(x);
                        max_y = max_y.max(y + 6);
                    }
                }
            }
            b'$' => {
                x = 0;
                i += 1;
            }
            b'-' => {
                x = 0;
                y += 6;
                i += 1;
            }
            b'"' => {
                i += 1;
                while i < data.len() && (data[i].is_ascii_digit() || data[i] == b';') {
                    i += 1;
                }
            }
            0x3F..=0x7E => {
                let bits = b - 0x3F;
                emit_column(bits, x, y, current_color, &mut pixels);
                x += 1;
                max_x = max_x.max(x);
                max_y = max_y.max(y + 6);
                i += 1;
            }
            _ => i += 1,
        }
    }

    if max_x <= 0 || max_y <= 0 || pixels.is_empty() {
        return None;
    }
    let width = max_x as u32;
    let height = max_y as u32;
    // Aşırı büyük/bozuk veriden kaynaklı bellek patlamasına karşı üst sınır.
    if (width as u64) * (height as u64) > 64_000_000 {
        return None;
    }

    let mut rgba = vec![0u8; (width as usize) * (height as usize) * 4];
    for (px, py, reg) in pixels {
        if px < 0 || py < 0 {
            continue;
        }
        let (px, py) = (px as u32, py as u32);
        if px >= width || py >= height {
            continue;
        }
        let c = colors.get(&reg).copied().unwrap_or([255, 255, 255, 255]);
        let idx = ((py * width + px) * 4) as usize;
        rgba[idx..idx + 4].copy_from_slice(&c);
    }

    Some(SixelImage { width, height, rgba })
}

fn emit_column(bits: u8, x: i64, y: i64, color: i64, pixels: &mut Vec<(i64, i64, i64)>) {
    for bit in 0..6u8 {
        if bits & (1 << bit) != 0 {
            pixels.push((x, y + bit as i64, color));
        }
    }
}

fn read_number(bytes: &[u8]) -> (Option<i64>, usize) {
    let mut n: i64 = 0;
    let mut len = 0;
    let mut any = false;
    for &b in bytes {
        if b.is_ascii_digit() {
            n = n * 10 + (b - b'0') as i64;
            len += 1;
            any = true;
        } else {
            break;
        }
    }
    (if any { Some(n) } else { None }, len)
}

fn hls_to_rgb(h: i64, l: i64, s: i64) -> [u8; 3] {
    let h = (h as f32) / 360.0;
    let l = (l as f32) / 100.0;
    let s = (s as f32) / 100.0;
    if s == 0.0 {
        let v = (l * 255.0) as u8;
        return [v, v, v];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let r = hue_to_rgb(p, q, h + 1.0 / 3.0);
    let g = hue_to_rgb(p, q, h);
    let bch = hue_to_rgb(p, q, h - 1.0 / 3.0);
    [(r * 255.0) as u8, (g * 255.0) as u8, (bch * 255.0) as u8]
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 1.0 / 2.0 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}
