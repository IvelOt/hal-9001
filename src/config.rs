use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub ui: UiConfig,
    pub theme: ThemeConfig,
    pub polling: PollingConfig,
    pub splash: SplashConfig,
    pub overview: OverviewConfig,
}

use crate::i18n::Language;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub frame_ms: u64,

    pub icons: bool,

    pub language: String,
}

impl UiConfig {
    pub fn resolved_language(&self) -> Language {
        if self.language.trim().eq_ignore_ascii_case("auto") || self.language.trim().is_empty() {
            Language::detect()
        } else {
            Language::parse(&self.language).unwrap_or_else(Language::detect)
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            frame_ms: 33,
            icons: true,
            language: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeConfig {
    pub name: String,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            name: "hal".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PollingConfig {
    pub system_ms: u64,
    pub network_ms: u64,
    pub bluetooth_ms: u64,
    pub audio_ms: u64,
    pub display_ms: u64,
    pub power_ms: u64,
    pub storage_ms: u64,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            system_ms: 1500,
            network_ms: 5000,
            bluetooth_ms: 3000,
            audio_ms: 1500,
            display_ms: 2000,
            power_ms: 5000,
            storage_ms: 8000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SplashConfig {
    pub min_ms: u64,
    pub enabled: bool,
}

impl Default for SplashConfig {
    fn default() -> Self {
        Self {
            min_ms: 1400,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OverviewConfig {
    pub ascii: String,
}

impl Default for OverviewConfig {
    fn default() -> Self {
        Self {
            ascii: "auto".to_string(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        for path in Self::candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match toml::from_str::<Config>(&text) {
                    Ok(cfg) => return cfg,
                    Err(e) => {
                        eprintln!("hal9001: config inválida em {path:?}: {e}");
                    }
                }
            }
        }
        Config::default()
    }

    pub fn save(&self) -> Result<std::path::PathBuf, String> {
        let toml_str =
            toml::to_string_pretty(self).map_err(|e| format!("erro ao serializar config: {e}"))?;

        let target_file = if let Ok(p) = std::env::var("HAL9001_CONFIG") {
            std::path::PathBuf::from(p)
        } else {
            let target_dir = directories::ProjectDirs::from("com", "hal9001", "hall-9001")
                .map(|dirs| dirs.config_dir().to_path_buf())
                .unwrap_or_else(|| {
                    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                    std::path::PathBuf::from(home).join(".config/hall-9001")
                });
            let _ = std::fs::create_dir_all(&target_dir);
            target_dir.join("config.toml")
        };

        if let Some(parent) = target_file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let tmp_file = target_file.with_extension("tmp");
        std::fs::write(&tmp_file, &toml_str)
            .map_err(|e| format!("erro ao salvar {tmp_file:?}: {e}"))?;
        std::fs::rename(&tmp_file, &target_file)
            .map_err(|e| format!("erro ao renomear {tmp_file:?} para {target_file:?}: {e}"))?;

        Ok(target_file)
    }

    fn candidate_paths() -> Vec<std::path::PathBuf> {
        let mut paths = Vec::new();
        if let Ok(p) = std::env::var("HAL9001_CONFIG") {
            paths.push(std::path::PathBuf::from(p));
        }
        if let Some(dirs) = directories::ProjectDirs::from("com", "hal9001", "hall-9001") {
            paths.push(dirs.config_dir().join("config.toml"));
        }
        if let Ok(home) = std::env::var("HOME") {
            paths.push(std::path::PathBuf::from(&home).join(".config/hall-9001/config.toml"));
            paths.push(std::path::PathBuf::from(&home).join(".config/hal9001/config.toml"));
        }
        paths.push(std::path::PathBuf::from("./config.toml"));
        paths
    }
}
