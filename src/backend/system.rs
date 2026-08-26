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

use crate::events::{Action, AppEvent, EventTx, Toast};

/// Contagem de pacotes por gerenciador detectado.
#[derive(Debug, Clone, Default)]
pub struct Packages {
    /// Total agregado entre gerenciadores detectados.
    pub total: u64,
    /// Detalhamento `("pacman", 1234)`, na ordem de detecção.
    pub by_manager: Vec<(&'static str, u64)>,
    /// Quantidade de atualizações pendentes (se já checado).
    pub pending_updates: Option<usize>,
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

    /// Tag textual em maiúsculas, estilo neofetch clássico (`CHARGING`,
    /// `DISCHARGING`, `FULL`, `NOT CHARGING`, `UNKNOWN`). Sem emojis.
    pub fn tag(self) -> &'static str {
        match self {
            BatteryStatus::Charging => "CHARGING",
            BatteryStatus::Full => "FULL",
            BatteryStatus::Discharging => "DISCHARGING",
            BatteryStatus::NotCharging => "NOT CHARGING",
            BatteryStatus::Unknown => "UNKNOWN",
        }
    }

    /// Sinal ASCII da potência conforme o estado: `+` carregando, `-`
    /// descarregando, vazio caso contrário.
    pub fn power_sign(self) -> &'static str {
        match self {
            BatteryStatus::Charging => "+",
            BatteryStatus::Discharging => "-",
            _ => "",
        }
    }

    /// Rótulo textual (pt-BR) para o modo detalhado.
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
#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    /// Percentual de carga em 0..=100.
    pub percent: f64,
    pub status: BatteryStatus,
    /// Potência instantânea em Watts, se disponível.
    pub power_watts: Option<f64>,
    /// Saúde da bateria em 0.0..=1.0 (capacidade atual de fábrica ÷ projeto).
    pub health: Option<f64>,
    /// Contagem de ciclos de recarga, se exposta pelo firmware.
    pub cycle_count: Option<u64>,
    /// Tecnologia da célula (`Li-poly`, `Li-ion`, ...).
    pub technology: Option<String>,
}

impl Battery {
    pub fn ratio(&self) -> f64 {
        (self.percent / 100.0).clamp(0.0, 1.0)
    }
}

/// Saúde da bateria: capacidade máxima atual ÷ capacidade de projeto, em
/// 0.0..=1.0. `None` quando faltam leituras ou o projeto é zero.
pub fn battery_health(full: f64, design: f64) -> Option<f64> {
    if design <= 0.0 || full <= 0.0 {
        return None;
    }
    Some((full / design).clamp(0.0, 1.5))
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

/// Perfil de energia ativo (power-profiles-daemon / scaling governor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerProfile {
    /// `power-saver` — Economia.
    PowerSaver,
    /// `balanced` — Equilibrado.
    Balanced,
    /// `performance` — Desempenho.
    Performance,
}

