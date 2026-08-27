use std::path::Path;
use std::time::Duration;

use sysinfo::{Disks, System};
use tokio::sync::broadcast;

use crate::events::{Action, AppEvent, EventTx, Toast};
use crate::i18n::{Language, SharedLang};

#[derive(Debug, Clone, Default)]
pub struct Packages {
    pub total: u64,

    pub by_manager: Vec<(&'static str, u64)>,

    pub pending_updates: Option<usize>,
}

impl Packages {
    pub fn summary(&self) -> String {
        if self.by_manager.is_empty() {
            return "N/A".into();
        }
        let names: Vec<&str> = self.by_manager.iter().map(|(n, _)| *n).collect();
        format!("{} ({})", self.total, names.join("+"))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Unknown,
}

impl BatteryStatus {
    pub fn parse(raw: &str) -> BatteryStatus {
        match raw.trim().to_ascii_lowercase().as_str() {
            "charging" => BatteryStatus::Charging,
            "discharging" => BatteryStatus::Discharging,
            "full" => BatteryStatus::Full,
            "not charging" => BatteryStatus::NotCharging,
            _ => BatteryStatus::Unknown,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            BatteryStatus::Charging => "CHARGING",
            BatteryStatus::Full => "FULL",
            BatteryStatus::Discharging => "DISCHARGING",
            BatteryStatus::NotCharging => "NOT CHARGING",
            BatteryStatus::Unknown => "UNKNOWN",
        }
    }

    pub fn power_sign(self) -> &'static str {
        match self {
            BatteryStatus::Charging => "+",
            BatteryStatus::Discharging => "-",
            _ => "",
        }
    }

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

#[derive(Debug, Clone, PartialEq)]
pub struct Battery {
    pub percent: f64,
    pub status: BatteryStatus,

    pub power_watts: Option<f64>,

    pub health: Option<f64>,

    pub cycle_count: Option<u64>,

    pub technology: Option<String>,
}

impl Battery {
    pub fn ratio(&self) -> f64 {
        (self.percent / 100.0).clamp(0.0, 1.0)
    }
}

pub fn battery_health(full: f64, design: f64) -> Option<f64> {
    if design <= 0.0 || full <= 0.0 {
        return None;
    }
    Some((full / design).clamp(0.0, 1.5))
}

#[derive(Debug, Clone, Copy)]
pub struct Volume {
    pub level: f64,
    pub muted: bool,
}

impl Volume {
    pub fn ratio(&self) -> f64 {
        self.level.clamp(0.0, 1.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerProfile {
    PowerSaver,

    Balanced,

    Performance,
}

impl PowerProfile {
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

    pub fn id(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "power-saver",
            PowerProfile::Balanced => "balanced",
            PowerProfile::Performance => "performance",
        }
    }

    pub fn label_in(self, lang: crate::i18n::Language) -> &'static str {
        let m = lang.messages();
        match self {
            PowerProfile::PowerSaver => m.profile_power_saver,
            PowerProfile::Balanced => m.profile_balanced,
            PowerProfile::Performance => m.profile_performance,
        }
    }

    pub fn label(self) -> &'static str {
        self.label_in(crate::i18n::Language::default())
    }

    pub fn tag(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "[POWER-SAVER]",
            PowerProfile::Balanced => "[BALANCED]",
            PowerProfile::Performance => "[PERFORMANCE]",
        }
    }

    pub fn governor(self) -> &'static str {
        match self {
            PowerProfile::PowerSaver => "powersave",
            PowerProfile::Balanced => "schedutil",
            PowerProfile::Performance => "performance",
        }
    }

