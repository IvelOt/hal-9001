//! Testes do Módulo 1 — coleta e parsing do Overview (neofetch).
//!
//! Exercitam parsers puros, cálculos de ratio, leitura via `/sys` simulado
//! (tempfile) e a degradação graciosa (fallback `N/A`) sem tocar hardware real.

use std::fs;

use hal9001::backend::system::{
    brightness_ratio, count_installed_dpkg, count_nonempty_lines, parse_amixer_volume,
    parse_wpctl_volume, ratio, read_battery, read_brightness, BatteryStatus, Packages,
};

// --- ratios --------------------------------------------------------------

#[test]
fn ratio_is_clamped_and_zero_safe() {
    assert_eq!(ratio(0, 0), 0.0);
    assert_eq!(ratio(1, 0), 0.0);
    assert!((ratio(1, 2) - 0.5).abs() < 1e-9);
    assert_eq!(ratio(10, 4), 1.0); // clamp em 1.0
}

// --- brilho --------------------------------------------------------------

#[test]
fn brightness_ratio_parses_and_guards() {
    assert!((brightness_ratio("120", "240").unwrap() - 0.5).abs() < 1e-9);
    assert_eq!(brightness_ratio("300", "240").unwrap(), 1.0);
    assert_eq!(brightness_ratio("100", "0"), None); // max inválido
    assert_eq!(brightness_ratio("abc", "240"), None); // não numérico
}

#[test]
fn read_brightness_from_sysfs_layout() {
    let dir = tempfile::tempdir().unwrap();
    let bl = dir.path().join("intel_backlight");
    fs::create_dir_all(&bl).unwrap();
    fs::write(bl.join("brightness"), "480\n").unwrap();
    fs::write(bl.join("max_brightness"), "960\n").unwrap();

    let r = read_brightness(dir.path()).unwrap();
    assert!((r - 0.5).abs() < 1e-9);
}

#[test]
fn read_brightness_missing_is_none() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_brightness(dir.path()), None);
    assert_eq!(read_brightness(std::path::Path::new("/no/such/path")), None);
}

// --- volume --------------------------------------------------------------

#[test]
fn wpctl_volume_variants() {
    let v = parse_wpctl_volume("Volume: 0.65\n").unwrap();
    assert!((v.level - 0.65).abs() < 1e-9);
    assert!(!v.muted);

    let m = parse_wpctl_volume("Volume: 0.30 [MUTED]").unwrap();
    assert!((m.level - 0.30).abs() < 1e-9);
    assert!(m.muted);

    assert_eq!(v.ratio(), 0.65);
    assert!(parse_wpctl_volume("garbage").is_none());
}

#[test]
fn amixer_volume_parsing() {
    let out = "  Front Left: Playback 32768 [65%] [on]\n  Front Right: Playback 32768 [65%] [on]";
    let v = parse_amixer_volume(out).unwrap();
    assert!((v.level - 0.65).abs() < 1e-9);
    assert!(!v.muted);

    let muted = "  Mono: Playback 0 [0%] [off]";
    let mv = parse_amixer_volume(muted).unwrap();
    assert!(mv.muted);
    assert_eq!(mv.level, 0.0);
}

// --- bateria -------------------------------------------------------------

#[test]
fn battery_status_parsing() {
    assert_eq!(BatteryStatus::parse("Charging"), BatteryStatus::Charging);
    assert_eq!(BatteryStatus::parse("discharging"), BatteryStatus::Discharging);
    assert_eq!(BatteryStatus::parse("Full"), BatteryStatus::Full);
    assert_eq!(BatteryStatus::parse("Not charging"), BatteryStatus::NotCharging);
    assert_eq!(BatteryStatus::parse("weird"), BatteryStatus::Unknown);
}

#[test]
fn read_battery_from_sysfs_with_power() {
    let dir = tempfile::tempdir().unwrap();
    let bat = dir.path().join("BAT0");
    fs::create_dir_all(&bat).unwrap();
    fs::write(bat.join("capacity"), "82\n").unwrap();
    fs::write(bat.join("status"), "Charging\n").unwrap();
    fs::write(bat.join("power_now"), "12500000\n").unwrap(); // 12.5 W

    let b = read_battery(dir.path()).unwrap();
    assert_eq!(b.percent, 82.0);
    assert_eq!(b.status, BatteryStatus::Charging);
    assert!((b.power_watts.unwrap() - 12.5).abs() < 1e-6);
    assert!((b.ratio() - 0.82).abs() < 1e-9);
}

#[test]
fn read_battery_power_from_current_voltage() {
    let dir = tempfile::tempdir().unwrap();
    let bat = dir.path().join("BAT0");
    fs::create_dir_all(&bat).unwrap();
    fs::write(bat.join("capacity"), "50").unwrap();
    fs::write(bat.join("status"), "Discharging").unwrap();
    // 1.0 A * 12.0 V = 12 W → em µA·µV.
    fs::write(bat.join("current_now"), "1000000").unwrap();
    fs::write(bat.join("voltage_now"), "12000000").unwrap();

    let b = read_battery(dir.path()).unwrap();
    assert!((b.power_watts.unwrap() - 12.0).abs() < 1e-6);
}

#[test]
fn read_battery_absent_is_none() {
    let dir = tempfile::tempdir().unwrap();
    // Só há um AC adapter, sem BAT*.
    fs::create_dir_all(dir.path().join("ADP0")).unwrap();
    assert_eq!(read_battery(dir.path()), None);
}

// --- pacotes -------------------------------------------------------------

#[test]
fn package_line_counters() {
    assert_eq!(count_nonempty_lines("a\nb\n\nc\n"), 3);
    let dpkg = "ii  bash\nrc  removed-pkg\nii  coreutils\n";
    assert_eq!(count_installed_dpkg(dpkg), 2);
}

#[test]
fn packages_summary_formats() {
    let p = Packages {
        total: 1234,
        by_manager: vec![("pacman", 1200), ("flatpak", 34)],
    };
    assert_eq!(p.summary(), "1234 (pacman+flatpak)");
    assert_eq!(Packages::default().summary(), "N/A");
}