impl PowerProfile {
    /// Deriva o perfil a partir do id do `power-profiles-daemon`
    /// (`power-saver`/`balanced`/`performance`) ou de um scaling governor do
    /// sysfs (`powersave`/`schedutil`/`performance`/...). Degrada para `None`.
    pub fn parse(raw: &str) -> Option<PowerProfile> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "power-saver" | "power_saver" | "powersave" | "economia" => {
                Some(PowerProfile::PowerSaver)
            }
            "balanced" | "equilibrado" | "schedutil" | "ondemand" | "conservative"
            | "userspace" => Some(PowerProfile::Balanced),
            "performance" | "desempenho" => Some(PowerProfile::Performance),
            _ => None,
        }
    }

    /// Id textual aceito por `powerprofilesctl set` (`power-saver`, ...).
    pub fn id(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "power-saver",
            PowerProfile::Balanced => "balanced",
            PowerProfile::Performance => "performance",
        }
    }

    /// Rótulo traduzido para toasts/UI conforme o idioma.
    pub fn label_in(self, lang: crate::i18n::Language) -> &'static str {
        let m = lang.messages();
        match self {
            PowerProfile::PowerSaver => m.profile_power_saver,
            PowerProfile::Balanced => m.profile_balanced,
            PowerProfile::Performance => m.profile_performance,
        }
    }

    /// Rótulo padrão para toasts/UI (`Economia`, `Equilibrado`, `Desempenho`).
    pub fn label(self) -> &'static str {
        self.label_in(crate::i18n::Language::default())
    }

    /// Tag textual em maiúsculas, estilo neofetch, sem emojis:
    /// `[POWER-SAVER]`, `[BALANCED]`, `[PERFORMANCE]`.
    pub fn tag(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "[POWER-SAVER]",
            PowerProfile::Balanced => "[BALANCED]",
            PowerProfile::Performance => "[PERFORMANCE]",
        }
    }

    /// Scaling governor equivalente, usado no fallback via sysfs.
    pub fn governor(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "powersave",
            PowerProfile::Balanced => "schedutil",
            PowerProfile::Performance => "performance",
        }
    }

    /// Próximo perfil no ciclo `PowerSaver` → `Balanced` → `Performance` →
    /// `PowerSaver`.
    pub fn next(&self) -> PowerProfile {
        match self {
            PowerProfile::PowerSaver => PowerProfile::Balanced,
            PowerProfile::Balanced => PowerProfile::Performance,
            PowerProfile::Performance => PowerProfile::PowerSaver,
        }
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
    /// Iluminação do teclado em 0.0..=1.0.
    pub kbd_backlight: Option<f32>,
    /// Perfil de energia ativo (`None` em máquinas sem daemon/governor legível).
    pub power_profile: Option<PowerProfile>,

    /// Campos extras exibidos apenas no modo detalhado (tecla `.`).
    pub detail: DetailInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
}

/// Informações extras exibidas apenas no **modo detalhado** do Overview.
///
/// Todos os campos degradam graciosamente (`Option`/`0`) quando o dado não
/// está disponível na plataforma.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetailInfo {
    // Placa-mãe & BIOS (DMI).
    pub board_vendor: Option<String>,
    pub board_name: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,

    // GPU.
    pub gpu: Option<String>,

    // CPU detalhada.
    pub cpu_arch: Option<String>,
    pub cpu_cores_physical: Option<usize>,
    pub cpu_cores_logical: usize,
    pub cpu_freq_ghz: Option<f64>,
    pub cpu_temp_c: Option<f64>,

    // Memória virtual / swap.
    pub swap_used: u64,
    pub swap_total: u64,

    // Ambiente gráfico.
    pub desktop: Option<String>,
    pub session_type: Option<String>,

    // Top 5 processos
    pub top_processes: Vec<ProcessInfo>,
}

impl DetailInfo {
    /// Fração de swap usada em 0.0..=1.0.
    pub fn swap_ratio(&self) -> f64 {
        ratio(self.swap_used, self.swap_total)
    }
}

/// Dados estáticos coletados uma única vez (caros ou imutáveis).
#[derive(Debug, Clone, Default)]
struct StaticInfo {
    host_model: Option<String>,
    packages: Option<Packages>,
    /// Parte estática do modo detalhado (DMI/BIOS/GPU/arquitetura).
    detail: DetailInfo,
}

impl SystemSnapshot {
    /// Coleta a partir de um `System` já refreshado, mesclando dados estáticos
    /// e leituras dinâmicas de `/sys` e utilitários de áudio.
    fn collect(sys: &System, disks: &Disks, stat: &StaticInfo, cpu_temp_c: Option<f64>) -> Self {
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "CPU desconhecida".to_string());

        let (disk_used, disk_total) = read_root_disk(disks)
            .map(|(u, t)| (Some(u), Some(t)))
            .unwrap_or((None, None));