    pub fn next(&self) -> PowerProfile {
        match self {
            PowerProfile::PowerSaver => PowerProfile::Balanced,
            PowerProfile::Balanced => PowerProfile::Performance,
            PowerProfile::Performance => PowerProfile::PowerSaver,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub host: String,
    pub user: String,
    pub shell: String,
    pub os: String,
    pub kernel: String,
    pub uptime_secs: u64,
    pub cpu_name: String,

    pub cpu_usage: f32,
    pub mem_used: u64,
    pub mem_total: u64,

    pub host_model: Option<String>,

    pub packages: Option<Packages>,

    pub brightness: Option<f64>,

    pub volume: Option<Volume>,

    pub battery: Option<Battery>,

    pub disk_used: Option<u64>,

    pub disk_total: Option<u64>,

    pub kbd_backlight: Option<f32>,

    pub power_profile: Option<PowerProfile>,

    pub detail: DetailInfo,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu_usage: f32,
    pub mem_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct DetailInfo {
    pub board_vendor: Option<String>,
    pub board_name: Option<String>,
    pub bios_version: Option<String>,
    pub bios_date: Option<String>,

    pub gpu: Option<String>,

    pub cpu_arch: Option<String>,
    pub cpu_cores_physical: Option<usize>,
    pub cpu_cores_logical: usize,
    pub cpu_freq_ghz: Option<f64>,
    pub cpu_temp_c: Option<f64>,

    pub swap_used: u64,
    pub swap_total: u64,

    pub desktop: Option<String>,
    pub session_type: Option<String>,

    pub top_processes: Vec<ProcessInfo>,
}

impl DetailInfo {
    pub fn swap_ratio(&self) -> f64 {
        ratio(self.swap_used, self.swap_total)
    }
}

#[derive(Debug, Clone, Default)]
struct StaticInfo {
    host_model: Option<String>,
    packages: Option<Packages>,

    detail: DetailInfo,
}

impl SystemSnapshot {
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

        let cpu_freq_mhz = sys.cpus().first().map(|c| c.frequency()).unwrap_or(0);
        let mut top_5: [Option<&sysinfo::Process>; 5] = [None, None, None, None, None];
        for p in sys.processes().values() {
            let mut current = Some(p);
            for t_ref in &mut top_5 {
                if let Some(c) = current {
                    if let Some(t) = *t_ref {
                        let cmp = c
                            .cpu_usage()
                            .partial_cmp(&t.cpu_usage())
                            .unwrap_or(std::cmp::Ordering::Equal)
                            .then_with(|| c.memory().cmp(&t.memory()));
                        if cmp == std::cmp::Ordering::Greater {
                            *t_ref = Some(c);
                            current = Some(t);
                        }
                    } else {
                        *t_ref = Some(c);
                        break;
                    }
                }
            }
        }
        let top_processes = top_5
            .into_iter()
            .flatten()
            .map(|p| ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu_usage: p.cpu_usage(),
                mem_bytes: p.memory(),
            })
            .collect();

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

    pub fn mem_ratio(&self) -> f64 {
        ratio(self.mem_used, self.mem_total)
    }

    pub fn cpu_ratio(&self) -> f64 {
        (self.cpu_usage as f64 / 100.0).clamp(0.0, 1.0)
    }

    pub fn disk_ratio(&self) -> Option<f64> {
        match (self.disk_used, self.disk_total) {
            (Some(u), Some(t)) => Some(ratio(u, t)),
            _ => None,
        }
    }
}

pub fn ratio(used: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        (used as f64 / total as f64).clamp(0.0, 1.0)
    }
}

fn read_host_model() -> Option<String> {
    let candidates = [
        "/sys/devices/virtual/dmi/id/product_name",
        "/sys/firmware/devicetree/base/model",
    ];
    for path in candidates {
        if let Ok(s) = std::fs::read_to_string(path) {
            let s = s.trim_matches(|c: char| c.is_whitespace() || c == '\0');
            if !s.is_empty() && s != "To be filled by O.E.M." && s != "System Product Name" {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn count_packages() -> Option<Packages> {
    let mut pkgs = Packages::default();

    if let Some(n) = count_lines("pacman", &["-Qq"]) {
        pkgs.by_manager.push(("pacman", n));
        pkgs.total += n;
    }

    if let Some(n) = count_dpkg() {
        pkgs.by_manager.push(("dpkg", n));
        pkgs.total += n;
    }

    if let Some(n) = count_lines("rpm", &["-qa"]) {
        pkgs.by_manager.push(("rpm", n));
        pkgs.total += n;
    }

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

fn count_lines(cmd: &str, args: &[&str]) -> Option<u64> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Some(count_nonempty_lines(&text))
}

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

fn read_dmi(dir: &Path, field: &str) -> Option<String> {
    let s = read_trimmed_file(&dir.join(field))?;
    if s == "To be filled by O.E.M." || s == "Default string" || s == "Unknown" {
        None
    } else {
        Some(s)
    }
}

fn detect_desktop() -> (Option<String>, Option<String>) {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
    let desktop = env("XDG_CURRENT_DESKTOP")
        .or_else(|| env("DESKTOP_SESSION"))
        .or_else(|| env("XDG_SESSION_DESKTOP"))
        .or_else(detect_wm_process);
    (desktop, env("XDG_SESSION_TYPE"))
}

fn detect_wm_process() -> Option<String> {
    for wm in [
        "sway", "i3", "hyprland", "bspwm", "dwm", "awesome", "xmonad",
    ] {
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

fn read_gpu() -> Option<String> {
    let out = std::process::Command::new("lspci").output().ok()?;
    if !out.status.success() {
        return None;
    }
    parse_lspci_gpu(&String::from_utf8_lossy(&out.stdout))
}

pub fn parse_lspci_gpu(output: &str) -> Option<String> {
    for line in output.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("vga compatible controller")
            || lower.contains("3d controller")
            || lower.contains("display controller")
        {
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

pub fn parse_thermal_temp(milli: &str) -> Option<f64> {
    let raw: f64 = milli.trim().parse().ok()?;
    let c = raw / 1000.0;
    if (0.0..=150.0).contains(&c) {
        Some(c)
    } else {
        None
    }
}

pub fn count_nonempty_lines(text: &str) -> u64 {
    text.lines().filter(|l| !l.trim().is_empty()).count() as u64
}

pub fn count_installed_dpkg(text: &str) -> u64 {
    text.lines()
        .filter(|l| l.trim_start().starts_with("ii"))
        .count() as u64
}

pub fn brightness_ratio(current: &str, max: &str) -> Option<f64> {
    let cur: f64 = current.trim().parse().ok()?;
    let mx: f64 = max.trim().parse().ok()?;
    if mx <= 0.0 {
        return None;
    }
    Some((cur / mx).clamp(0.0, 1.0))
}

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

pub fn parse_wpctl_volume(output: &str) -> Option<Volume> {
    let rest = output.trim().strip_prefix("Volume:")?.trim();
    let muted = rest.contains("[MUTED]");
    let num = rest.split_whitespace().next()?;
    let level: f64 = num.parse().ok()?;
    Some(Volume { level, muted })
}

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

pub fn read_battery(dir: &Path) -> Option<Battery> {
    let entries = std::fs::read_dir(dir).ok()?;

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

    Some((ua * uv) / 1_000_000_000_000.0)
}

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

fn read_trimmed_file(path: &Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn read_u64_file(path: &Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

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

fn read_power_profile() -> Option<PowerProfile> {
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

    for bin in ["powerprofilesctl", "/usr/sbin/powerprofilesctl"] {
        if let Ok(out) = std::process::Command::new(bin).arg("get").output() {
            if out.status.success() {
                if let Some(p) = PowerProfile::parse(&String::from_utf8_lossy(&out.stdout)) {
                    return Some(p);
                }
            }
        }
    }

    read_governor_profile(Path::new(
        "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor",
    ))
}

pub fn read_governor_profile(path: &Path) -> Option<PowerProfile> {
    PowerProfile::parse(&read_trimmed_file(path)?)
}

pub const CONTROL_STEP: i32 = 5;

pub fn delta_arg(delta: i32) -> String {
    if delta >= 0 {
        format!("{delta}%+")
    } else {
        format!("{}%-", delta.abs())
    }
}

pub fn pactl_delta_arg(delta: i32) -> String {
    if delta >= 0 {
        format!("+{delta}%")
    } else {
        format!("-{}%", delta.abs())
    }
}

async fn run_ok(cmd: &str, args: &[&str], lang: Language) -> Result<(), String> {
    let out = tokio::process::Command::new(cmd)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("{cmd}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{cmd} {} (status {:?})",
            lang.messages().word_failed,
            out.status.code()
        ))
    }
}

pub async fn adjust_brightness(delta: i32, lang: Language) -> Result<u8, String> {
    run_ok("brightnessctl", &["set", &delta_arg(delta)], lang).await?;
    read_brightness(Path::new("/sys/class/backlight"))
        .map(|r| (r * 100.0).round() as u8)
        .ok_or_else(|| lang.messages().err_brightness_unavailable.to_string())
}

pub async fn adjust_kbd_brightness(delta: i32, lang: Language) -> Result<u8, String> {
    let applied = run_ok(
        "brightnessctl",
        &["--device", "*kbd*", "set", &delta_arg(delta)],
        lang,
    )
    .await
    .is_ok()
        || run_ok(
            "brightnessctl",
            &["--device", "*::kbd_backlight", "set", &delta_arg(delta)],
            lang,
        )
        .await
        .is_ok();

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
                        if let (Ok(c), Ok(m)) =
                            (cur.trim().parse::<f32>(), max.trim().parse::<f32>())
                        {
                            let mut step = m * (delta as f32 / 100.0);
                            if step != 0.0 && step.abs() < 1.0 {
                                step = step.signum();
                            }
                            let new = (c + step).clamp(0.0, m);
                            let _ =
                                std::fs::write(base.join("brightness"), (new as i32).to_string());
                        }
                    }
                }
            }
        }
    }

    read_kbd_backlight(Path::new("/sys/class/leds"))
        .map(|r| (r * 100.0).round() as u8)
        .ok_or_else(|| lang.messages().err_kbd_unavailable.to_string())
}

pub async fn adjust_volume(delta: i32, lang: Language) -> Result<u8, String> {
    let rel = delta_arg(delta);
    let applied = run_ok("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &rel], lang)
        .await
        .is_ok()
        || run_ok("amixer", &["sset", "Master", &rel], lang)
            .await
            .is_ok()
        || run_ok(
            "pactl",
            &["set-sink-volume", "@DEFAULT_SINK@", &pactl_delta_arg(delta)],
            lang,
        )
        .await
        .is_ok();
    if !applied {
        return Err(lang.messages().err_no_audio_backend.to_string());
    }
    read_volume()
        .map(|v| (v.ratio() * 100.0).round() as u8)
        .ok_or_else(|| lang.messages().err_volume_unavailable.to_string())
}

pub async fn toggle_mute(lang: Language) -> Result<bool, String> {
    let applied = run_ok(
        "wpctl",
        &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
        lang,
    )
    .await
    .is_ok()
        || run_ok("amixer", &["sset", "Master", "toggle"], lang)
            .await
            .is_ok()
        || run_ok(
            "pactl",
            &["set-sink-mute", "@DEFAULT_SINK@", "toggle"],
            lang,
        )
        .await
        .is_ok();
    if !applied {
        return Err(lang.messages().err_no_audio_backend.to_string());
    }
    read_volume()
        .map(|v| v.muted)
        .ok_or_else(|| lang.messages().err_volume_unavailable.to_string())
}

async fn apply_power_profile(profile: PowerProfile, lang: Language) -> Result<(), String> {
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
        lang,
    )
    .await
    .is_ok()
    {
        return Ok(());
    }

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
        lang,
    )
    .await
    .is_ok()
    {
        return Ok(());
    }

    for bin in ["powerprofilesctl", "/usr/sbin/powerprofilesctl"] {
        if run_ok(bin, &["set", profile.id()], lang).await.is_ok() {
            return Ok(());
        }
    }

    if write_scaling_governor(profile.governor()) {
        return Ok(());
    }

    Err(lang.messages().err_no_power_backend.to_string())
}

fn write_scaling_governor(governor: &str) -> bool {
    let base = Path::new("/sys/devices/system/cpu");
    let Ok(entries) = std::fs::read_dir(base) else {
        return false;
    };
    let mut applied = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };

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

pub async fn toggle_airplane_mode(lang: Language) -> Result<bool, String> {
    let mut wifi_on = false;
    if let Ok(out) = std::process::Command::new("nmcli")
        .args(["radio", "wifi"])
        .output()
    {
        if String::from_utf8_lossy(&out.stdout).contains("enabled") {
            wifi_on = true;
        }
    }

    let mut bt_on = false;
    if let Ok(out) = std::process::Command::new("rfkill")
        .args(["list", "bluetooth"])
        .output()
    {
        if !String::from_utf8_lossy(&out.stdout).contains("Soft blocked: yes") {
            bt_on = true;
        }
    }

    let turn_off = wifi_on || bt_on;

    if turn_off {
        let _ = run_ok("nmcli", &["radio", "wifi", "off"], lang).await;
        let _ = run_ok("rfkill", &["block", "bluetooth"], lang).await;
    } else {
        let _ = run_ok("nmcli", &["radio", "wifi", "on"], lang).await;
        let _ = run_ok("rfkill", &["unblock", "bluetooth"], lang).await;
    }

    Ok(turn_off)
}

pub async fn cycle_power_profile(lang: Language) -> Result<PowerProfile, String> {
    let current = read_power_profile().unwrap_or(PowerProfile::Balanced);
    let next = current.next();
    apply_power_profile(next, lang).await?;
    Ok(next)
}

async fn apply_control(action: &Action, lang: Language, tx: &EventTx) -> bool {
    let m = lang.messages();
    let toast = match action {
        Action::BrightnessUp => match adjust_brightness(CONTROL_STEP, lang).await {
            Ok(p) => Toast::info(format!("{}: {p}%", m.toast_brightness_prefix)),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_brightness_prefix)),
        },
        Action::BrightnessDown => match adjust_brightness(-CONTROL_STEP, lang).await {
            Ok(p) => Toast::info(format!("{}: {p}%", m.toast_brightness_prefix)),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_brightness_prefix)),
        },
        Action::VolumeUp => match adjust_volume(CONTROL_STEP, lang).await {
            Ok(p) => Toast::info(format!("{}: {p}%", m.toast_volume_prefix)),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_volume_prefix)),
        },
        Action::VolumeDown => match adjust_volume(-CONTROL_STEP, lang).await {
            Ok(p) => Toast::info(format!("{}: {p}%", m.toast_volume_prefix)),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_volume_prefix)),
        },
        Action::ToggleMute => match toggle_mute(lang).await {
            Ok(true) => Toast::info(m.toast_muted),
            Ok(false) => Toast::info(m.toast_unmuted),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_volume_prefix)),
        },
        Action::CyclePowerProfile => match cycle_power_profile(lang).await {
            Ok(p) => Toast::info(format!("{}: {}", m.toast_profile_prefix, p.label_in(lang))),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_profile_prefix)),
        },
        Action::KbdBrightnessUp => match adjust_kbd_brightness(CONTROL_STEP, lang).await {
            Ok(p) => Toast::info(format!("{}: {p}%", m.toast_kbd_brightness_prefix)),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_kbd_brightness_prefix)),
        },
        Action::KbdBrightnessDown => match adjust_kbd_brightness(-CONTROL_STEP, lang).await {
            Ok(p) => Toast::info(format!("{}: {p}%", m.toast_kbd_brightness_prefix)),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_kbd_brightness_prefix)),
        },
        Action::ToggleAirplaneMode => match toggle_airplane_mode(lang).await {
            Ok(true) => Toast::info(m.toast_airplane_off),
            Ok(false) => Toast::info(m.toast_airplane_on),
            Err(e) => Toast::error(format!("{}: {e}", m.toast_airplane_error_prefix)),
        },
        _ => return false,
    };
    let _ = tx.send(AppEvent::Toast(toast));
    true
}

