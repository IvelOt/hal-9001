//! Configuração da Home estilo Fastfetch do HAL-9001.
//!
//! Carrega um arquivo TOML em `~/.config/hall-9001/config.toml` permitindo
//! personalizar o logo ASCII, o tema de acentos de cor e quais métricas de
//! sistema exibir. Quando o arquivo está ausente ou parcial, usa fallbacks
//! embutidos (tema `retro`, logo padrão, todas as métricas ativas).

use std::path::PathBuf;

use serde::Deserialize;

/// Temas de acento de cor disponíveis para a Home estilo Fastfetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum AccentTheme {
    #[default]
    Retro,
    Green,
    Cyan,
    Magenta,
}

impl AccentTheme {
    /// Resolve um nome textual (tolera `green`, `cyan`, `retro`, `magenta`).
    pub fn from_str_loose(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "green" => Self::Green,
            "cyan" => Self::Cyan,
            "magenta" => Self::Magenta,
            _ => Self::Retro,
        }
    }
}

/// Métricas de sistema que a Home pode exibir (todas ligadas por padrão).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Metrics {
    pub os: bool,
    pub host: bool,
    pub kernel: bool,
    pub uptime: bool,
    pub cpu: bool,
    pub ram: bool,
    pub disks: bool,
    pub battery: bool,
    pub shell: bool,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            os: true,
            host: true,
            kernel: true,
            uptime: true,
            cpu: true,
            ram: true,
            disks: true,
            battery: true,
            shell: true,
        }
    }
}

/// Configuração completa lida do TOML (com fallbacks embutidos).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Nome de usuário/host exibido na Home (ex.: `user@host`).
    pub user: String,
    /// Texto do cabeçalho da Home.
    pub title: String,
    /// Tema de acentos de cor.
    pub accent: AccentTheme,
    /// Logo ASCII (multilinha); `~` é expandido para o diretório home caso o
    /// usuário aponte para um arquivo com `file = "..."`.
    pub logo: String,
    /// Métricas de sistema a exibir na Home.
    pub metrics: Metrics,
}

impl Default for Config {
    fn default() -> Self {
        // Logo ASCII padrão: um "HAL-9000" estilizado em caixas.
        let logo = r#"
  ██     ██ ███████  █████  ██      ██       ████████  ██████   ██████  ██  ██████   ██
  ██     ██ ██      ██   ██ ██      ██          ██    ██    ██ ██       ██ ██    ██  ██
  ██  █  ██ ███████ ███████ ██      ██          ██    ██    ██ ██       ██ ██    ██  ██
  ██ ███ ██      ██ ██   ██ ██      ██          ██    ██    ██ ██       ██ ██    ██  ██
   ███ ███  ███████ ██   ██ ███████ ███████     ██     ██████   ██████  ██  ██████    ██
"#;
        Self {
            user: "user@host".to_string(),
            title: "Welcome back to HAL-9001".to_string(),
            accent: AccentTheme::Retro,
            logo: logo.to_string(),
            metrics: Metrics::default(),
        }
    }
}

/// Retorna o caminho do arquivo de configuração.
pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("hall-9001")
        .join("config.toml")
}

/// Carrega a configuração, aplicando fallbacks embutidos quando o arquivo
/// não existe ou contém valores inválidos.
pub fn load() -> Config {
    let mut config = Config::default();
    let path = config_path();
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => {
            // Garante um arquivo de exemplo na primeira execução.
            let _ = std::fs::write(
                &path,
                format!(
                    "{}\n{:#}\n",
                    "# config.toml do HAL-9001 (Home estilo Fastfetch)",
                    example_toml(&config)
                ),
            );
            return config;
        }
    };

    #[derive(Deserialize)]
    struct Disk {
        logo: Option<String>,
        #[serde(rename = "logo-file")]
        logo_file: Option<String>,
    }

    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct File {
        user: Option<String>,
        title: Option<String>,
        accent: Option<String>,
        metrics: Option<Metrics>,
        disk: Option<Disk>,
    }

    let parsed: File = match toml::from_str(&content) {
        Ok(file) => file,
        Err(_) => return config,
    };

    if let Some(user) = parsed.user {
        config.user = user;
    }
    if let Some(title) = parsed.title {
        config.title = title;
    }
    if let Some(accent) = parsed.accent {
        config.accent = AccentTheme::from_str_loose(&accent);
    }
    if let Some(metrics) = parsed.metrics {
        config.metrics = metrics;
    }
    if let Some(disk) = parsed.disk {
        if let Some(logo_file) = disk.logo_file {
            if let Some(logo) = read_logo_file(&logo_file) {
                config.logo = logo;
            }
        } else if let Some(logo) = disk.logo {
            config.logo = logo;
        }
    }
    config
}