        // Modo detalhado: parte estática (DMI/GPU/arch) + parte dinâmica.
        let cpu_freq_mhz = sys.cpus().first().map(|c| c.frequency()).unwrap_or(0);
        let mut procs: Vec<_> = sys.processes().values().collect();
        procs.sort_unstable_by(|a, b| {
            b.cpu_usage().partial_cmp(&a.cpu_usage()).unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.memory().cmp(&a.memory()))
        });
        let top_processes = procs.into_iter().take(5).map(|p| ProcessInfo {
            pid: p.pid().as_u32(),
            name: p.name().to_string_lossy().into_owned(),
            cpu_usage: p.cpu_usage(),
            mem_bytes: p.memory(),
        }).collect();

        let detail = DetailInfo {
            cpu_cores_logical: sys.cpus().len(),
            cpu_cores_physical: sys.physical_core_count(),
            cpu_freq_ghz: if cpu_freq_mhz > 0 {
                Some(cpu_freq_mhz as f64 / 1000.0)
            } else {
                None
            },
            cpu_temp_c,
            swap_used: sys.used_swap(),
            swap_total: sys.total_swap(),
            top_processes,
            ..stat.detail.clone()
        };

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
            kbd_backlight: read_kbd_backlight(Path::new("/sys/class/leds")),
            power_profile: read_power_profile(),
            detail,
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
// Modo detalhado (tecla `.`)
// ---------------------------------------------------------------------------

/// Coleta a parte estática do modo detalhado (DMI/BIOS/GPU/arquitetura/DE).
/// Chamado uma única vez; os campos dinâmicos (freq/temp/swap) são preenchidos
/// a cada snapshot.
fn read_static_detail() -> DetailInfo {
    let dmi = Path::new("/sys/devices/virtual/dmi/id");
    let (desktop, session_type) = detect_desktop();
    DetailInfo {
        board_vendor: read_dmi(dmi, "board_vendor"),
        board_name: read_dmi(dmi, "board_name"),
        bios_version: read_dmi(dmi, "bios_version"),
        bios_date: read_dmi(dmi, "bios_date"),
        gpu: read_gpu(),
        cpu_arch: Some(std::env::consts::ARCH.to_string()),
        desktop,
        session_type,
        ..DetailInfo::default()
    }
}

/// Lê um campo DMI, descartando placeholders comuns de OEM.
fn read_dmi(dir: &Path, field: &str) -> Option<String> {
    let s = read_trimmed_file(&dir.join(field))?;
    if s == "To be filled by O.E.M." || s == "Default string" || s == "Unknown" {
        None
    } else {
        Some(s)
    }
}

/// Detecta ambiente gráfico e tipo de sessão a partir de variáveis de ambiente.
/// Retorna `(DE/WM, session_type)`.
fn detect_desktop() -> (Option<String>, Option<String>) {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let desktop = env("XDG_CURRENT_DESKTOP")
        .or_else(|| env("DESKTOP_SESSION"))
        .or_else(|| env("XDG_SESSION_DESKTOP"))
        .or_else(detect_wm_process);
    (desktop, env("XDG_SESSION_TYPE"))
}

/// Checa processos conhecidos de window managers standalone.
fn detect_wm_process() -> Option<String> {
    for wm in ["sway", "i3", "hyprland", "bspwm", "dwm", "awesome", "xmonad"] {
        if std::process::Command::new("pgrep")
            .args(["-x", wm])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(wm.to_string());
        }
    }
    None
}

/// GPU via `lspci`; degrada para `None` quando o utilitário não existe.
fn read_gpu() -> Option<String> {
    let out = std::process::Command::new("lspci").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lspci_gpu(&String::from_utf8_lossy(&out.stdout))
}

/// Extrai o nome da primeira controladora VGA/3D da saída do `lspci`.
pub fn parse_lspci_gpu(output: &str) -> Option<String> {
    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("vga compatible controller")
            || lower.contains("3d controller")
            || lower.contains("display controller")
        {
            // Formato: "00:02.0 VGA compatible controller: Intel Corporation ...".
            // O separador classe→nome é o primeiro ": " (o slot usa ":" sem espaço).
            if let Some((_, name)) = line.split_once(": ") {
                let name = name.trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

/// Temperatura da CPU (°C) a partir de `/sys/class/thermal`, preferindo zonas
/// cujo `type` indique CPU/pacote.
pub fn read_cpu_temp(dir: &Path) -> Option<f64> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut zones: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("thermal_zone"))
                .unwrap_or(false)
        })
        .collect();
    zones.sort();

    let mut fallback: Option<f64> = None;
    for zone in zones {
        let temp = read_trimmed_file(&zone.join("temp")).and_then(|t| parse_thermal_temp(&t));
        let Some(temp) = temp else { continue };
        let kind = read_trimmed_file(&zone.join("type"))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if kind.contains("x86_pkg")
            || kind.contains("cpu")
            || kind.contains("coretemp")
            || kind.contains("k10temp")
        {
            return Some(temp);
        }
        fallback.get_or_insert(temp);
    }
    fallback
}

