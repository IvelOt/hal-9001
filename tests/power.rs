use std::path::Path;

use hal9001::backend::power::BatteryBypass;

fn write_file(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

#[test]
fn probe_at_detects_universal_sysfs_threshold_and_reports_status() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.join("sys/class/power_supply/BAT0/charge_control_end_threshold");
    write_file(&path, "100\n");

    let bypass = BatteryBypass::probe_at(root).expect("universal sysfs knob should be found");
    assert!(!bypass.is_enabled());
}

#[test]
fn probe_at_detects_ideapad_conservation_mode() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.join("sys/devices/platform/ideapad_acpi/conservation_mode");
    write_file(&path, "1\n");

    let bypass = BatteryBypass::probe_at(root).expect("ideapad conservation_mode should be found");
    assert!(bypass.is_enabled());
}

#[test]
fn probe_at_returns_none_on_a_desktop_without_any_known_knob() {
    let dir = tempfile::tempdir().unwrap();
    assert!(BatteryBypass::probe_at(dir.path()).is_none());
}

#[test]
fn toggle_writes_through_sysfs_and_flips_reported_status() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let path = root.join("sys/devices/platform/samsung-laptop/battery_life_extender");
    write_file(&path, "0\n");

    let bypass = BatteryBypass::probe_at(root).unwrap();
    assert!(!bypass.is_enabled());

    let enabled = bypass.toggle().expect("writable sysfs file should toggle");
    assert!(enabled);
    assert!(bypass.is_enabled());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "1");

    let enabled = bypass
        .toggle()
        .expect("writable sysfs file should toggle back");
    assert!(!enabled);
    assert!(!bypass.is_enabled());
}
