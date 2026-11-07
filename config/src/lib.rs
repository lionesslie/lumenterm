//! lumenterm-config
//!
//! TOML tabanlı config + `notify` ile hot-reload. `app` katmanı
//! `ConfigWatcher::poll()` ile her event loop turunda (bloklamadan)
//! değişiklik olup olmadığını kontrol eder; değişiklik varsa yeni
//! `Config` üretilir ve `renderer`/`ui` buna göre güncellenir (pencere
//! kapanmaz, GPU kaynakları yeniden kurulmaz — yalnızca renk/font/opaklık
//! uniform'ları değişir).

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub cursor: CursorConfig,
    #[serde(default)]
    pub blocks: BlocksConfig,
    #[serde(default)]
    pub animations: AnimationsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
    pub fallback: Vec<String>,
    /// "grayscale" | "subpixel" | "none"
    pub antialiasing: String,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".into(),
            size: 14.0,
            fallback: vec!["Noto Color Emoji".into(), "Noto Sans CJK SC".into()],
            antialiasing: "grayscale".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub cursor: String,
    pub opacity: f32,
    /// ANSI 0-15 renkleri, hex string olarak.
    pub palette: [String; 16],
}

impl Default for ThemeConfig {
    fn default() -> Self {
        // "Lumen Dark": koyu, yüksek kontrastlı varsayılan tema.
        Self {
            name: "lumen-dark".into(),
            background: "#000000".into(),
            foreground: "#ffffff".into(),
            cursor: "#ffffff".into(),
            opacity: 0.98,
            palette: [
                "#15171e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff",
                "#c0caf5", "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7",
                "#7dcfff", "#c0caf5",
            ]
            .map(String::from),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub padding: u32,
    pub corner_radius: f32,
    /// Pencerenin native "top bar"ını (başlık çubuğu + kapat/küçült/
    /// büyüt düğmeleri) winit üzerinden aç/kapat. `true` ⇒ sistem
    /// dekorasyonu görünür (bir DE/DM altında -- GNOME, KDE, Windows
    /// masaüstü vb. -- bu düğmelere ihtiyaç duyan kullanıcılar için).
    /// `false` (varsayılan) ⇒ dekorasyon yok, tamamen sade bir pencere
    /// (bir WM -- i3, sway, bspwm vb. -- altında çalışan ve pencere
    /// kontrolünü zaten kendi kısayollarıyla yapan kullanıcılar için;
    /// bu kullanıcılar bu düğmelere ihtiyaç duymaz). `lumenterm.toml`'da
    /// değiştirip kaydettiğinizde pencereyi kapatmadan anında uygulanır
    /// (hot-reload, bkz. `app`'teki `set_decorations` çağrısı).
    pub decorations: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            padding: 8,
            corner_radius: 10.0,
            decorations: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CursorConfig {
    /// "block" | "beam" | "underline"
    pub style: String,
    pub blink: bool,
    pub blink_interval_ms: u64,
    /// Yumuşak geçiş/animasyon süresi (ms). 0 = animasyonsuz.
    pub animation_ms: u64,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            style: "block".into(),
            blink: true,
            blink_interval_ms: 600,
            animation_ms: 90,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BlocksConfig {
    /// Kimlik özelliği: opsiyonel komut blok katmanı. `true` (varsayılan)
    /// iken OSC 133 işaretleme arka planda pasif çalışır ve kısayolla
    /// katlama kullanılabilir hale gelir; kullanıcı hiçbir şeyi
    /// katlamadığı sürece görünüşte hiçbir fark yaratmaz.
    pub enabled: bool,
    pub fold_shortcut: String,
}

impl Default for BlocksConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fold_shortcut: "ctrl+shift+return".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimationsConfig {
    /// Tüm "hareket" animasyonlarını (cursor'ın hücreden hücreye yumuşak
    /// kayması, komut paletinin fade/slide-in açılışı) tek yerden
    /// aç/kapat. `false` yapıldığında bu animasyonlar anında (0ms) biter
    /// -- cursor doğrudan hedef hücrede belirir, palet anında açılır.
    /// Cursor'ın yanıp sönmesi (`cursor.blink`) ayrı bir ayardır ve bu
    /// bayraktan etkilenmez, çünkü o bir hareket animasyonu değil,
    /// görünürlük durumudur.
    pub enabled: bool,
}

impl Default for AnimationsConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            theme: ThemeConfig::default(),
            window: WindowConfig::default(),
            cursor: CursorConfig::default(),
            blocks: BlocksConfig::default(),
            animations: AnimationsConfig::default(),
        }
    }
}

/// Hazır temaların TOML içeriği, derleme zamanında binary'ye gömülür
/// (`include_str!`). Böylece kurulum yöntemi ne olursa olsun (cargo
/// install, elle `cp target/release/lumenterm`, farklı bir dizin...)
/// temalar her zaman mevcuttur -- çalışma zamanında depodaki
/// `config/themes/*.toml` dosyalarının var olmasına bağımlı değiliz.
const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("lumen-dark", include_str!("../themes/lumen-dark.toml")),
    ("lumen-light", include_str!("../themes/lumen-light.toml")),
    ("sunset", include_str!("../themes/sunset.toml")),
    ("glacier", include_str!("../themes/glacier.toml")),
];

