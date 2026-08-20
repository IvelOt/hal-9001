//! Backend de sistema (sysinfo + leituras de `/sys`/`wpctl`) → [`SystemSnapshot`].
//!
//! Além de CPU/RAM (via `sysinfo`), coleta dados típicos de neofetch/fastfetch:
//! pacotes instalados, brilho, volume, bateria, disco raiz e modelo do host.
//! Todas as leituras degradam graciosamente (campos `Option`/`N/A`) quando o
//! recurso não existe (ex.: desktop sem bateria, monitor externo sem brilho).

use std::path::Path;
use std::time::Duration;

use sysinfo::{Disks, System};
use tokio::sync::broadcast;

use crate::events::{Action, AppEvent, EventTx};

/// Contagem de pacotes por gerenciador detectado.
#[derive(Debug, Clone, Default)]
pub struct Packages {
    /// Total agregado entre gerenciadores detectados.
    pub total: u64,
    /// Detalhamento `("pacman", 1234)`, na ordem de detecção.
    pub by_manager: Vec<(&'static str, u64)>,
}

impl Packages {
    /// Resumo curto `1234 (pacman)` ou `1234 (pacman+flatpak)`.
    pub fn summary(&self) -> String {
        if self.by_manager.is_empty() {
            return "N/A".into();
        }
        let names: Vec<&str> = self.by_manager.iter().map(|(n, _)| *n).collect();
        format!("{} ({})", self.total, names.join("+"))
    }
}

/// Estado de carga da bateria primária.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

impl BatteryStatus {
    /// Deriva do conteúdo de `/sys/class/power_supply/BAT*/status`.
    pub fn parse(raw: &str) -> BatteryStatus {
        match raw.trim().to_ascii_lowercase().as_str() {
            "charging" => BatteryStatus::Charging,
            "discharging" => BatteryStatus::Discharging,
            "full" => BatteryStatus::Full,
            "not charging" => BatteryStatus::NotCharging,
            _ => BatteryStatus::Unknown,
        }
    }

    /// Ícone compacto para a UI.
    pub fn icon(self) -> &'static str {
        match self {
            BatteryStatus::Charging => "⚡",
            BatteryStatus::Full => "🔌",
            BatteryStatus::Discharging => "🔋",
            BatteryStatus::NotCharging => "⏸",
            BatteryStatus::Unknown => "?",
        }
    }

    /// Rótulo textual.
    pub fn label(self) -> &'static str {
        match self {
            BatteryStatus::Charging => "Carregando",
            BatteryStatus::Full => "Completa",
            BatteryStatus::Discharging => "Descarregando",
            BatteryStatus::NotCharging => "Sem carga",
            BatteryStatus::Unknown => "Desconhecido",
        }
    }
}

/// Bateria primária (BAT0/BAT1).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Battery {
    /// Percentual de carga em 0..=100.
    pub percent: f64,
    pub status: BatteryStatus,
    /// Potência instantânea em Watts, se disponível.
    pub power_watts: Option<f64>,
}

impl Battery {
    pub fn ratio(&self) -> f64 {
        (self.percent / 100.0).clamp(0.0, 1.0)
    }
}

/// Volume do sink de áudio padrão.
#[derive(Debug, Clone, Copy)]
pub struct Volume {
    /// Nível em 0.0..=1.0 (pode passar de 1.0 em hardware; é clampeado ao exibir).
    pub level: f64,
    pub muted: bool,
}

impl Volume {
    pub fn ratio(&self) -> f64 {
        self.level.clamp(0.0, 1.0)
    }
}

