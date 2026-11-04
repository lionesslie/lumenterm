# LumenTerm

Rust + wgpu(Vulkan) tabanlı terminal emülatörü. Birincil hedef Linux
(X11 + Wayland); Windows desteği de eklendi (bkz. "Platform desteği").
"Katmanlı scrollback" kimliği: klasik kesintisiz scrollback + opsiyonel,
OSC 133 tabanlı katlanabilir komut blokları.

## Platform desteği

| Platform | GPU backend | PTY | Shell varsayılanı | Config yolu |
|---|---|---|---|---|
| Linux (X11/Wayland) | **Vulkan** (zorunlu, fallback yok) | Unix PTY | `$SHELL` → yoksa `/bin/bash` | `$XDG_CONFIG_HOME/lumenterm` → yoksa `~/.config/lumenterm` |
| Windows 10/11 | **Vulkan** (öncelikli) → bulunamazsa **DX12**'ye loglanan fallback | ConPTY (`portable-pty` üzerinden) | PATH'te varsa `pwsh.exe`, yoksa `powershell.exe`, o da yoksa `%COMSPEC%`/`cmd.exe` | `%APPDATA%\lumenterm` |

Windows'ta GPU backend kararı kasıtlı olarak Linux'takiyle aynı tutuldu:
proje Vulkan'ı birinci sınıf backend olarak hedefliyor ve çoğu modern
Windows makinesinde (NVIDIA/AMD/Intel sürücüleri) zaten bir Vulkan ICD'si
bulunuyor. DX12 yalnızca gerçekten Vulkan adaptörü bulunamazsa devreye
giriyor ve bu durum `log::warn!` ile açıkça bildiriliyor — sessiz bir
"otomatik en iyi backend" seçimi değil.

## Durum

Bu depo, geliştirme promptunda tanımlanan **Faz 1 + Faz 2**'yi içerir ve
derlenebilir niyetle yazılmıştır:

- Cargo workspace, mimari dokümanındaki modül ayrımına birebir uyar
  (`core`, `renderer`, `font`, `config`, `ui`, `app`).
- `app`: winit penceresi, `wgpu::Backends::VULKAN`'a kilitli GPU init,
  PTY üzerinden gerçek bir shell, `vte`/`alacritty_terminal` ile grid
  güncelleme, hücre arka planı + gerçek glyph'lerin aynı pipeline ile
  çizimi, yumuşak cursor animasyonu + blink, fare ile seçim/kopyalama
  (bırakınca panoya yazar), fare tekerleğiyle scrollback kaydırma,
  `about_to_wait` içinde damage-tracking (yalnızca `CoreEvent::Wakeup`,
  blink zamanı veya cursor animasyonu sürerken redraw; aksi halde
  `ControlFlow::Wait`).
