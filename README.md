# LumenTerm

Rust ve wgpu ile yazılmış, GPU hızlandırmalı bir terminal emülatörü. Linux
(X11/Wayland) ve Windows üzerinde çalışır.

## Özellikler

- wgpu tabanlı çizim (Linux'ta Vulkan; Windows'ta Vulkan, yoksa DX12)
- Glyph atlası ile tek draw call'da metin ve arka plan çizimi
- Yatay/dikey split pane'ler, her pane için ayrı PTY oturumu
- Fuzzy aramalı komut paleti
- Kitty graphics protokolü ve Sixel desteği
- OSC 133 tabanlı komut blokları ve katlanabilir scrollback
- TOML yapılandırma, canlı yeniden yükleme ve hazır temalar
- Fare ile seçim/kopyalama, tekerlekle scrollback

## Platform desteği

| Platform | GPU backend | PTY | Varsayılan shell |
|---|---|---|---|
| Linux | Vulkan (zorunlu) | Unix PTY | `$SHELL` veya `/bin/bash` |
| Windows 10/11 | Vulkan, yoksa DX12 | ConPTY | `pwsh.exe`, `powershell.exe` veya `cmd.exe` |

## Gereksinimler

- Rust 1.78 veya üzeri
- Çalışan bir Vulkan sürücüsü
- Windows: MSVC toolchain ve Visual Studio Build Tools
- Linux: X11 veya Wayland oturumu

## Derleme

```bash
cargo build --release
```

```bash
# Linux
./target/release/lumenterm

# Windows
.\target\release\lumenterm.exe
```

## Yapılandırma

Yapılandırma dosyası ilk çalıştırmada otomatik oluşturulur:

- Linux: `$XDG_CONFIG_HOME/lumenterm/lumenterm.toml` veya `~/.config/lumenterm/lumenterm.toml`
- Windows: `%APPDATA%\lumenterm\lumenterm.toml`

Dosya kaydedildiğinde değişiklikler uygulama kapatılmadan uygulanır. Örnek
yapılandırma için [`lumenterm.toml.example`](lumenterm.toml.example)
dosyasına, hazır temalar için `config/themes/` klasörüne bakabilirsiniz.

## Kısayollar

| Kısayol | Eylem |
|---|---|
| `Ctrl+Shift+P` | Komut paletini aç/kapat |
| `Ctrl+Shift+O` | Aktif pane'i yatay böl |
| `Ctrl+Shift+U` | Aktif pane'i dikey böl |
| `Ctrl+Shift+W` | Aktif pane'i kapat |
| `Alt+Tab` | Pane'ler arasında geçiş |
| `Ctrl+Shift+Enter` | Fare altındaki komut bloğunu katla/genişlet |
| Sürükleme | Metin seç (bırakınca kopyalanır) |
| Tekerlek | Scrollback'te kaydır |

## Shell entegrasyonu

Komut bloklarının doğru çalışması için ilgili betiği shell yapılandırmanıza
ekleyin:

- bash: `app/assets/shell-integration/bash.sh`
- zsh: `app/assets/shell-integration/zsh.sh`
- PowerShell: `app/assets/shell-integration/powershell.ps1`

## Proje yapısı

| Crate | Görev |
|---|---|
| `core` | PTY, terminal durumu, blok takibi, grafik protokolleri |
| `renderer` | wgpu pipeline ve glyph atlası |
| `font` | Shaping ve rasterizasyon |
| `config` | TOML yapılandırma ve temalar |
| `ui` | Split ağacı ve komut paleti |
| `app` | Pencere, girdi işleme ve ana döngü |

## Lisans

MIT OR Apache-2.0