/// Snapshot enxuto e pronto-para-render do estado do sistema.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub host: String,
    pub user: String,
    pub shell: String,
    pub os: String,
    pub kernel: String,
    pub uptime_secs: u64,
    pub cpu_name: String,
    /// Uso global de CPU em 0.0..=100.0.
    pub cpu_usage: f32,
    pub mem_used: u64,
    pub mem_total: u64,

    // --- Módulo 1: dados no estilo neofetch ---
    /// Modelo do equipamento (DMI/devicetree), se legível.
    pub host_model: Option<String>,
    /// Pacotes instalados por gerenciador.
    pub packages: Option<Packages>,
    /// Brilho da tela em 0.0..=1.0.
    pub brightness: Option<f64>,
    /// Volume do áudio padrão.
    pub volume: Option<Volume>,
    /// Bateria primária.
    pub battery: Option<Battery>,
    /// Espaço usado no disco raiz `/` (bytes).
    pub disk_used: Option<u64>,
    /// Espaço total do disco raiz `/` (bytes).
    pub disk_total: Option<u64>,
}

/// Dados estáticos coletados uma única vez (caros ou imutáveis).
#[derive(Debug, Clone, Default)]
struct StaticInfo {
    host_model: Option<String>,
    packages: Option<Packages>,
}

impl SystemSnapshot {
    /// Coleta a partir de um `System` já refreshado, mesclando dados estáticos
    /// e leituras dinâmicas de `/sys` e utilitários de áudio.
    fn collect(sys: &System, disks: &Disks, stat: &StaticInfo) -> Self {
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "CPU desconhecida".to_string());

        let (disk_used, disk_total) = read_root_disk(disks)
            .map(|(u, t)| (Some(u), Some(t)))
            .unwrap_or((None, None));

        SystemSnapshot {
            host: System::host_name().unwrap_or_else(|| "localhost".into()),
            user: std::env::var("USER").unwrap_or_else(|_| "user".into()),
            shell: std::env::var("SHELL")
                .ok()
                .and_then(|s| s.rsplit('/').next().map(str::to_string))
                .unwrap_or_else(|| "sh".into()),
            os: System::long_os_version()
                .or_else(System::name)
                .unwrap_or_else(|| "Linux".into()),
            kernel: System::kernel_version().unwrap_or_else(|| "?".into()),
            uptime_secs: System::uptime(),
            cpu_name,
            cpu_usage: sys.global_cpu_usage(),
            mem_used: sys.used_memory(),
            mem_total: sys.total_memory(),

            host_model: stat.host_model.clone(),
            packages: stat.packages.clone(),
            brightness: read_brightness(Path::new("/sys/class/backlight")),
            volume: read_volume(),
            battery: read_battery(Path::new("/sys/class/power_supply")),
            disk_used,
            disk_total,
        }
    }

    /// Fração de memória usada em 0.0..=1.0.
    pub fn mem_ratio(&self) -> f64 {
        ratio(self.mem_used, self.mem_total)
    }

    /// Fração de CPU usada em 0.0..=1.0.
    pub fn cpu_ratio(&self) -> f64 {
        (self.cpu_usage as f64 / 100.0).clamp(0.0, 1.0)
    }

    /// Fração de disco raiz usada em 0.0..=1.0, se disponível.
    pub fn disk_ratio(&self) -> Option<f64> {
        match (self.disk_used, self.disk_total) {
            (Some(u), Some(t)) => Some(ratio(u, t)),
            _ => None,
        }
    }
}

/// Divisão segura `used/total` em 0.0..=1.0.
pub fn ratio(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    }
}

// ---------------------------------------------------------------------------
// Coletores estáticos
// ---------------------------------------------------------------------------

