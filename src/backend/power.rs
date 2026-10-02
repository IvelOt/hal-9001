use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tokio::sync::broadcast;

use crate::events::{Action, EventTx};
use crate::i18n::{Language, SharedLang};

pub async fn run(
    lang: SharedLang,
    tx: EventTx,
    _actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    super::pending_stub("power", &lang, lang.messages().pending_module_power, tx).await
}

/// How a probed sysfs knob expresses "bypass on" vs. "bypass off".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BypassKind {
    /// A numeric charge-limit threshold: a low value caps charging
    /// (bypass/conservation on), `off` restores unrestricted charging.
    Threshold { on: &'static str, off: &'static str },
    /// A plain `0`/`1` conservation-mode flag.
    Binary,
}

/// An agnostic handle to whatever battery-bypass / conservation-mode knob
/// [`BatteryBypass::probe`] found on this host, if any.
#[derive(Debug, Clone)]
pub struct BatteryBypass {
    path: PathBuf,
    kind: BypassKind,
}

impl BatteryBypass {
    /// Probes the real filesystem for a supported knob. Returns `None` on
    /// desktops or laptops whose vendor driver exposes none of the known
    /// paths, so callers can skip rendering the feature entirely.
    pub fn probe() -> Option<Self> {
        Self::probe_at(Path::new("/"))
    }

    /// Cascade used by [`Self::probe`], parameterized over a root so tests
    /// can point it at a tempdir standing in for `/`.
    pub fn probe_at(root: &Path) -> Option<Self> {
        // 1. Universal kernel sysfs (ThinkPad, modern ASUS, Framework, Dell,
        // Huawei): /sys/class/power_supply/BAT*/charge_control_end_threshold
        // (or charge_control_limit_max).
        let power_supply = root.join("sys/class/power_supply");
        if let Some(path) = find_in_dir_with_prefix(
            &power_supply,
            "BAT",
            &["charge_control_end_threshold", "charge_control_limit_max"],
        ) {
            return Some(Self {
                path,
                kind: BypassKind::Threshold {
                    on: "60",
                    off: "100",
                },
            });
        }

        // 2. Lenovo IdeaPad / Yoga / Legion ACPI conservation mode.
        let ideapad_drivers = root.join("sys/bus/platform/drivers/ideapad_acpi");
        if let Some(path) = find_in_dir_with_prefix(&ideapad_drivers, "", &["conservation_mode"]) {
            return Some(Self {
                path,
                kind: BypassKind::Binary,
            });
        }
        let ideapad_direct = root.join("sys/devices/platform/ideapad_acpi/conservation_mode");
        if ideapad_direct.exists() {
            return Some(Self {
                path: ideapad_direct,
                kind: BypassKind::Binary,
            });
        }

        // 3. LG Gram battery care limit.
        let lg = root.join("sys/devices/platform/lg-laptop/battery_care_limit");
        if lg.exists() {
            return Some(Self {
                path: lg,
                kind: BypassKind::Threshold {
                    on: "80",
                    off: "100",
                },
            });
        }

        // 4. Samsung battery life extender.
        let samsung = root.join("sys/devices/platform/samsung-laptop/battery_life_extender");
        if samsung.exists() {
            return Some(Self {
                path: samsung,
                kind: BypassKind::Binary,
            });
        }

        // 5. Legacy ASUS WMI charge threshold.
        let asus = root.join("sys/devices/platform/asus-nb-wmi/charge_control_end_threshold");
        if asus.exists() {
            return Some(Self {
                path: asus,
                kind: BypassKind::Threshold {
                    on: "60",
                    off: "100",
                },
            });
        }

        None
    }

    /// Reads the current state straight from sysfs.
    pub fn is_enabled(&self) -> bool {
        let content = std::fs::read_to_string(&self.path).unwrap_or_default();
        let value = content.trim();
        match self.kind {
            BypassKind::Threshold { off, .. } => !value.is_empty() && value != off,
            BypassKind::Binary => value == "1",
        }
    }

