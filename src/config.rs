use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceMode {
    Minimal,
    #[default]
    Balanced,
    Enhanced,
}
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub cache_entries: usize,
    pub refresh_ms: u64,
    pub page_size: usize,
    pub artwork_entries: usize,
    pub artwork_pixels: u32,
}
impl PerformanceMode {
    pub fn limits(self) -> Limits {
        match self {
            Self::Minimal => Limits {
                cache_entries: 1,
                refresh_ms: 2000,
                page_size: 15,
                artwork_entries: 0,
                artwork_pixels: 0,
            },
            Self::Balanced => Limits {
                cache_entries: 8,
                refresh_ms: 1000,
                page_size: 25,
                artwork_entries: 4,
                artwork_pixels: 256,
            },
            Self::Enhanced => Limits {
                cache_entries: 32,
                refresh_ms: 500,
                page_size: 50,
                artwork_entries: 16,
                artwork_pixels: 512,
            },
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub performance: PerformanceMode,
    pub volume: f64,
    pub theme: String,
    pub daemon: bool,
    pub notifications: bool,
    pub mouse: bool,
    pub artwork: bool,
    pub lyrics: bool,
    pub artwork_protocol: String,
    pub logging: bool,
    pub restore_queue: bool,
    pub max_queue: usize,
    pub keybindings: BTreeMap<String, String>,
    pub colors: BTreeMap<String, String>,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            performance: PerformanceMode::Balanced,
            volume: 0.7,
            theme: "monochrome".into(),
            daemon: false,
            notifications: false,
            mouse: true,
            artwork: true,
            lyrics: true,
            artwork_protocol: "auto".into(),
            logging: false,
            restore_queue: true,
            max_queue: 10_000,
            keybindings: BTreeMap::new(),
            colors: BTreeMap::new(),
        }
    }
}
impl Config {
    pub fn parse(s: &str) -> Result<Self> {
        let mut value: toml::Table = toml::from_str(s).context("invalid config TOML")?;
        // Recognized retired settings are discarded; all other unknown keys stay errors.
        if let Some(backend) = value.remove("backend") {
            ensure!(
                backend.as_str().is_some_and(|name| [
                    "disabled",
                    "auto",
                    "firefox",
                    "apple_web",
                    "music_kit"
                ]
                .contains(&name)),
                "unknown retired backend setting"
            );
        }
        for key in ["browser", "bridge_port", "headless"] {
            value.remove(key);
        }
        let c: Self = toml::Value::Table(value)
            .try_into()
            .context("invalid config TOML")?;
        ensure!(
            c.volume.is_finite() && (0.0..=1.0).contains(&c.volume),
            "volume must be between 0 and 1"
        );
        ensure!(
            (1..=10_000).contains(&c.max_queue),
            "max_queue must be 1..10000"
        );
        ensure!(
            [
                "dark",
                "light",
                "catppuccin_mocha",
                "catppuccin_latte",
                "gruvbox",
                "nord",
                "monochrome",
                "tokyo_night",
                "tokio",
                "custom"
            ]
            .contains(&c.theme.as_str()),
            "unknown theme"
        );
        ensure!(
            ["auto", "halfblocks", "kitty", "iterm2"].contains(&c.artwork_protocol.as_str()),
            "unsupported artwork protocol; use auto, halfblocks, kitty or iterm2 (Sixel requires verified terminal pixel geometry)"
        );
        crate::keys::validate_bindings(&c.keybindings)?;
        for color in c.colors.values() {
            ensure!(
                color.len() == 7
                    && color.starts_with('#')
                    && color[1..].bytes().all(|b| b.is_ascii_hexdigit()),
                "custom colors must be #RRGGBB"
            );
        }
        Ok(c)
    }
    pub fn load(paths: &Paths) -> Result<Self> {
        let file = paths.config.join("config.toml");
        if !file.exists() {
            return Ok(Self::default());
        }
        ensure!(
            fs::metadata(&file)?.len() <= 64 * 1024,
            "config file exceeds 64 KiB"
        );
        let text = fs::read_to_string(&file)?;
        let config = Self::parse(&text)?;
        let value: toml::Table = toml::from_str(&text)?;
        if ["backend", "browser", "bridge_port", "headless"]
            .iter()
            .any(|key| value.contains_key(*key))
        {
            config.save(paths)?;
        }
        Ok(config)
    }
    pub fn save(&self, paths: &Paths) -> Result<()> {
        private_write(
            &paths.config.join("config.toml"),
            toml::to_string_pretty(self)?.as_bytes(),
        )
    }
}

#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
    pub runtime: PathBuf,
}
impl Paths {
    pub fn discover() -> Result<Self> {
        let home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
        let base = |key: &str, fallback: PathBuf| {
            std::env::var_os(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or(fallback)
        };
        let data = base("XDG_DATA_HOME", home.join(".local/share")).join("rustune");
        let runtime = base("XDG_RUNTIME_DIR", data.clone()).join("rustune");
        Ok(Self {
            config: base("XDG_CONFIG_HOME", home.join(".config")).join("rustune"),
            cache: base("XDG_CACHE_HOME", home.join(".cache")).join("rustune"),
            data,
            runtime,
        })
    }
    pub fn init(&self) -> Result<()> {
        for p in [&self.config, &self.data, &self.cache, &self.runtime] {
            private_dir(p)?;
        }
        Ok(())
    }
    pub fn socket(&self) -> PathBuf {
        self.runtime.join("service.sock")
    }
}

pub fn private_dir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    ensure!(
        !fs::symlink_metadata(p)?.file_type().is_symlink(),
        "private directory cannot be a symlink"
    );
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("file needs parent directory")?;
    private_dir(parent)?;
    let tmp = parent.join(format!(
        ".write-{}-{:016x}",
        std::process::id(),
        fastrand::u64(..)
    ));
    let result = (|| -> Result<()> {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
