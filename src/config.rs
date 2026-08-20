//! Carregamento e defaults do `config.toml`.
//!
//! Procura, em ordem: `$HAL9001_CONFIG`, `~/.config/hal9001/config.toml`,
//! `./config.toml`. Se nada existir ou o parse falhar, usa os defaults.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub ui: UiConfig,
    pub theme: ThemeConfig,
    pub polling: PollingConfig,
    pub splash: SplashConfig,
    pub overview: OverviewConfig,
}

use crate::i18n::Language;

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PollingConfig {
    pub system_ms: u64,
    pub network_ms: u64,
    pub power_ms: u64,
    pub storage_ms: u64,
}

impl Default for PollingConfig {
    fn default() -> Self {
        Self {
            system_ms: 1500,
            network_ms: 5000,
            power_ms: 5000,
            storage_ms: 8000,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Clone, Deserialize)]
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

    fn candidate_paths() -> Vec<std::path::PathBuf> {
        let mut paths = Vec::new();
        if let Ok(p) = std::env::var("HAL9001_CONFIG") {
            paths.push(std::path::PathBuf::from(p));
        }
        if let Some(dirs) = directories::BaseDirs::new() {
            paths.push(
                dirs.config_dir()
                    .join("hal9001")
                    .join("config.toml"),
            );
        }
        paths.push(std::path::PathBuf::from("config.toml"));
        paths
    }
}
