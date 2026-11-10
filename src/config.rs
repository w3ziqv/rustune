use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs};

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
    pub equalizer: rustune_core::equalizer::EqSettings,
    pub performance: PerformanceMode,
    pub catalog_country: String,
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
            equalizer: rustune_core::equalizer::EqSettings::default(),
            performance: PerformanceMode::Balanced,
            catalog_country: default_catalog_country(),
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
        let mut c: Self = toml::Value::Table(value)
            .try_into()
            .context("invalid config TOML")?;
        c.equalizer.validate()?;
        ensure!(
            c.catalog_country.len() == 2
                && c.catalog_country.bytes().all(|b| b.is_ascii_alphabetic()),
            "catalog_country must be a two-letter country code"
        );
        c.catalog_country.make_ascii_uppercase();
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

pub use rustune_core::paths::{Paths, private_dir, private_write};

pub fn default_catalog_country() -> String {
    "PL".into()
}