- `font`: `cosmic-text` ile shaping + dahili `SwashCache` ile karakter
  başına rasterizasyon (grayscale AA maskesi veya renkli emoji RGBA'sı),
  fallback zinciri (emoji/CJK).
- `renderer`: `GlyphAtlas` — RGBA8 `wgpu::Texture` üzerinde basit bir
  "shelf" packer, her yeni karakteri bir kez paketler ve cache'ler;
  shader hem düz dolgu hem maske-glyph hem renkli-glyph çizebiliyor
  (`has_glyph`/`color_glyph` instance alanlarıyla dallanıyor).
- `config`: TOML config + `notify` ile hot-reload, 4 hazır tema
  (`config/themes/*.toml`).
- `core`: PTY spawn + arka plan okuma thread'i, `EventProxy`, OSC 133
  izleyen `BlockTracker` (kimlik özelliğinin temeli).
- `ui`: ikili split/pane ağacı (`splits.rs`) ve `nucleo` tabanlı komut
  paleti (`palette.rs`) — kısayol (Ctrl+Shift+P) `ui.palette_open`'ı
  değiştiriyor ama henüz GPU'da bir overlay olarak çizilmiyor.

## Henüz yapılmayanlar (sıradaki fazlar)

- **Faz 2 artıkları**: Komut paleti/scrollback arama overlay'inin
  gerçekten GPU'da çizilmesi (şu an yalnızca durum tutuluyor), geniş
  (wide/CJK) karakterlerin iki hücre kaplaması, subpixel AA seçeneği
  (şu an yalnızca grayscale).
- **Faz 4**: Kitty/Sixel grafik protokolü, komut paletinin arama
  sonuçlarının ekrana yansıması, split/pane UI'ının gerçek birden çok
  `Session` ile çalıştırılması (şu an tek pane/tek oturum).
- **Faz 5**: Blok katmanının UI'da katlama/genişletme olarak gösterimi,
  chrome cilası (köşe yarıçapı, sekme çubuğu çizimi, ek animasyonlar).

## Faz 2'de alınan tasarım kararları

- **Tek pipeline, iki mantıksal instance tipi**: Orijinal "tek draw call"
  hedefi korunuyor ama artık her hücre 1-2 instance üretebiliyor (arka
  plan + varsa glyph) — hepsi aynı `instance_buf`'a yazılıp **tek**
  `render_pass.draw()` çağrısıyla çiziliyor (bkz. `GpuState::render`).
- **Karakter başına rasterizasyon**: Tam paragraf shaping'i yerine her
  karakter ayrı rasterize ediliyor; monospace bir terminalde bu basit ve
  hızlı (glyph atlas'ta cache'lendiği için aynı karakter ikinci kez asla
  yeniden çizilmiyor). Ligature desteği gerekirse (örn. `=>`, `!=`)
  ileride çok-karakterli "cluster" tespiti eklenmesi gerekir.
- **Cursor, scrollback'teyken gizlenir**: `display_offset() > 0` iken
  (kullanıcı geçmişe kaydırdıysa) cursor çizilmiyor — xterm/Alacritty'nin
  de yaptığı gibi, gerçek imleç zaten görünür alanın dışında.
- **Seçim scroll'da temizlenir**: Basitlik için fare tekerleğiyle
  kaydırma, aktif seçimi iptal ediyor; scrollback içi seçim ileride
  mutlak grid koordinatlarına taşınarak eklenebilir.

## Build

> Bu ortamda ağ erişimi kapalı olduğundan `cargo build` bağımlılıkları
> indiremedi ve bu kod burada derlenerek doğrulanamadı. Kendi
> makinenizde (Vulkan sürücüleri + `libvulkan1`, `mesa-vulkan-drivers`
> veya üretici sürücüsü kurulu) şunu çalıştırın:

```bash
# Linux
cargo build --release
./target/release/lumenterm
```

```powershell
# Windows (MSVC toolchain: `rustup default stable-x86_64-pc-windows-msvc`,
# Visual Studio Build Tools kurulu olmalı — Rust'ın kendisi için gerekli,
# LumenTerm'in Vulkan/DX12 kullanımıyla ilgisi yok)
cargo build --release
.\target\release\lumenterm.exe
```

İlk derlemede bazı bağımlılıkların (özellikle `alacritty_terminal`,
`cosmic-text`, `wgpu`) API'leri sürüm güncellemeleriyle küçük farklar
gösterebilir; derleyici hatalarını türe göre (ör. `Term::new` imzası,
`vte::ansi::Processor` yolu) ilgili crate'in o anki `docs.rs` sayfasına
bakarak hizalamanız gerekebilir — mimari ve modül sınırları değişmeden
kalır.

## Shell entegrasyonu (OSC 133 / blok katmanı)

- **bash/zsh (Linux)**: `app/assets/shell-integration/{bash,zsh}.sh`
  dosyalarını `~/.bashrc` ya da `~/.zshrc` sonuna source edin.
- **PowerShell (Windows)**: `app/assets/shell-integration/powershell.ps1`
  dosyasını `$PROFILE`'ınıza dahil edin (bkz. dosyanın başındaki yorum).
  PowerShell'in bash/zsh'teki preexec/precmd hook çiftine birebir
  karşılığı olmadığından, komut başlangıcı `PSReadLine`'ın Enter
  handler'ı üzerinden yaklaşık olarak işaretlenir; `PSReadLine` yoksa
  (eski Windows PowerShell sürümleri) yalnızca prompt başlangıcı/bitişi
  işaretlenir, blok katmanı yine çalışır ama komut sınırı biraz daha az
  hassastır.

Bu entegrasyonlar her komutun başlangıç/bitiş sınırlarını arka planda
(görünmez) işaretler; kullanıcı hiçbir şey fark etmez, ama
`ui`/`core::BlockTracker` bu sınırları bilir ve istendiğinde (Faz 5'te)
bir kısayolla o komut+çıktısını katlar.

## Lisans

MIT OR Apache-2.0