    /// Flips the knob with a direct write and returns the new state. When
    /// the write is rejected (HAL-9001 normally runs unprivileged) this
    /// returns [`BypassError::PermissionDenied`] instead of elevating on its
    /// own: spawning `pkexec`/`sudo` without a TTY wedges the raw-mode TUI,
    /// so the caller asks for the password through the in-app sudo modal and
    /// finishes with [`apply_value_with_sudo`].
    pub fn toggle(&self) -> Result<bool, BypassError> {
        let enabling = !self.is_enabled();
        let value = self.target_value(enabling);
        match std::fs::write(&self.path, value) {
            Ok(()) => Ok(enabling),
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                Err(BypassError::PermissionDenied {
                    path: self.path.clone(),
                    target_value: value,
                    enabling,
                })
            }
            Err(e) => Err(BypassError::Failed(e.to_string())),
        }
    }

    fn target_value(&self, enabling: bool) -> &'static str {
        match self.kind {
            BypassKind::Threshold { on, off } => {
                if enabling {
                    on
                } else {
                    off
                }
            }
            BypassKind::Binary => {
                if enabling {
                    "1"
                } else {
                    "0"
                }
            }
        }
    }
}

/// Why [`BatteryBypass::toggle`] could not flip the knob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BypassError {
    /// The direct write needs root: retry with [`apply_value_with_sudo`]
    /// writing `target_value` to `path`, which yields the `enabling` state.
    PermissionDenied {
        path: PathBuf,
        target_value: &'static str,
        enabling: bool,
    },
    Failed(String),
}

impl std::fmt::Display for BypassError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PermissionDenied { path, .. } => {
                write!(f, "permission denied: {}", path.display())
            }
            Self::Failed(e) => f.write_str(e),
        }
    }
}

/// Outcome of an elevated write attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SudoWriteError {
    /// sudo rejected the password (or needs one and none was given).
    AuthFailed,
    Failed(String),
}

/// Writes `value` to `path` as root via `sudo -S`, feeding `password` on
/// stdin (the same pattern `storage.rs` uses). `-k` ignores any cached
/// credential so sudo always consumes the password line instead of handing
/// it to the command. The value travels as an argument rather than on
/// stdin so a wrong password is never followed by a second stdin line that
/// sudo would count as another failed attempt (pam_faillock).
pub fn apply_value_with_sudo(
    path: &Path,
    value: &str,
    password: &str,
) -> Result<(), SudoWriteError> {
    run_sudo_write(&["-k", "-S", "-p", ""], path, value, Some(password))
}

/// Same as [`apply_value_with_sudo`] but only succeeds when sudo already
/// holds a cached credential (`sudo -n`), so the password modal can be
/// skipped entirely.
pub fn apply_value_with_cached_sudo(path: &Path, value: &str) -> Result<(), SudoWriteError> {
    run_sudo_write(&["-n"], path, value, None)
}

