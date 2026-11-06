# LumenTerm

Rust ve wgpu/Vulkan üzerine kurulu, modern ve sade görünümlü bir terminal emülatörü. Hedef platform Linux (X11 + Wayland); Windows desteği de var.

LumenTerm, Alacritty/Kitty/WezTerm'in bir kopyası değil. Kendi kimliği **katmanlı scrollback**: varsayılan olarak klasik, kesintisiz bir scrollback akışı — hiçbir blok zorunluluğu yok. Ama her komutun sınırları arka planda sessizce işaretleniyor, isteyince (`Ctrl+Shift+Enter`) herhangi bir komut + çıktısını tek satıra katlayabiliyorsunuz. Warp'ın zorunlu blok modeli değil, opsiyonel ve hafif bir katman.

## Özellikler

- **GPU render** — wgpu üzerinden Vulkan (Windows'ta DX12'ye loglanan fallback), instanced hücre çizimi, damage tracking ile boşta ~0 CPU/GPU kullanımı
- **Tam VT100/xterm uyumluluğu** — 256 renk + true color, scrollback, fare ile seçim/kopyalama, OSC 52 clipboard
- **Gerçek glyph render'ı** — cosmic-text + swash ile shaping/rasterizasyon, grayscale AA, emoji/CJK fallback zinciri
- **Canlı config reload** — tema, font, opaklık; pencereyi kapatmadan anında uygulanır
- **Kitty graphics protokolü + Sixel fallback** — görseller terminal metniyle aynı frame'de GPU'da composite edilir
- **Komut paleti / fuzzy arama** — `Ctrl+Shift+P`, GPU'da çizilen overlay
- **Split/pane** — ikili bölme ağacı, sürüklenebilir ayraçlar, gerçek ayrı oturumlar
- **Katlanabilir komut blokları** — OSC 133 shell entegrasyonu, `Ctrl+Shift+Enter` ile katla/genişlet
- **Yumuşak chrome** — yuvarlatılmış köşeler, cursor animasyonu, komut paletinde fade-in

## Kurulum / Derleme

```bash
# Linux
cargo build --release
./target/release/lumenterm
```

```powershell
# Windows (MSVC toolchain)
cargo build --release
.\target\release\lumenterm.exe
```

> **Bilinen bir sürüm uyuşmazlığı var:** `vte` 0.13.1, `alacritty_terminal 0.24.2`'nin beklediği API'yi kırdı. İlk derlemede `E0107: missing generics for struct Processor` hatası alırsanız:
> ```bash
> rm -f Cargo.lock
> cargo update -p vte --precise 0.13.0
> cargo build --release
> ```

## Kısayollar

| Kısayol | Eylem |
|---|---|
| `Ctrl+Shift+P` | Komut paleti / fuzzy arama |
| `Ctrl+Shift+O` | Pane'i yatay böl |
| `Ctrl+Shift+U` | Pane'i dikey böl |
| `Ctrl+Shift+W` | Aktif pane'i kapat |
| `Alt+Tab` | Pane'ler arası geçiş |
| `Ctrl+Shift+Enter` | Fare altındaki komut bloğunu katla/genişlet |
| Fare sürükleme (ayraç) | Pane boyutlandır |
| Fare sol tık + sürükle | Seç (bırakınca kopyalanır) |
| Fare tekerleği | Scrollback'te kaydır |

## Yapılandırma

İlk açılışta `~/.config/lumenterm/lumenterm.toml` (Windows: `%APPDATA%\lumenterm`) otomatik oluşturulur ve dosya değişikliğinde anında yeniden yüklenir. Örnek:

```toml
[theme]
background = "#000000"
foreground = "#ffffff"
opacity = 0.98

[window]
# true: native başlık çubuğu + kapat/küçült düğmeleri (DE/DM kullanıcıları).
# false: dekorasyonsuz, sade pencere (WM kullanıcıları — i3, sway, bspwm...).
decorations = false

[blocks]
enabled = true
```

Dört hazır tema `config/themes/` altında: `lumen-dark` (varsayılan), `lumen-light`, `sunset`, `glacier`.

Blok katmanının çalışması için shell entegrasyonu gerekir — `app/assets/shell-integration/` altında bash, zsh ve PowerShell için hazır script'ler var; kendi `rc`/`$PROFILE` dosyanıza source edin.

## Mimari

Cargo workspace, her biri tek bir sorumluluğa sahip crate'lere ayrılmış:

```
core/      PTY yönetimi, VTE/grid durumu, OSC 133 blok takibi, Kitty/Sixel çözücü
renderer/  wgpu/Vulkan init, glyph+görsel atlas, instanced render pipeline
font/      cosmic-text/swash entegrasyonu, fallback zinciri
config/    TOML şema, hot-reload, temalar
ui/        split/pane ağacı, komut paleti
app/       winit event loop, girdi haritalama, her şeyi birleştirir
```

## Durum

Erken aşama. Mimari ve render hattı uçtan uca yazıldı, ama ağ erişimi olmayan bir ortamda geliştirildiği için henüz gerçek bir derleyiciden geçmedi — ilk derlemede küçük hatalarla karşılaşmanız olası. Katmanlı scrollback şu an mevcut görünümü sıkıştırıyor, henüz ek geçmişi pencereye çekmiyor; Kitty'nin PNG yükü ve bazı ileri seviye aksiyonları henüz desteklenmiyor.

## Lisans

MIT OR Apache-2.0