/// Converte millicelsius (`45000`) em °C (`45.0`); ignora valores absurdos.
pub fn parse_thermal_temp(milli: &str) -> Option<f64> {
    let raw: f64 = milli.trim().parse().ok()?;
    let c = raw / 1000.0;
    if (0.0..=150.0).contains(&c) {
        Some(c)
    } else {
        None
    }
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

/// Lê o brilho do teclado em `/sys/class/leds`.
pub fn read_kbd_backlight(dir: &Path) -> Option<f32> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.ends_with("::kbd_backlight") || name_str.contains("kbd") {
            let base = entry.path();
            let cur = std::fs::read_to_string(base.join("brightness")).ok();
            let max = std::fs::read_to_string(base.join("max_brightness")).ok();
            if let (Some(c), Some(m)) = (cur, max) {
                if let Some(r) = brightness_ratio(&c, &m) {
                    return Some(r as f32);
                }
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
                    health: read_battery_health(&base),
                    cycle_count: read_u64_file(&base.join("cycle_count")),
                    technology: read_trimmed_file(&base.join("technology")),
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

/// Saúde a partir de `energy_full`/`energy_full_design` (µWh) ou, na falta,
/// `charge_full`/`charge_full_design` (µAh).
fn read_battery_health(base: &Path) -> Option<f64> {
    for (full_name, design_name) in [
        ("energy_full", "energy_full_design"),
        ("charge_full", "charge_full_design"),
    ] {
        let full = read_u64_file(&base.join(full_name));
        let design = read_u64_file(&base.join(design_name));
        if let (Some(f), Some(d)) = (full, design) {
            if let Some(h) = battery_health(f as f64, d as f64) {
                return Some(h);
            }
        }
    }
    None
}

/// Lê um arquivo e devolve seu conteúdo `trim`ado, se não vazio.
fn read_trimmed_file(path: &Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Lê um arquivo contendo um único inteiro sem sinal.
fn read_u64_file(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
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

/// Lê o perfil de energia ativo via `busctl`/`dbus-send`/`powerprofilesctl`, com
/// fallback para o scaling governor do sysfs e, por fim, `None` (ex.: desktop
/// sem daemon).
fn read_power_profile() -> Option<PowerProfile> {
    // 1. Tenta busctl (D-Bus nativo sem depender de runtime python).
    if let Ok(out) = std::process::Command::new("busctl")
        .args([
            "get-property",
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
            "ActiveProfile",
        ])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            let trimmed = s.trim().trim_start_matches("s ").trim_matches('"').trim();
            if let Some(p) = PowerProfile::parse(trimmed) {
                return Some(p);
            }
        }
    }

    // 2. Tenta dbus-send (D-Bus padrão).
    if let Ok(out) = std::process::Command::new("dbus-send")
        .args([
            "--system",
            "--print-reply",
            "--dest=net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "org.freedesktop.DBus.Properties.Get",
            "string:net.hadess.PowerProfiles",
            "string:ActiveProfile",
        ])
        .output()
    {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some((_, val)) = s.rsplit_once("string \"") {
                if let Some((profile_str, _)) = val.split_once('"') {
                    if let Some(p) = PowerProfile::parse(profile_str) {
                        return Some(p);
                    }
                }
            }
        }
    }

    // 3. Tenta CLI powerprofilesctl.
    for bin in ["powerprofilesctl", "/usr/sbin/powerprofilesctl"] {
        if let Ok(out) = std::process::Command::new(bin).arg("get").output() {
            if out.status.success() {
                if let Some(p) = PowerProfile::parse(&String::from_utf8_lossy(&out.stdout)) {
                    return Some(p);
                }
            }
        }
    }

    // 4. Fallback sysfs governor.
    read_governor_profile(Path::new(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
    ))
}

/// Deriva um [`PowerProfile`] a partir de um arquivo de scaling governor.
pub fn read_governor_profile(path: &Path) -> Option<PowerProfile> {
    PowerProfile::parse(&read_trimmed_file(path)?)
}

// ---------------------------------------------------------------------------
// Controles interativos (brilho / volume / perfil de energia)
// ---------------------------------------------------------------------------

/// Passo padrão (em %) de cada ajuste de brilho/volume.
pub const CONTROL_STEP: i32 = 5;

/// Monta o argumento de delta relativo (`5%+` / `5%-`) aceito por
/// `brightnessctl`/`wpctl`/`amixer`. Deltas negativos viram `%-`.
pub fn delta_arg(delta: i32) -> String {
    if delta >= 0 {
        format!("{delta}%+")
    } else {
        format!("{}%-", delta.abs())
    }
}

/// Argumento de volume relativo para `pactl set-sink-volume` (`+5%` / `-5%`).
pub fn pactl_delta_arg(delta: i32) -> String {
    if delta >= 0 {
        format!("+{delta}%")
    } else {
        format!("-{}%", delta.abs())
    }
}

/// Executa um comando externo de forma assíncrona, retornando `Ok(())` apenas
/// quando ele existe e termina com sucesso.
async fn run_ok(cmd: &str, args: &[&str]) -> Result<(), String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{cmd} falhou (status {:?})", out.status.code()))
    }
}

