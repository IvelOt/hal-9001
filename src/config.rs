//! Carregamento, defaults e persistência do `config.toml`.
//!
//! Procura, em ordem: `$HAL9001_CONFIG`, `~/.config/hall-9001/config.toml`,
//! `~/.config/hal9001/config.toml`, `./config.toml`. Se nada existir ou o parse
//! falhar, usa os defaults.

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
    /// Intervalo entre frames de render (ms). ~33ms ≈ 30fps.
    pub frame_ms: u64,
    /// Habilita ícones nerd-font (fallback ASCII quando `false`).
    pub icons: bool,
    /// Idioma da interface: `"auto"`, `"pt-BR"`, `"en-US"`, `"es-ES"`.
    pub language: String,
}

impl UiConfig {
    /// Resolve o idioma configurado, utilizando detecção automática por `$LANG` quando `"auto"`.
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
    /// Nome do tema embutido: `hal` (padrão), `mono`.
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
    /// Tempo mínimo da splash antes de revelar o Overview (ms).
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
    /// Seleção da logo: `auto`, `main`, `medium`, `compact`, `none`.
    /// (Aliases legados: `A`=main, `B`=compact, `C`=medium.)
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
    /// Carrega a config do primeiro caminho existente, ou defaults.
    pub fn load() -> Self {
        for path in Self::candidate_paths() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                match toml::from_str::<Config>(&text) {
                    Ok(cfg) => return cfg,
                    Err(e) => {
                        // Não temos TUI ainda; um aviso em stderr é aceitável
                        // aqui pois ocorre antes de entrar no alt-screen.
                        eprintln!("hal9001: config inválida em {path:?}: {e}");
                    }
                }
            }
        }
        Config::default()
    }

    /// Salva a configuração atual no diretório de configuração do usuário (`~/.config/hall-9001/config.toml`).
    pub fn save(&self) -> Result<std::path::PathBuf, String> {
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| format!("erro ao serializar config: {e}"))?;
        let target_dir = directories::ProjectDirs::from("com", "hal9001", "hall-9001")
            .map(|dirs| dirs.config_dir().to_path_buf())
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                std::path::PathBuf::from(home).join(".config/hall-9001")
            });
        let _ = std::fs::create_dir_all(&target_dir);
        let target_file = target_dir.join("config.toml");
        std::fs::write(&target_file, toml_str)
            .map_err(|e| format!("erro ao salvar {target_file:?}: {e}"))?;
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