/// İlk çalıştırmada (ya da eksikse) hazır temaları kullanıcının config
/// dizinine (`~/.config/lumenterm/themes/`, Windows'ta `%APPDATA%\lumenterm\themes`)
/// yazar. Zaten mevcut bir dosyanın üzerine yazmaz -- kullanıcı bir
/// hazır temayı elle değiştirmişse bunu korur.
pub fn ensure_builtin_themes() -> std::io::Result<()> {
    let Some(dir) = Config::default_path().parent().map(|p| p.join("themes")) else {
        return Ok(());
    };
    std::fs::create_dir_all(&dir)?;
    for (name, content) in BUILTIN_THEMES {
        let path = dir.join(format!("{name}.toml"));
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
    }
    Ok(())
}

/// `~/.config/lumenterm/themes/<name>.toml` dosyasını okuyup yalnızca
/// `[theme]` tablosunu döner. Kullanıcı kendi özel temasını da aynı
/// dizine aynı biçimde (`[theme]` tablosu içeren bir `.toml`) koyabilir.
pub fn load_theme(name: &str) -> anyhow::Result<ThemeConfig> {
    let dir = Config::default_path()
        .parent()
        .ok_or_else(|| anyhow::anyhow!("config dizini belirlenemedi"))?
        .to_path_buf();
    let path = dir.join("themes").join(format!("{name}.toml"));
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("tema dosyası okunamadı ({}): {e}", path.display()))?;

    #[derive(Deserialize)]
    struct ThemeFile {
        theme: ThemeConfig,
    }
    let parsed: ThemeFile = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("tema dosyası ayrıştırılamadı ({}): {e}", path.display()))?;
    Ok(parsed.theme)
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if !path.exists() {
            let default = Config::default();
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, toml::to_string_pretty(&default)?)?;
            return Ok(default);
        }
        let raw = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&raw)?)
    }

    pub fn default_path() -> PathBuf {
        dirs_config_path().join("lumenterm").join("lumenterm.toml")
    }
}

#[cfg(target_os = "windows")]
fn dirs_config_path() -> PathBuf {
    // Windows geleneği: %APPDATA%\lumenterm\lumenterm.toml (Roaming).
    std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\".into());
        PathBuf::from(home).join("AppData").join("Roaming")
    })
}

#[cfg(not(target_os = "windows"))]
fn dirs_config_path() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
            PathBuf::from(home).join(".config")
        })
}

/// Dosya değişimini `notify` ile izler; `poll()` bloklamadan son olayı
/// döndürür (varsa).
pub struct ConfigWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<notify::Result<notify::Event>>,
    path: PathBuf,
}

impl ConfigWatcher {
    pub fn new(path: PathBuf) -> anyhow::Result<Self> {
        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?;
        if let Some(parent) = path.parent() {
            watcher.watch(parent, RecursiveMode::NonRecursive)?;
        }
        Ok(Self {
            _watcher: watcher,
            rx,
            path,
        })
    }

    /// Değişiklik varsa yeniden yüklenmiş `Config`'i döndürür.
    pub fn poll(&self) -> Option<Config> {
        let mut changed = false;
        while let Ok(res) = self.rx.try_recv() {
            if let Ok(event) = res {
                if event.paths.iter().any(|p| p == &self.path) {
                    changed = true;
                }
            }
        }
        if changed {
            Config::load(&self.path).ok()
        } else {
            None
        }
    }
}