/// Ajusta o brilho da tela em `delta` % (relativo) via `brightnessctl` e
/// devolve o novo percentual lido do sysfs. Degrada com `Err` quando não há
/// backlight controlável (ex.: monitor externo) ou o utilitário está ausente.
pub async fn adjust_brightness(delta: i32) -> Result<u8, String> {
    run_ok("brightnessctl", &["set", &delta_arg(delta)]).await?;
    read_brightness(Path::new("/sys/class/backlight"))
        .map(|r| (r * 100.0).round() as u8)
        .ok_or_else(|| "brilho indisponível".to_string())
}

/// Ajusta o brilho do teclado em `delta` %.
pub async fn adjust_kbd_brightness(delta: i32) -> Result<u8, String> {
    let applied = run_ok("brightnessctl", &["--device", "*kbd*", "set", &delta_arg(delta)]).await.is_ok()
        || run_ok("brightnessctl", &["--device", "*::kbd_backlight", "set", &delta_arg(delta)]).await.is_ok();
    
    // Fallback sysfs puro
    if !applied {
        if let Ok(entries) = std::fs::read_dir("/sys/class/leds") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.ends_with("::kbd_backlight") || name_str.contains("kbd") {
                    let base = entry.path();
                    if let (Ok(cur), Ok(max)) = (
                        std::fs::read_to_string(base.join("brightness")),
                        std::fs::read_to_string(base.join("max_brightness")),
                    ) {
                        if let (Ok(c), Ok(m)) = (cur.trim().parse::<f32>(), max.trim().parse::<f32>()) {
                            let mut step = m * (delta as f32 / 100.0);
                            if step.abs() < 1.0 { step = step.signum(); }
                            let new = (c + step).clamp(0.0, m);
                            let _ = std::fs::write(base.join("brightness"), (new as i32).to_string());
                        }
                    }
                }
            }
        }
    }
    
    read_kbd_backlight(Path::new("/sys/class/leds"))
        .map(|r| (r * 100.0).round() as u8)
        .ok_or_else(|| "teclado indisponível".to_string())
}