/// Lê o conteúdo de um arquivo de logo, expandindo `~`.
fn read_logo_file(path: &str) -> Option<String> {
    let expanded = if let Some(rest) = path.strip_prefix("~/") {
        let home = std::env::var("HOME").ok()?;
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    };
    std::fs::read_to_string(expanded).ok()
}

/// Constrói um TOML de exemplo a partir de uma configuração.
fn example_toml(config: &Config) -> String {
    let metrics = &config.metrics;
    format!(
        r#"
user = "{user}"
title = "{title}"
accent = "retro"   # green | cyan | retro | magenta

[logo]
# Multilinha ou aponte para um arquivo com logo-file = "~/meu-logo.txt"
logo = '''
██     ██ ███████  █████  ██      ██       ████████  ██████   ██████  ██  ██████   ██
██     ██ ██      ██   ██ ██      ██          ██    ██    ██ ██       ██ ██    ██  ██
██  █  ██ ███████ ███████ ██      ██          ██    ██    ██ ██       ██ ██    ██  ██
██ ███ ██      ██ ██   ██ ██      ██          ██    ██    ██ ██       ██ ██    ██  ██
 ███ ███  ███████ ██   ██ ███████ ███████     ██     ██████   ██████  ██  ██████    ██
'''

[metrics]
os      = {os}
host    = {host}
kernel  = {kernel}
uptime  = {uptime}
cpu     = {cpu}
ram     = {ram}
disks   = {disks}
battery = {battery}
shell   = {shell}
"#,
        user = config.user,
        title = config.title,
        os = metrics.os,
        host = metrics.host,
        kernel = metrics.kernel,
        uptime = metrics.uptime,
        cpu = metrics.cpu,
        ram = metrics.ram,
        disks = metrics.disks,
        battery = metrics.battery,
        shell = metrics.shell,
    )
}

/// Informações de sistema lidas de `/etc` e `/proc` para a Home Fastfetch.
#[derive(Debug, Clone, Default)]
pub struct HostInfo {
    pub os: String,
    pub host: String,
    pub kernel: String,
    pub cpu: String,
    pub shell: String,
}

/// Coleta informações de OS/host/kernel/CPU/shell de forma tolerante.
pub fn collect_host_info() -> HostInfo {
    HostInfo {
        os: read_os_release(),
        host: read_hostname(),
        kernel: read_kernel(),
        cpu: read_cpu_model(),
        shell: read_shell(),
    }
}

/// Lê o nome bonito do sistema em `/etc/os-release` (`PRETTY_NAME`).
fn read_os_release() -> String {
    let Ok(content) = std::fs::read_to_string("/etc/os-release") else {
        return "Unknown Linux".to_string();
    };
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return value.trim_matches('"').to_string();
        }
    }
    "Unknown Linux".to_string()
}

/// Lê o nome do host.
fn read_hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|content| content.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Lê a versão do kernel (`/proc/version`).
fn read_kernel() -> String {
    std::fs::read_to_string("/proc/version")
        .ok()
        .map(|content| content.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Lê o nome/modelo da CPU (`/proc/cpuinfo`, linha `model name`).
fn read_cpu_model() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|content| {
            content
                .lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

/// Lê o shell padrão do usuário (`$SHELL`).
fn read_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_all_metrics() {
        let config = Config::default();
        assert!(config.metrics.os && config.metrics.host && config.metrics.kernel);
        assert!(config.metrics.uptime && config.metrics.cpu);
        assert!(config.metrics.ram && config.metrics.disks);
        assert!(config.metrics.battery && config.metrics.shell);
        assert!(!config.logo.trim().is_empty());
    }

    #[test]
    fn accent_parses_known_values() {
        assert_eq!(AccentTheme::from_str_loose("green"), AccentTheme::Green);
        assert_eq!(AccentTheme::from_str_loose("CYAN"), AccentTheme::Cyan);
        assert_eq!(AccentTheme::from_str_loose("retro"), AccentTheme::Retro);
        assert_eq!(AccentTheme::from_str_loose("magenta"), AccentTheme::Magenta);
        assert_eq!(AccentTheme::from_str_loose("qualquer"), AccentTheme::Retro);
    }

    #[test]
    fn parses_metrics_toml() {
        let toml = r#"
os = false
cpu = false
"#;
        let parsed: Metrics = toml::from_str(toml).unwrap();
        assert!(!parsed.os);
        assert!(!parsed.cpu);
        assert!(parsed.host); // padrão preservado
    }

    #[test]
    fn host_info_collects_nonempty() {
        let info = collect_host_info();
        assert!(!info.os.is_empty());
        assert!(!info.kernel.is_empty());
        assert!(!info.host.is_empty());
    }
}