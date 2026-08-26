//! Testes do Módulo 1 — coleta e parsing do Overview (neofetch).
//!
//! Exercitam parsers puros, cálculos de ratio, leitura via `/sys` simulado
//! (tempfile) e a degradação graciosa (fallback `N/A`) sem tocar hardware real.

use std::fs;

use hal9001::backend::system::{
    battery_health, brightness_ratio, count_installed_dpkg, count_nonempty_lines, delta_arg,
    pactl_delta_arg, parse_amixer_volume, parse_lspci_gpu, parse_thermal_temp, parse_wpctl_volume,
    ratio, read_battery, read_brightness, read_cpu_temp, BatteryStatus, Packages, CONTROL_STEP,
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

// --- controles interativos (brilho / volume) -----------------------------

#[test]
fn control_delta_args_relative_syntax() {
    // Passo padrão positivo/negativo no formato `brightnessctl`/`wpctl`/`amixer`.
    assert_eq!(delta_arg(CONTROL_STEP), "5%+");
    assert_eq!(delta_arg(-CONTROL_STEP), "5%-");
    assert_eq!(delta_arg(10), "10%+");
    assert_eq!(delta_arg(0), "0%+");

    // `pactl` usa o sinal como prefixo.
    assert_eq!(pactl_delta_arg(CONTROL_STEP), "+5%");
    assert_eq!(pactl_delta_arg(-CONTROL_STEP), "-5%");
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
fn battery_health_and_details_from_sysfs() {
    // saúde = full/design; ignora zero/negativos.
    assert!((battery_health(4400.0, 5000.0).unwrap() - 0.88).abs() < 1e-9);
    assert_eq!(battery_health(0.0, 5000.0), None);
    assert_eq!(battery_health(4400.0, 0.0), None);

    let dir = tempfile::tempdir().unwrap();
    let bat = dir.path().join("BAT0");
    fs::create_dir_all(&bat).unwrap();
    fs::write(bat.join("capacity"), "76\n").unwrap();
    fs::write(bat.join("status"), "Discharging\n").unwrap();
    fs::write(bat.join("energy_full"), "44000000\n").unwrap();
    fs::write(bat.join("energy_full_design"), "50000000\n").unwrap();
    fs::write(bat.join("cycle_count"), "212\n").unwrap();
    fs::write(bat.join("technology"), "Li-poly\n").unwrap();

    let b = read_battery(dir.path()).unwrap();
    assert!((b.health.unwrap() - 0.88).abs() < 1e-9);
    assert_eq!(b.cycle_count, Some(212));
    assert_eq!(b.technology.as_deref(), Some("Li-poly"));
}

// --- GPU / temperatura ---------------------------------------------------

#[test]
fn lspci_gpu_extraction() {
    let out = "\
00:00.0 Host bridge: Intel Corporation Device 1234
00:02.0 VGA compatible controller: Intel Corporation Raptor Lake-P [Iris Xe Graphics]
2e:00.0 3D controller: NVIDIA Corporation GA107M [GeForce RTX 3050]";
    // Pega a primeira controladora VGA/3D.
    assert_eq!(
        parse_lspci_gpu(out).as_deref(),
        Some("Intel Corporation Raptor Lake-P [Iris Xe Graphics]")
    );
    assert_eq!(parse_lspci_gpu("no gpu here"), None);
}

#[test]
fn thermal_temp_parsing_and_reading() {
    assert!((parse_thermal_temp("45000").unwrap() - 45.0).abs() < 1e-9);
    assert_eq!(parse_thermal_temp("999000"), None); // absurdo → descartado
    assert_eq!(parse_thermal_temp("abc"), None);

    let dir = tempfile::tempdir().unwrap();
    // Zona genérica primeiro, zona de CPU depois — deve preferir a de CPU.
    let z0 = dir.path().join("thermal_zone0");
    let z1 = dir.path().join("thermal_zone1");
    fs::create_dir_all(&z0).unwrap();
    fs::create_dir_all(&z1).unwrap();
    fs::write(z0.join("type"), "acpitz\n").unwrap();
    fs::write(z0.join("temp"), "40000\n").unwrap();
    fs::write(z1.join("type"), "x86_pkg_temp\n").unwrap();
    fs::write(z1.join("temp"), "55000\n").unwrap();

    assert!((read_cpu_temp(dir.path()).unwrap() - 55.0).abs() < 1e-9);
    assert_eq!(read_cpu_temp(std::path::Path::new("/no/such/zone")), None);
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
        pending_updates: None,
    };
    assert_eq!(p.summary(), "1234 (pacman+flatpak)");
    assert_eq!(Packages::default().summary(), "N/A");
}

#[test]
fn power_profile_parsing_and_cycling() {
    use hal9001::backend::system::PowerProfile;

    assert_eq!(PowerProfile::parse("power-saver"), Some(PowerProfile::PowerSaver));
    assert_eq!(PowerProfile::parse("powersave"), Some(PowerProfile::PowerSaver));
    assert_eq!(PowerProfile::parse("balanced"), Some(PowerProfile::Balanced));
    assert_eq!(PowerProfile::parse("schedutil"), Some(PowerProfile::Balanced));
    assert_eq!(PowerProfile::parse("performance"), Some(PowerProfile::Performance));
    assert_eq!(PowerProfile::parse("unknown-value"), None);

    assert_eq!(PowerProfile::PowerSaver.next(), PowerProfile::Balanced);
    assert_eq!(PowerProfile::Balanced.next(), PowerProfile::Performance);
    assert_eq!(PowerProfile::Performance.next(), PowerProfile::PowerSaver);

    assert_eq!(PowerProfile::PowerSaver.id(), "power-saver");
    assert_eq!(PowerProfile::Balanced.id(), "balanced");
    assert_eq!(PowerProfile::Performance.id(), "performance");

    assert_eq!(PowerProfile::PowerSaver.label(), "Economia");
    assert_eq!(PowerProfile::Balanced.label(), "Equilibrado");
    assert_eq!(PowerProfile::Performance.label(), "Desempenho");

    assert_eq!(PowerProfile::PowerSaver.tag(), "[POWER-SAVER]");
    assert_eq!(PowerProfile::Balanced.tag(), "[BALANCED]");
    assert_eq!(PowerProfile::Performance.tag(), "[PERFORMANCE]");
}