/// Ajusta o volume do sink padrão em `delta` % (relativo), tentando `wpctl`
/// (PipeWire), depois `amixer` (ALSA) e por fim `pactl` (PulseAudio). Devolve o
/// novo percentual lido de volta.
pub async fn adjust_volume(delta: i32) -> Result<u8, String> {
    let rel = delta_arg(delta);
    let applied = run_ok("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &rel])
        .await
        .is_ok()
        || run_ok("amixer", &["sset", "Master", &rel]).await.is_ok()
        || run_ok(
            "pactl",
            &["set-sink-volume", "@DEFAULT_SINK@", &pactl_delta_arg(delta)],
        )
        .await
        .is_ok();
    if !applied {
        return Err("nenhum backend de áudio disponível".to_string());
    }
    read_volume()
        .map(|v| (v.ratio() * 100.0).round() as u8)
        .ok_or_else(|| "volume indisponível".to_string())
}

/// Alterna o mudo do sink padrão (`wpctl`/`amixer`/`pactl`) e devolve o novo
/// estado (`true` = mudo).
pub async fn toggle_mute() -> Result<bool, String> {
    let applied = run_ok("wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
        .await
        .is_ok()
        || run_ok("amixer", &["sset", "Master", "toggle"]).await.is_ok()
        || run_ok("pactl", &["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
            .await
            .is_ok();
    if !applied {
        return Err("nenhum backend de áudio disponível".to_string());
    }
    read_volume()
        .map(|v| v.muted)
        .ok_or_else(|| "volume indisponível".to_string())
}

/// Aplica um perfil de energia via `busctl`/`dbus-send`/`powerprofilesctl set`
/// (com fallback para escrita do scaling governor no sysfs). `Ok(())` apenas
/// quando algum backend aceitou a mudança.
async fn apply_power_profile(profile: PowerProfile) -> Result<(), String> {
    // 1. Tenta busctl (D-Bus nativo do systemd).
    if run_ok(
        "busctl",
        &[
            "set-property",
            "net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "net.hadess.PowerProfiles",
            "ActiveProfile",
            "s",
            profile.id(),
        ],
    )
    .await
    .is_ok()
    {
        return Ok(());
    }

    // 2. Tenta dbus-send (D-Bus padrão).
    let variant = format!("variant:string:\"{}\"", profile.id());
    if run_ok(
        "dbus-send",
        &[
            "--system",
            "--print-reply",
            "--dest=net.hadess.PowerProfiles",
            "/net/hadess/PowerProfiles",
            "org.freedesktop.DBus.Properties.Set",
            "string:net.hadess.PowerProfiles",
            "string:ActiveProfile",
            &variant,
        ],
    )
    .await
    .is_ok()
    {
        return Ok(());
    }

    // 3. Tenta CLI powerprofilesctl.
    for bin in ["powerprofilesctl", "/usr/sbin/powerprofilesctl"] {
        if run_ok(bin, &["set", profile.id()]).await.is_ok() {
            return Ok(());
        }
    }

    // 4. Fallback sysfs governor.
    if write_scaling_governor(profile.governor()) {
        return Ok(());
    }

    Err("nenhum backend de perfil de energia disponível".to_string())
}

/// Escreve `governor` em todos os `cpu*/cpufreq/scaling_governor` legíveis.
/// Retorna `true` se ao menos uma CPU aceitou a escrita (exige privilégio).
fn write_scaling_governor(governor: &str) -> bool {
    let base = Path::new("/sys/devices/system/cpu");
    let Ok(entries) = std::fs::read_dir(base) else {
        return false;
    };
    let mut applied = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // Aceita apenas cpuN (evita cpufreq/, cpuidle/, ...).
        if !(name.starts_with("cpu") && name[3..].chars().all(|c| c.is_ascii_digit()))
            || name.len() <= 3
        {
            continue;
        }
        let path = entry.path().join("cpufreq/scaling_governor");
        if std::fs::write(&path, governor).is_ok() {
            applied = true;
        }
    }
    applied
}

/// Alterna o Modo Avião (desliga Wi-Fi e Bluetooth se um deles estiver ligado,
/// liga caso contrário). Retorna `true` se o modo avião ficou ATIVO (rádios OFF).
pub async fn toggle_airplane_mode() -> Result<bool, String> {
    let mut wifi_on = false;
    if let Ok(out) = std::process::Command::new("nmcli").args(["radio", "wifi"]).output() {
        if String::from_utf8_lossy(&out.stdout).contains("enabled") {
            wifi_on = true;
        }
    }
    
    let mut bt_on = false;
    if let Ok(out) = std::process::Command::new("rfkill").args(["list", "bluetooth"]).output() {
        if !String::from_utf8_lossy(&out.stdout).contains("Soft blocked: yes") {
            bt_on = true;
        }
    }
    
    let turn_off = wifi_on || bt_on;
    
    if turn_off {
        let _ = run_ok("nmcli", &["radio", "wifi", "off"]).await;
        let _ = run_ok("rfkill", &["block", "bluetooth"]).await;
    } else {
        let _ = run_ok("nmcli", &["radio", "wifi", "on"]).await;
        let _ = run_ok("rfkill", &["unblock", "bluetooth"]).await;
    }
    
    Ok(turn_off)
}

/// Lê o perfil de energia atual, avança para o [`PowerProfile::next`] e o
/// aplica. Devolve o novo perfil ou uma mensagem de erro.
pub async fn cycle_power_profile() -> Result<PowerProfile, String> {
    let current = read_power_profile().unwrap_or(PowerProfile::Balanced);
    let next = current.next();
    apply_power_profile(next).await?;
    Ok(next)
}

/// Aplica uma ação de controle (brilho/volume) e emite o toast correspondente.
/// Retorna `true` quando um ajuste foi tentado (exigindo snapshot imediato).
async fn apply_control(action: &Action, tx: &EventTx) -> bool {
    let toast = match action {
        Action::BrightnessUp => match adjust_brightness(CONTROL_STEP).await {
            Ok(p) => Toast::info(format!("Brilho: {p}%")),
            Err(e) => Toast::error(format!("Brilho: {e}")),
        },
        Action::BrightnessDown => match adjust_brightness(-CONTROL_STEP).await {
            Ok(p) => Toast::info(format!("Brilho: {p}%")),
            Err(e) => Toast::error(format!("Brilho: {e}")),
        },
        Action::VolumeUp => match adjust_volume(CONTROL_STEP).await {
            Ok(p) => Toast::info(format!("Volume: {p}%")),
            Err(e) => Toast::error(format!("Volume: {e}")),
        },
        Action::VolumeDown => match adjust_volume(-CONTROL_STEP).await {
            Ok(p) => Toast::info(format!("Volume: {p}%")),
            Err(e) => Toast::error(format!("Volume: {e}")),
        },
        Action::ToggleMute => match toggle_mute().await {
            Ok(true) => Toast::info("Áudio: Mudo"),
            Ok(false) => Toast::info("Áudio: Ativo"),
            Err(e) => Toast::error(format!("Áudio: {e}")),
        },
        Action::CyclePowerProfile => match cycle_power_profile().await {
            Ok(p) => Toast::info(format!("Perfil de Energia: {}", p.label())),
            Err(e) => Toast::error(format!("Perfil de Energia: {e}")),
        },
        Action::KbdBrightnessUp => match adjust_kbd_brightness(CONTROL_STEP).await {
            Ok(p) => Toast::info(format!("[TECLADO] Brilho: {p}%")),
            Err(e) => Toast::error(format!("[TECLADO] Brilho: {e}")),
        },
        Action::KbdBrightnessDown => match adjust_kbd_brightness(-CONTROL_STEP).await {
            Ok(p) => Toast::info(format!("[TECLADO] Brilho: {p}%")),
            Err(e) => Toast::error(format!("[TECLADO] Brilho: {e}")),
        },
        Action::ToggleAirplaneMode => match toggle_airplane_mode().await {
            Ok(true) => Toast::info("[MODO AVIÃO] Rádios desativados (Wi-Fi & Bluetooth OFF)"),
            Ok(false) => Toast::info("[MODO AVIÃO] Rádios reativados (Wi-Fi & Bluetooth ON)"),
            Err(e) => Toast::error(format!("[MODO AVIÃO] Erro: {e}")),
        },
        _ => return false,
    };
    let _ = tx.send(AppEvent::Toast(toast));
    true
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
    let mut stat = StaticInfo {
        host_model: read_host_model(),
        packages: count_packages(),
        detail: read_static_detail(),
    };

    let (updates_tx, mut updates_rx) = tokio::sync::mpsc::channel::<usize>(1);
    let (trigger_tx, mut trigger_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        loop {
            let n = check_updates().await;
            let _ = updates_tx.send(n).await;
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(300)) => {}
                _ = trigger_rx.recv() => {}
            }
        }
    });

    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms.max(250)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let refresh = |sys: &mut System, disks: &mut Disks, stat: &StaticInfo| -> Box<SystemSnapshot> {
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        disks.refresh();
        let cpu_temp = read_cpu_temp(Path::new("/sys/class/thermal"));
        Box::new(SystemSnapshot::collect(sys, disks, stat, cpu_temp))
    };

    // Primeiro refresh estabelece a baseline de CPU; o valor de uso só é
    // significativo a partir do segundo tick.
    loop {
        tokio::select! {
            Some(n) = updates_rx.recv() => {
                if let Some(p) = &mut stat.packages {
                    p.pending_updates = Some(n);
                }
                let snap = refresh(&mut sys, &mut disks, &stat);
                let _ = tx.send(AppEvent::System(snap));
            }
            _ = ticker.tick() => {
                let snap = refresh(&mut sys, &mut disks, &stat);
                if tx.send(AppEvent::System(snap)).is_err() {
                    // App encerrou: nada a fazer.
                    break;
                }
            }
            res = actions.recv() => match res {
                Ok(action) => {
                    if action == Action::CheckUpdates {
                        let _ = trigger_tx.try_send(());
                        let pending = stat.packages.as_ref().and_then(|p| p.pending_updates);
                        let msg = match pending {
                            Some(0) => "Sistema atualizado.".to_string(),
                            Some(n) => format!("Existem {} atualizações pendentes. Execute a atualização no terminal.", n),
                            None => "Verificando atualizações...".to_string(),
                        };
                        let _ = tx.send(AppEvent::Toast(Toast::info(msg)));
                    }
                    // Ajustes de brilho/volume disparam um snapshot imediato
                    // para refletir o novo valor sem esperar o próximo tick.
                    if apply_control(&action, &tx).await {
                        let snap = refresh(&mut sys, &mut disks, &stat);
                        if tx.send(AppEvent::System(snap)).is_err() {
                            break;
                        }
                    }
                }
                // Perdemos mensagens por lag: seguimos no próximo tick.
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                // Todos os emissores sumiram: o app encerrou.
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    Ok(())
}

async fn check_updates() -> usize {
    let mut total = 0;
    if let Ok(out) = tokio::process::Command::new("checkupdates").output().await {
        if out.status.success() {
            total += String::from_utf8_lossy(&out.stdout).lines().count();
        } else if let Ok(out2) = tokio::process::Command::new("pacman").arg("-Qu").output().await {
            if out2.status.success() {
                total += String::from_utf8_lossy(&out2.stdout).lines().count();
            }
        }
    } else if let Ok(out2) = tokio::process::Command::new("pacman").arg("-Qu").output().await {
        if out2.status.success() {
            total += String::from_utf8_lossy(&out2.stdout).lines().count();
        }
    }
    if let Ok(out) = tokio::process::Command::new("flatpak").args(["remote-ls", "--updates"]).output().await {
        if out.status.success() {
            total += String::from_utf8_lossy(&out.stdout).lines().count();
        }
    }
    if std::path::Path::new("/usr/lib/update-notifier/apt-check").exists() {
        if let Ok(out) = tokio::process::Command::new("/usr/lib/update-notifier/apt-check").output().await {
            let s = String::from_utf8_lossy(&out.stderr);
            if let Some(num) = s.split(';').next() {
                if let Ok(n) = num.parse::<usize>() {
                    total += n;
                }
            }
        }
    } else if let Ok(out) = tokio::process::Command::new("apt-get").args(["-s", "upgrade"]).output().await {
        if out.status.success() {
            total += String::from_utf8_lossy(&out.stdout).lines().filter(|l| l.starts_with("Inst ")).count();
        }
    }
    total
}