fn run_sudo_write(
    sudo_flags: &[&str],
    path: &Path,
    value: &str,
    password: Option<&str>,
) -> Result<(), SudoWriteError> {
    let mut child = Command::new("sudo")
        .args(sudo_flags)
        .arg("--")
        .args(["sh", "-c", "printf '%s' \"$1\" > \"$2\"", "hal9001"])
        .arg(value)
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SudoWriteError::Failed(format!("sudo: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        if let Some(pw) = password {
            // A write error here means sudo already exited; its status and
            // stderr below carry the real reason.
            let _ = stdin.write_all(pw.as_bytes());
            let _ = stdin.write_all(b"\n");
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|e| SudoWriteError::Failed(format!("sudo: {e}")))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if crate::backend::storage::is_sudo_auth_failure(&stderr) {
        Err(SudoWriteError::AuthFailed)
    } else {
        Err(SudoWriteError::Failed(stderr))
    }
}

/// Toggles whatever battery-bypass knob is present on this host. Probes
/// fresh on every call since the check is a handful of cheap path lookups.
pub async fn toggle_bypass(lang: Language) -> Result<bool, BypassError> {
    let bypass = BatteryBypass::probe().ok_or_else(|| {
        BypassError::Failed(lang.messages().err_battery_bypass_unavailable.to_string())
    })?;
    tokio::task::spawn_blocking(move || bypass.toggle())
        .await
        .map_err(|e| BypassError::Failed(e.to_string()))?
}

fn find_in_dir_with_prefix(dir: &Path, prefix: &str, filenames: &[&str]) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_string_lossy().starts_with(prefix) {
            continue;
        }
        for filename in filenames {
            let candidate = entry.path().join(filename);
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn probe_finds_universal_kernel_sysfs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/class/power_supply/BAT0/charge_control_end_threshold"),
            "100\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find BAT0 threshold");
        assert!(!bypass.is_enabled());
        assert_eq!(
            bypass.kind,
            BypassKind::Threshold {
                on: "60",
                off: "100"
            }
        );
    }

    #[test]
    fn probe_finds_charge_control_limit_max_variant() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/class/power_supply/BAT1/charge_control_limit_max"),
            "60\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find BAT1 limit_max");
        assert!(bypass.is_enabled());
    }

    #[test]
    fn probe_finds_ideapad_conservation_mode_under_glob_dir() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/bus/platform/drivers/ideapad_acpi/VPC2004:00/conservation_mode"),
            "1\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find ideapad conservation_mode");
        assert!(bypass.is_enabled());
        assert_eq!(bypass.kind, BypassKind::Binary);
    }

    #[test]
    fn probe_finds_ideapad_conservation_mode_direct_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/devices/platform/ideapad_acpi/conservation_mode"),
            "0\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find direct conservation_mode");
        assert!(!bypass.is_enabled());
    }

    #[test]
    fn probe_finds_lg_gram_battery_care_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/devices/platform/lg-laptop/battery_care_limit"),
            "80\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find LG Gram knob");
        assert!(bypass.is_enabled());
        assert_eq!(
            bypass.kind,
            BypassKind::Threshold {
                on: "80",
                off: "100"
            }
        );
    }

    #[test]
    fn probe_finds_samsung_battery_life_extender() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/devices/platform/samsung-laptop/battery_life_extender"),
            "1\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find Samsung knob");
        assert!(bypass.is_enabled());
    }

    #[test]
    fn probe_finds_legacy_asus_wmi_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/devices/platform/asus-nb-wmi/charge_control_end_threshold"),
            "100\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find legacy ASUS WMI knob");
        assert!(!bypass.is_enabled());
    }

    #[test]
    fn probe_prefers_universal_sysfs_over_vendor_specific() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_file(
            &root.join("sys/class/power_supply/BAT0/charge_control_end_threshold"),
            "60\n",
        );
        write_file(
            &root.join("sys/devices/platform/samsung-laptop/battery_life_extender"),
            "0\n",
        );

        let bypass = BatteryBypass::probe_at(root).expect("should find a knob");
        assert_eq!(
            bypass.kind,
            BypassKind::Threshold {
                on: "60",
                off: "100"
            }
        );
    }

    #[test]
    fn probe_returns_none_when_no_known_path_exists() {
        let dir = tempfile::tempdir().unwrap();
        assert!(BatteryBypass::probe_at(dir.path()).is_none());
    }

    #[test]
    fn toggle_flips_threshold_value_on_a_writable_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("sys/class/power_supply/BAT0/charge_control_end_threshold");
        write_file(&path, "100\n");

        let bypass = BatteryBypass::probe_at(root).unwrap();
        assert!(!bypass.is_enabled());

        let enabled = bypass.toggle().expect("direct write should succeed");
        assert!(enabled);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "60");
        assert!(bypass.is_enabled());

        let enabled = bypass.toggle().expect("direct write should succeed");
        assert!(!enabled);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "100");
    }

    #[test]
    fn toggle_reports_permission_denied_instead_of_elevating() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("sys/class/power_supply/BAT0/charge_control_end_threshold");
        write_file(&path, "100\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        if std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
            // Running as root: permission bits are not enforced.
            return;
        }

        let bypass = BatteryBypass::probe_at(root).unwrap();
        assert_eq!(
            bypass.toggle(),
            Err(BypassError::PermissionDenied {
                path: path.clone(),
                target_value: "60",
                enabling: true,
            })
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "100\n");
    }

    #[test]
    fn toggle_flips_binary_value_on_a_writable_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let path = root.join("sys/devices/platform/ideapad_acpi/conservation_mode");
        write_file(&path, "0\n");

        let bypass = BatteryBypass::probe_at(root).unwrap();
        let enabled = bypass.toggle().expect("direct write should succeed");
        assert!(enabled);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "1");
    }
}