pub async fn run(
    poll_ms: u64,
    lang: SharedLang,
    tx: EventTx,
    mut actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    let mut sys = System::new_all();
    let mut disks = Disks::new_with_refreshed_list();

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

    loop {
        tokio::select! {
            Some(n) = updates_rx.recv() => {
                let prev = stat.packages.as_ref().and_then(|p| p.pending_updates);
                if let Some(p) = &mut stat.packages {
                    p.pending_updates = Some(n);
                }
                if n > 0 && prev != Some(n) {
                    let msg = lang.messages().toast_updates_available.replace("{n}", &n.to_string());
                    let _ = tx.send(AppEvent::Toast(Toast::warn(msg)));
                }
                let snap = refresh(&mut sys, &mut disks, &stat);
                let _ = tx.send(AppEvent::System(snap));
            }
            _ = ticker.tick() => {
                let snap = refresh(&mut sys, &mut disks, &stat);
                if tx.send(AppEvent::System(snap)).is_err() {

                    break;
                }
            }
            res = actions.recv() => match res {
                Ok(action) => {
                    if action == Action::CheckUpdates {
                        let _ = trigger_tx.try_send(());
                        let pending = stat.packages.as_ref().and_then(|p| p.pending_updates);
                        let m = lang.messages();
                        let msg = match pending {
                            Some(0) => m.toast_system_updated.to_string(),
                            Some(n) => m.toast_updates_pending.replace("{n}", &n.to_string()),
                            None => m.toast_checking_updates.to_string(),
                        };
                        let _ = tx.send(AppEvent::Toast(Toast::info(msg)));
                    }

                    if apply_control(&action, lang.get(), &tx).await {
                        let snap = refresh(&mut sys, &mut disks, &stat);
                        if tx.send(AppEvent::System(snap)).is_err() {
                            break;
                        }
                    }
                }

                Err(broadcast::error::RecvError::Lagged(_)) => {}

                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    Ok(())
}

async fn check_updates() -> usize {
    let mut total = 0;
    let mut arch_checked = false;
    if let Ok(out) = tokio::process::Command::new("checkupdates").output().await {
        if out.status.success() {
            total += String::from_utf8_lossy(&out.stdout).lines().count();
            arch_checked = true;
        }
    }
    if !arch_checked {
        if let Ok(out) = tokio::process::Command::new("pacman")
            .arg("-Qu")
            .output()
            .await
        {
            if out.status.success() {
                total += String::from_utf8_lossy(&out.stdout).lines().count();
            }
        }
    }
    if let Ok(out) = tokio::process::Command::new("flatpak")
        .args(["remote-ls", "--updates"])
        .output()
        .await
    {
        if out.status.success() {
            total += String::from_utf8_lossy(&out.stdout).lines().count();
        }
    }
    if std::path::Path::new("/usr/lib/update-notifier/apt-check").exists() {
        if let Ok(out) = tokio::process::Command::new("/usr/lib/update-notifier/apt-check")
            .output()
            .await
        {
            let s = String::from_utf8_lossy(&out.stderr);
            if let Some(num) = s.split(';').next() {
                if let Ok(n) = num.parse::<usize>() {
                    total += n;
                }
            }
        }
    } else if let Ok(out) = tokio::process::Command::new("apt-get")
        .args(["-s", "upgrade"])
        .output()
        .await
    {
        if out.status.success() {
            total += String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.starts_with("Inst "))
                .count();
        }
    }
    total
}