/// Lê o modelo do equipamento (DMI ou devicetree).
fn read_host_model() -> Option<String> {
    let candidates = [
        "/sys/devices/virtual/dmi/id/product_name",
        "/sys/firmware/devicetree/base/model",
    ];
    for path in candidates {
        if let Ok(s) = std::fs::read_to_string(path) {
            // devicetree costuma terminar com NUL.
            let s = s.trim_matches(|c: char| c.is_whitespace() || c == '\0');
            if !s.is_empty() && s != "To be filled by O.E.M." && s != "System Product Name" {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Conta pacotes instalados nos gerenciadores detectados no `PATH`.
fn count_packages() -> Option<Packages> {
    let mut pkgs = Packages::default();

    // pacman (Arch): `pacman -Qq` — uma linha por pacote.
    if let Some(n) = count_lines("pacman", &["-Qq"]) {
        pkgs.by_manager.push(("pacman", n));
        pkgs.total += n;
    }
    // dpkg (Debian/Ubuntu): linhas iniciadas por "ii".
    if let Some(n) = count_dpkg() {
        pkgs.by_manager.push(("dpkg", n));
        pkgs.total += n;
    }
    // rpm (Fedora/RHEL/openSUSE).
    if let Some(n) = count_lines("rpm", &["-qa"]) {
        pkgs.by_manager.push(("rpm", n));
        pkgs.total += n;
    }
    // flatpak (transversal).
    if let Some(n) = count_lines("flatpak", &["list", "--app", "--columns=application"]) {
        pkgs.by_manager.push(("flatpak", n));
        pkgs.total += n;
    }

    if pkgs.by_manager.is_empty() {
        None
    } else {
        Some(pkgs)
    }
}

/// Executa um comando e conta as linhas não-vazias do stdout.
fn count_lines(cmd: &str, args: &[&str]) -> Option<u64> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(count_nonempty_lines(&text))
}

/// dpkg exige contar apenas pacotes efetivamente instalados (status `ii`).
fn count_dpkg() -> Option<u64> {
    let out = std::process::Command::new("dpkg-query")
        .args(["-f", "${db:Status-Abbrev}\n", "-W"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(count_installed_dpkg(&text))
}

// ---------------------------------------------------------------------------
// Coletores dinâmicos e parsers puros (testáveis)
// ---------------------------------------------------------------------------

/// Conta linhas não vazias (usado para gerenciadores um-por-linha).
pub fn count_nonempty_lines(text: &str) -> u64 {
    text.lines().filter(|l| !l.trim().is_empty()).count() as u64
}

/// Conta pacotes dpkg com status abreviado começando por `ii`.
pub fn count_installed_dpkg(text: &str) -> u64 {
    text.lines()
        .filter(|l| l.trim_start().starts_with("ii"))
        .count() as u64
}

/// Brilho em 0.0..=1.0 a partir de `current`/`max` já lidos como texto.
pub fn brightness_ratio(current: &str, max: &str) -> Option<f64> {
    let cur: f64 = current.trim().parse().ok()?;
    let mx: f64 = max.trim().parse().ok()?;
    if mx <= 0.0 {
        return None;
    }
    Some((cur / mx).clamp(0.0, 1.0))
}

/// Lê o primeiro backlight disponível sob `dir` (`/sys/class/backlight`).
pub fn read_brightness(dir: &Path) -> Option<f64> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let base = entry.path();
        let cur = std::fs::read_to_string(base.join("brightness")).ok();
        let max = std::fs::read_to_string(base.join("max_brightness")).ok();
        if let (Some(c), Some(m)) = (cur, max) {
            if let Some(r) = brightness_ratio(&c, &m) {
                return Some(r);
            }
        }
    }
    None
}

/// Parseia a saída de `wpctl get-volume @DEFAULT_AUDIO_SINK@`.
///
/// Exemplos: `Volume: 0.65`, `Volume: 0.65 [MUTED]`.
pub fn parse_wpctl_volume(output: &str) -> Option<Volume> {
    let rest = output.trim().strip_prefix("Volume:")?.trim();
    let muted = rest.contains("[MUTED]");
    let num = rest.split_whitespace().next()?;
    let level: f64 = num.parse().ok()?;
    Some(Volume { level, muted })
}

/// Parseia a saída de `amixer sget Master` (fallback ALSA).
///
/// Procura por `[65%]` e o token `[on]`/`[off]`.
pub fn parse_amixer_volume(output: &str) -> Option<Volume> {
    let mut level = None;
    let mut muted = false;
    for tok in output.split(['[', ']']) {
        let tok = tok.trim();
        if let Some(pct) = tok.strip_suffix('%') {
            if let Ok(p) = pct.trim().parse::<f64>() {
                level = Some((p / 100.0).clamp(0.0, 2.0));
            }
        } else if tok.eq_ignore_ascii_case("off") {
            muted = true;
        }
    }
    level.map(|level| Volume { level, muted })
}

/// Lê o volume via `wpctl` (PipeWire) com fallback para `amixer` (ALSA).
fn read_volume() -> Option<Volume> {
    if let Ok(out) = std::process::Command::new("wpctl")
        .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
        .output()
    {
        if out.status.success() {
            if let Some(v) = parse_wpctl_volume(&String::from_utf8_lossy(&out.stdout)) {
                return Some(v);
            }
        }
    }
    if let Ok(out) = std::process::Command::new("amixer")
        .args(["sget", "Master"])
        .output()
    {
        if out.status.success() {
            if let Some(v) = parse_amixer_volume(&String::from_utf8_lossy(&out.stdout)) {
                return Some(v);
            }
        }
    }
    None
}

/// Lê a primeira bateria (`BAT*`) sob `dir` (`/sys/class/power_supply`).
pub fn read_battery(dir: &Path) -> Option<Battery> {
    let entries = std::fs::read_dir(dir).ok()?;
    // Coleta e ordena para preferir BAT0 sobre BAT1 de forma determinística.
    let mut bats: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("BAT"))
                .unwrap_or(false)
        })
        .collect();
    bats.sort();

    for base in bats {
        let capacity = std::fs::read_to_string(base.join("capacity")).ok();
        let status = std::fs::read_to_string(base.join("status")).ok();
        if let Some(cap) = capacity {
            if let Ok(percent) = cap.trim().parse::<f64>() {
                return Some(Battery {
                    percent: percent.clamp(0.0, 100.0),
                    status: status
                        .map(|s| BatteryStatus::parse(&s))
                        .unwrap_or(BatteryStatus::Unknown),
                    power_watts: read_battery_power(&base),
                });
            }
        }
    }
    None
}

/// Potência instantânea em Watts a partir de `power_now` (µW) ou
/// `current_now`×`voltage_now` (µA·µV).
fn read_battery_power(base: &Path) -> Option<f64> {
    if let Ok(p) = std::fs::read_to_string(base.join("power_now")) {
        if let Ok(uw) = p.trim().parse::<f64>() {
            if uw > 0.0 {
                return Some(uw / 1_000_000.0);
            }
        }
    }
    let current = std::fs::read_to_string(base.join("current_now")).ok()?;
    let voltage = std::fs::read_to_string(base.join("voltage_now")).ok()?;
    let ua: f64 = current.trim().parse().ok()?;
    let uv: f64 = voltage.trim().parse().ok()?;
    if ua <= 0.0 || uv <= 0.0 {
        return None;
    }
    // (µA * µV) / 1e12 = W.
    Some((ua * uv) / 1_000_000_000_000.0)
}

/// Retorna `(usado, total)` do disco montado em `/`, se encontrado.
fn read_root_disk(disks: &Disks) -> Option<(u64, u64)> {
    disks
        .list()
        .iter()
        .find(|d| d.mount_point() == Path::new("/"))
        .map(|d| {
            let total = d.total_space();
            let used = total.saturating_sub(d.available_space());
            (used, total)
        })
}

// ---------------------------------------------------------------------------
// Task de polling
// ---------------------------------------------------------------------------

/// Task de polling do sistema.
pub async fn run(
    poll_ms: u64,
    tx: EventTx,
    mut actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    let mut sys = System::new_all();
    let mut disks = Disks::new_with_refreshed_list();

    // Dados estáticos: coletados uma única vez (contagem de pacotes é cara).
    let stat = StaticInfo {
        host_model: read_host_model(),
        packages: count_packages(),
    };

    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms.max(250)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Primeiro refresh estabelece a baseline de CPU; o valor de uso só é
    // significativo a partir do segundo tick.
    loop {
        ticker.tick().await;

        // Drena ações pendentes (ex.: Refresh) sem bloquear.
        while actions.try_recv().is_ok() {}

        sys.refresh_cpu_usage();
        sys.refresh_memory();
        disks.refresh();

        if tx
            .send(AppEvent::System(Box::new(SystemSnapshot::collect(
                &sys, &disks, &stat,
            ))))
            .is_err()
        {
            // App encerrou: nada a fazer.
            break;
        }
    }
    Ok(())
}
