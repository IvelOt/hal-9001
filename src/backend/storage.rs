
use std::collections::HashMap;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use sysinfo::Disks;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::Connection;

use crate::events::{
    Action, AppEvent, DeviceId, EventTx, SudoPasswordRequest, SudoPasswordTx, Toast,
};

const IO_BUFFER_SIZE: usize = 4 * 1024 * 1024;

const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);

const UDISKS_SERVICE: &str = "org.freedesktop.UDisks2";
const UDISKS_ROOT: &str = "/org/freedesktop/UDisks2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsKind {
    Ext4,
    Vfat,
    Exfat,
    Ntfs,
    Btrfs,
    CryptoLuks,
    Swap,
    Other(String),
    Unknown,
}

impl FsKind {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "" => FsKind::Unknown,
            "ext4" => FsKind::Ext4,
            "vfat" => FsKind::Vfat,
            "exfat" => FsKind::Exfat,
            "ntfs" => FsKind::Ntfs,
            "btrfs" => FsKind::Btrfs,
            "crypto_luks" => FsKind::CryptoLuks,
            "swap" | "linux_raid_member" if raw.eq_ignore_ascii_case("swap") => FsKind::Swap,
            other => FsKind::Other(other.to_string()),
        }
    }

    pub fn label(&self) -> &str {
        match self {
            FsKind::Ext4 => "ext4",
            FsKind::Vfat => "vfat",
            FsKind::Exfat => "exfat",
            FsKind::Ntfs => "ntfs",
            FsKind::Btrfs => "btrfs",
            FsKind::CryptoLuks => "crypto_LUKS",
            FsKind::Swap => "swap",
            FsKind::Other(s) => s.as_str(),
            FsKind::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusType {
    Usb,
    Sata,
    Nvme,
    Scsi,
    Mmc,
    Unknown,
}

impl BusType {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "usb" => BusType::Usb,
            "ata" | "sata" => BusType::Sata,
            "nvme" => BusType::Nvme,
            "scsi" => BusType::Scsi,
            "sdio" | "mmc" => BusType::Mmc,
            _ => BusType::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BusType::Usb => "USB",
            BusType::Sata => "SATA",
            BusType::Nvme => "NVMe",
            BusType::Scsi => "SCSI",
            BusType::Mmc => "MMC",
            BusType::Unknown => "?",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartitionInfo {
    pub id: DeviceId,
    pub dev_node: String,
    pub label: String,
    pub fs: FsKind,
    pub size: u64,

    pub used: Option<u64>,
    pub mount_points: Vec<String>,

    pub is_swap: bool,

    pub is_system: bool,
}

impl PartitionInfo {
    pub fn is_mounted(&self) -> bool {
        !self.mount_points.is_empty()
    }

    pub fn usage_ratio(&self) -> Option<f64> {
        let used = self.used? as f64;
        if self.size == 0 {
            return None;
        }
        Some((used / self.size as f64).clamp(0.0, 1.0))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DriveInfo {
    pub id: DeviceId,
    pub dev_node: String,

    pub block_path: Option<String>,
    pub model: String,
    pub vendor: String,
    pub size: u64,
    pub removable: bool,
    pub ejectable: bool,

    pub can_power_off: bool,
    pub bus: BusType,

    pub rotational: bool,

    pub is_system: bool,

    pub is_ventoy: bool,
    pub partitions: Vec<PartitionInfo>,
}

impl DriveInfo {

    pub fn friendly_label(&self) -> String {
        if let Some(p) = primary_partition(self) {
            if !p.label.trim().is_empty() {
                return p.label.clone();
            }
        }
        let vm = format!("{} {}", self.vendor, self.model).trim().to_string();
        if !vm.is_empty() {

            if self.bus == BusType::Mmc {
                return format!("{vm} ({})", self.dev_node);
            }
            return vm;
        }
        self.dev_node.clone()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct StorageSnapshot {
    pub udisks_available: bool,
    pub drives: Vec<DriveInfo>,
}

impl StorageSnapshot {
    pub fn drive(&self, idx: usize) -> Option<&DriveInfo> {
        self.drives.get(idx)
    }

    pub fn partition(&self, drive_idx: usize, part_idx: usize) -> Option<&PartitionInfo> {
        self.drives.get(drive_idx)?.partitions.get(part_idx)
    }

    pub fn partition_by_id(&self, id: &DeviceId) -> Option<&PartitionInfo> {
        self.drives
            .iter()
            .flat_map(|d| &d.partitions)
            .find(|p| &p.id == id)
    }

    pub fn is_system_target(&self, id: &DeviceId) -> bool {
        self.drives.iter().any(|d| &d.id == id && d.is_system)
            || self
                .drives
                .iter()
                .flat_map(|d| &d.partitions)
                .any(|p| &p.id == id && p.is_system)
    }

    pub fn drive_dev_node(&self, id: &DeviceId) -> Option<String> {
        self.drives
            .iter()
            .find(|d| &d.id == id)
            .map(|d| d.dev_node.clone())
    }

    pub fn drive_size(&self, id: &DeviceId) -> Option<u64> {
        self.drives.iter().find(|d| &d.id == id).map(|d| d.size)
    }

    pub fn drive_by_id<'a>(&'a self, id: &DeviceId) -> Option<&'a DriveInfo> {
        self.drives.iter().find(|d| &d.id == id)
    }

    pub fn dev_node_for_block_path(&self, block_path: &str) -> Option<String> {
        for drive in &self.drives {
            if drive.block_path.as_deref() == Some(block_path) {
                return Some(drive.dev_node.clone());
            }
            if let Some(p) = drive.partitions.iter().find(|p| p.id.0 == block_path) {
                return Some(p.dev_node.clone());
            }
        }
        None
    }
}

const BLOCK_DEVICE_PREFIX: &str = "/org/freedesktop/UDisks2/block_devices/";

const DRIVE_PREFIX: &str = "/org/freedesktop/UDisks2/drives/";

pub fn resolve_block_object_path(
    snapshot: &StorageSnapshot,
    target_id: &DeviceId,
) -> Option<String> {
    if target_id.0.starts_with(BLOCK_DEVICE_PREFIX) {
        return Some(target_id.0.clone());
    }
    if target_id.0.starts_with(DRIVE_PREFIX) {
        return snapshot
            .drives
            .iter()
            .find(|d| &d.id == target_id)
            .and_then(|d| d.block_path.clone());
    }
    None
}

fn mounted_partition_paths(snapshot: &StorageSnapshot, target_id: &DeviceId) -> Vec<String> {
    if let Some(drive) = snapshot.drive_by_id(target_id) {
        return drive
            .partitions
            .iter()
            .filter(|p| p.is_mounted())
            .map(|p| p.id.0.clone())
            .collect();
    }
    for drive in &snapshot.drives {
        if let Some(p) = drive.partitions.iter().find(|p| &p.id == target_id) {
            if p.is_mounted() {
                return vec![p.id.0.clone()];
            }
        }
    }
    Vec::new()
}

fn mkfs_hint(fs_type: &str) -> Option<(&'static str, &'static str)> {
    match fs_type.trim().to_ascii_lowercase().as_str() {
        "vfat" | "fat32" | "fat" => Some(("mkfs.vfat", "dosfstools")),
        "exfat" => Some(("mkfs.exfat", "exfatprogs (ou exfat-utils)")),
        "ext4" => Some(("mkfs.ext4", "e2fsprogs")),
        "ext3" => Some(("mkfs.ext3", "e2fsprogs")),
        "ext2" => Some(("mkfs.ext2", "e2fsprogs")),
        "ntfs" => Some(("mkfs.ntfs", "ntfs-3g")),
        "btrfs" => Some(("mkfs.btrfs", "btrfs-progs")),
        _ => None,
    }
}

fn is_missing_mkfs_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_ascii_lowercase();
    lower.contains("not found")
        || lower.contains("no such file or directory")
        || lower.contains("command not found")
        || lower.contains("failed to execute")
}

fn format_error_message(fs_type: &str, err: &anyhow::Error) -> String {
    if is_missing_mkfs_error(err) {
        if let Some((bin, pkg)) = mkfs_hint(fs_type) {
            return format!("{bin} ausente — instale {pkg}");
        }
    }
    format!("Falha ao formatar: {err}")
}

pub fn is_permission_denied_error(err: &anyhow::Error) -> bool {
    if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
        if io_err.kind() == std::io::ErrorKind::PermissionDenied {
            return true;
        }
    }
    err.to_string()
        .to_ascii_lowercase()
        .contains("permission denied")
}

pub fn is_not_authorized_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_ascii_lowercase();
    lower.contains("notauthorized")
        || lower.contains("not authorized")
        || lower.contains("no polkit agent")
        || lower.contains("authentication is required")
}

pub fn mkfs_command(fs_type: &str, label: &str, dev_node: &str) -> Option<(String, Vec<String>)> {
    let (bin, _pkg) = mkfs_hint(fs_type)?;
    let mut args = Vec::new();
    match fs_type.trim().to_ascii_lowercase().as_str() {
        "vfat" | "fat32" | "fat" => {
            args.push("-F".to_string());
            args.push("32".to_string());
            if !label.is_empty() {
                args.push("-n".to_string());
                args.push(label.to_string());
            }
        }
        "exfat" => {
            if !label.is_empty() {
                args.push("-n".to_string());
                args.push(label.to_string());
            }
        }
        "ext4" | "ext3" | "ext2" => {
            args.push("-F".to_string());
            if !label.is_empty() {
                args.push("-L".to_string());
                args.push(label.to_string());
            }
        }
        "ntfs" => {
            args.push("-f".to_string());
            if !label.is_empty() {
                args.push("-L".to_string());
                args.push(label.to_string());
            }
        }
        "btrfs" => {
            args.push("-f".to_string());
            if !label.is_empty() {
                args.push("-L".to_string());
                args.push(label.to_string());
            }
        }
        _ => return None,
    }
    args.push(dev_node.to_string());
    Some((bin.to_string(), args))
}

async fn sudo_cached() -> bool {
    tokio::process::Command::new("sudo")
        .args(["-n", "true"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn request_sudo_password(
    sudo_tx: &SudoPasswordTx,
    label: &str,
    retry_error: Option<String>,
) -> Option<String> {
    let (respond, respond_rx) = tokio::sync::oneshot::channel();
    if sudo_tx
        .send(SudoPasswordRequest {
            label: label.to_string(),
            retry_error,
            respond,
        })
        .is_err()
    {
        return None;
    }
    respond_rx.await.ok().flatten()
}

pub fn is_sudo_auth_failure(stderr_text: &str) -> bool {
    let lower = stderr_text.to_ascii_lowercase();
    lower.contains("incorrect password")
        || lower.contains("sorry, try again")
        || lower.contains("senha incorreta")
        || lower.contains("no password was provided")
        || lower.contains("a password is required")
}

struct SudoRun {
    lines: tokio::sync::mpsc::UnboundedReceiver<String>,
    handle: tokio::task::JoinHandle<anyhow::Result<(std::process::ExitStatus, String)>>,
}

async fn read_stream_lines<R: tokio::io::AsyncRead + Unpin>(
    mut reader: R,
    line_tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> String {
    let mut buf = [0u8; 4096];
    let mut partial: Vec<u8> = Vec::new();
    let mut full = String::new();
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        for &b in &buf[..n] {
            if b == b'\n' || b == b'\r' {
                if !partial.is_empty() {
                    let line = String::from_utf8_lossy(&partial).to_string();
                    full.push_str(&line);
                    full.push('\n');
                    let _ = line_tx.send(line);
                    partial.clear();
                }
            } else {
                partial.push(b);
            }
        }
    }
    if !partial.is_empty() {
        let line = String::from_utf8_lossy(&partial).to_string();
        full.push_str(&line);
        let _ = line_tx.send(line);
    }
    full
}

fn spawn_sudo(
    password: Option<String>,
    program: String,
    args: Vec<String>,
) -> anyhow::Result<SudoRun> {
    let (line_tx, line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let handle = tokio::spawn(async move {
        let mut cmd = tokio::process::Command::new("sudo");
        if password.is_some() {
            cmd.arg("-S").arg("-k").arg("--");
        } else {
            cmd.arg("-n").arg("--");
        }
        cmd.arg(&program).args(&args);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn()?;

        if let Some(pw) = password {
            let mut stdin = child.stdin.take().expect("stdin piped");
            stdin.write_all(pw.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.shutdown().await?;
        } else {
            drop(child.stdin.take());
        }

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let (_stdout_text, stderr_text) = tokio::join!(
            read_stream_lines(stdout, line_tx.clone()),
            read_stream_lines(stderr, line_tx),
        );
        let status = child.wait().await?;
        Ok((status, stderr_text))
    });
    Ok(SudoRun {
        lines: line_rx,
        handle,
    })
}

pub fn sudo_invocation(cached: bool, program: &str, args: &[String]) -> Vec<String> {
    let mut v = Vec::new();
    if cached {
        v.push("-n".to_string());
    } else {
        v.push("-S".to_string());
        v.push("-k".to_string());
    }
    v.push("--".to_string());
    v.push(program.to_string());
    v.extend(args.iter().cloned());
    v
}

pub fn parse_dd_bytes_copied(line: &str) -> Option<u64> {
    let trimmed = line.trim_start();
    let digits: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = trimmed[digits.len()..].trim_start();
    if !rest.starts_with("byte") {
        return None;
    }
    digits.parse().ok()
}

async fn next_sudo_attempt(
    sudo_tx: &SudoPasswordTx,
    label: &str,
    retry_error: &mut Option<String>,
) -> Result<Option<String>, String> {
    if retry_error.is_none() && sudo_cached().await {
        return Ok(None);
    }
    match request_sudo_password(sudo_tx, label, retry_error.take()).await {
        Some(pw) => Ok(Some(pw)),
        None => Err("operação cancelada pelo usuário".to_string()),
    }
}

fn is_fat_fs_type(fs_type: &str) -> bool {
    matches!(
        fs_type.trim().to_ascii_lowercase().as_str(),
        "vfat" | "fat32" | "fat"
    )
}

fn fat_volume_label(label: &str) -> [u8; 11] {
    let mut buf = [b' '; 11];
    for (i, b) in label.bytes().take(11).enumerate() {
        buf[i] = b.to_ascii_uppercase();
    }
    buf
}

fn format_fat32_on_file(mut file: std::fs::File, label: &str) -> anyhow::Result<()> {
    let options = fatfs::FormatVolumeOptions::new()
        .fat_type(fatfs::FatType::Fat32)
        .volume_label(fat_volume_label(label));
    fatfs::format_volume(&mut file, options)?;
    file.sync_all()?;
    Ok(())
}

pub fn format_fat32_pure_rust(dev_node: &str, label: &str) -> anyhow::Result<()> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev_node)?;
    format_fat32_on_file(file, label)
}

#[derive(Debug, Clone, PartialEq)]
pub struct VentoyIsoEntry {
    pub name: String,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

pub fn detect_ventoy(partitions: &[PartitionInfo]) -> bool {
    partitions
        .iter()
        .any(|p| p.label.eq_ignore_ascii_case("ventoy") || p.label.eq_ignore_ascii_case("vtoyefi"))
}

pub fn ventoy_data_partition(drive: &DriveInfo) -> Option<&PartitionInfo> {
    if let Some(p) = drive
        .partitions
        .iter()
        .find(|p| p.label.eq_ignore_ascii_case("ventoy"))
    {
        return Some(p);
    }
    drive
        .partitions
        .iter()
        .filter(|p| !p.label.eq_ignore_ascii_case("vtoyefi"))
        .max_by_key(|p| p.size)
}

pub fn primary_partition(drive: &DriveInfo) -> Option<&PartitionInfo> {
    if let Some(p) = drive
        .partitions
        .iter()
        .find(|p| p.is_mounted() && !p.is_system)
    {
        return Some(p);
    }
    if let Some(p) = drive
        .partitions
        .iter()
        .filter(|p| !p.is_system)
        .max_by_key(|p| p.size)
    {
        return Some(p);
    }
    ventoy_data_partition(drive)
}

pub fn is_iso_or_img(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".iso") || lower.ends_with(".img")
}

pub fn build_ventoy_entries(
    raw: Vec<(String, u64, Option<std::time::SystemTime>)>,
) -> Vec<VentoyIsoEntry> {
    let mut entries: Vec<VentoyIsoEntry> = raw
        .into_iter()
        .filter(|(name, _, _)| is_iso_or_img(name))
        .map(|(name, size, modified)| VentoyIsoEntry {
            name,
            size,
            modified,
        })
        .collect();
    entries.sort_by_key(|a| a.name.to_ascii_lowercase());
    entries
}

pub fn compute_speed_eta(
    window_bytes: u64,
    window_secs: f64,
    bytes_written: u64,
    total_bytes: u64,
) -> (f64, u64) {
    let secs = window_secs.max(0.001);
    let speed_mbps = (window_bytes as f64 / 1_048_576.0) / secs;
    let remaining = total_bytes.saturating_sub(bytes_written);
    let eta_secs = if speed_mbps > 0.0 {
        (remaining as f64 / (speed_mbps * 1_048_576.0)).round() as u64
    } else {
        0
    };
    (speed_mbps, eta_secs)
}

pub fn is_system_disk(drive: &DriveInfo, partition: &PartitionInfo) -> bool {
    const PROTECTED_MOUNTS: [&str; 4] = ["/", "/boot", "/boot/efi", "/home"];

    let mounted_protected = partition.mount_points.iter().any(|mp| {
        let trimmed = mp.trim_end_matches('/');
        let normalized = if trimmed.is_empty() { "/" } else { trimmed };
        PROTECTED_MOUNTS.contains(&normalized)
    });

    let fixed_internal = !drive.removable && drive.bus != BusType::Usb;

    mounted_protected || partition.is_swap || fixed_internal
}

pub fn parse_proc_swaps(text: &str) -> Vec<String> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let dev = cols.next()?;
            let kind = cols.next()?;
            if kind.eq_ignore_ascii_case("partition") && dev.starts_with("/dev/") {
                Some(dev.to_string())
            } else {
                None
            }
        })
        .collect()
}

pub fn parse_proc_mounts(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut cols = line.split_whitespace();
            let dev = cols.next()?;
            let mp = cols.next()?;
            if dev.starts_with("/dev/") {
                Some((dev.to_string(), unescape_mount_octal(mp)))
            } else {
                None
            }
        })
        .collect()
}

fn unescape_mount_octal(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            if let Ok(code) = u8::from_str_radix(&s[i + 1..i + 4], 8) {
                out.push(code as char);
                i += 4;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

type PropMap = HashMap<String, OwnedValue>;
type IfaceMap = HashMap<String, PropMap>;
type ManagedObjects = HashMap<OwnedObjectPath, IfaceMap>;

fn prop_string(props: &PropMap, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
}

fn prop_u64(props: &PropMap, key: &str) -> Option<u64> {
    props.get(key).and_then(|v| u64::try_from(v.clone()).ok())
}

fn prop_i32(props: &PropMap, key: &str) -> Option<i32> {
    props.get(key).and_then(|v| i32::try_from(v.clone()).ok())
}

fn prop_bool(props: &PropMap, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| bool::try_from(v.clone()).ok())
}

fn prop_object_path(props: &PropMap, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| ObjectPath::try_from(v.clone()).ok())
        .map(|p| p.as_str().to_string())
}

fn prop_bytes_path(props: &PropMap, key: &str) -> Option<String> {
    let bytes = props
        .get(key)
        .and_then(|v| Vec::<u8>::try_from(v.clone()).ok())?;
    Some(bytes_to_path(&bytes))
}

fn bytes_to_path(bytes: &[u8]) -> String {
    let trimmed = bytes.split(|&b| b == 0).next().unwrap_or(bytes);
    String::from_utf8_lossy(trimmed).to_string()
}

fn prop_mount_points(props: &PropMap, key: &str) -> Vec<String> {
    props
        .get(key)
        .and_then(|v| Vec::<Vec<u8>>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .iter()
        .map(|b| bytes_to_path(b))
        .collect()
}

fn build_snapshot(objects: &ManagedObjects, swaps_text: &str, disks: &Disks) -> StorageSnapshot {
    let active_swaps = parse_proc_swaps(swaps_text);

    let mut drives: Vec<DriveInfo> = Vec::new();
    let mut drive_path_index: HashMap<String, usize> = HashMap::new();

    for (path, ifaces) in objects.iter() {
        let Some(drive_props) = ifaces.get("org.freedesktop.UDisks2.Drive") else {
            continue;
        };
        let rotation_rate = prop_i32(drive_props, "RotationRate").unwrap_or(-1);
        let drive = DriveInfo {
            id: DeviceId(path.as_str().to_string()),
            dev_node: String::new(),
            block_path: None,
            model: prop_string(drive_props, "Model").unwrap_or_default(),
            vendor: prop_string(drive_props, "Vendor").unwrap_or_default(),
            size: prop_u64(drive_props, "Size").unwrap_or(0),
            removable: prop_bool(drive_props, "Removable").unwrap_or(false),
            ejectable: prop_bool(drive_props, "Ejectable").unwrap_or(false),
            can_power_off: prop_bool(drive_props, "CanPowerOff").unwrap_or(false),
            bus: BusType::parse(&prop_string(drive_props, "ConnectionBus").unwrap_or_default()),
            rotational: rotation_rate > 0,
            is_system: false,
            is_ventoy: false,
            partitions: Vec::new(),
        };
        drive_path_index.insert(path.as_str().to_string(), drives.len());
        drives.push(drive);
    }

    for (path, ifaces) in objects.iter() {
        let Some(block_props) = ifaces.get("org.freedesktop.UDisks2.Block") else {
            continue;
        };
        let Some(drive_path) = prop_object_path(block_props, "Drive") else {
            continue;
        };
        if drive_path == "/" {
            continue;
        }
        let Some(&drive_idx) = drive_path_index.get(&drive_path) else {
            continue;
        };

        let dev_node = prop_bytes_path(block_props, "Device").unwrap_or_default();
        let has_partition_table_entry = ifaces.contains_key("org.freedesktop.UDisks2.Partition");
        let is_whole_disk_block = !has_partition_table_entry;

        let id_type = prop_string(block_props, "IdType").unwrap_or_default();
        if is_whole_disk_block {
            drives[drive_idx].dev_node = dev_node.clone();
            drives[drive_idx].block_path = Some(path.as_str().to_string());
            if id_type.trim().is_empty() {
                continue;
            }
        }

        let mount_points = ifaces
            .get("org.freedesktop.UDisks2.Filesystem")
            .map(|fs_props| prop_mount_points(fs_props, "MountPoints"))
            .unwrap_or_default();

        let size = prop_u64(block_props, "Size").unwrap_or(0);
        let used = mount_points
            .first()
            .and_then(|mp| disk_usage_for_mount(disks, mp));
        let hint_system = prop_bool(block_props, "HintSystem").unwrap_or(false);
        let is_swap = active_swaps.iter().any(|s| s == &dev_node);

        let partition = PartitionInfo {
            id: DeviceId(path.as_str().to_string()),
            dev_node,
            label: prop_string(block_props, "IdLabel").unwrap_or_default(),
            fs: FsKind::parse(&id_type),
            size,
            used,
            mount_points,
            is_swap,
            is_system: hint_system,
        };
        drives[drive_idx].partitions.push(partition);
    }

    for drive in drives.iter_mut() {
        drive.partitions.sort_by(|a, b| a.dev_node.cmp(&b.dev_node));
        let mut drive_is_system = false;
        for idx in 0..drive.partitions.len() {
            let system =
                drive.partitions[idx].is_system || is_system_disk(drive, &drive.partitions[idx]);
            drive.partitions[idx].is_system = system;
            drive_is_system |= system;
        }

        drive_is_system |= !drive.removable && drive.bus != BusType::Usb;
        drive.is_system = drive_is_system;

        if drive_is_system {
            for part in &mut drive.partitions {
                part.is_system = true;
            }
        }

        drive.is_ventoy = detect_ventoy(&drive.partitions);
    }
    drives.sort_by(|a, b| a.dev_node.cmp(&b.dev_node));

    StorageSnapshot {
        udisks_available: true,
        drives,
    }
}

fn disk_usage_for_mount(disks: &Disks, mount_point: &str) -> Option<u64> {
    disks
        .list()
        .iter()
        .find(|d| d.mount_point().to_string_lossy() == mount_point)
        .map(|d| d.total_space().saturating_sub(d.available_space()))
}

fn free_bytes_for_mount(mount_point: &str) -> Option<u64> {
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .find(|d| d.mount_point().to_string_lossy() == mount_point)
        .map(|d| d.available_space())
}

async fn get_managed_objects(conn: &Connection) -> anyhow::Result<ManagedObjects> {
    let reply = conn
        .call_method(
            Some(UDISKS_SERVICE),
            UDISKS_ROOT,
            Some("org.freedesktop.DBus.ObjectManager"),
            "GetManagedObjects",
            &(),
        )
        .await?;
    Ok(reply.body().deserialize::<ManagedObjects>()?)
}

async fn udisks_call(
    conn: &Connection,
    path: &str,
    iface: &str,
    method: &str,
) -> anyhow::Result<()> {
    let opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    conn.call_method(Some(UDISKS_SERVICE), path, Some(iface), method, &(opts,))
        .await?;
    Ok(())
}

async fn mount_and_get_path(conn: &Connection, path: &str) -> anyhow::Result<String> {
    let opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    let reply = conn
        .call_method(
            Some(UDISKS_SERVICE),
            path,
            Some("org.freedesktop.UDisks2.Filesystem"),
            "Mount",
            &(opts,),
        )
        .await?;
    Ok(reply.body().deserialize::<String>()?)
}

async fn ensure_mounted(
    conn: &Connection,
    part_path: &str,
    existing_mount: Option<String>,
) -> anyhow::Result<String> {
    if let Some(mp) = existing_mount {
        if !mp.is_empty() {
            return Ok(mp);
        }
    }
    mount_and_get_path(conn, part_path).await
}

async fn unmount(conn: &Connection, path: &str) -> anyhow::Result<()> {
    udisks_call(conn, path, "org.freedesktop.UDisks2.Filesystem", "Unmount").await
}

pub fn is_no_usb_device_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_ascii_lowercase();
    lower.contains("no usb device")
}

pub fn skips_power_off(drive: &DriveInfo) -> bool {
    drive.bus == BusType::Mmc || !drive.can_power_off
}

async fn eject(conn: &Connection, drive: &DriveInfo) -> anyhow::Result<()> {
    let path = drive.id.0.as_str();

    if skips_power_off(drive) {
        if drive.ejectable {
            let _ = udisks_call(conn, path, "org.freedesktop.UDisks2.Drive", "Eject").await;
        }
        unsafe {
            libc::sync();
        }
        return Ok(());
    }

    if drive.ejectable {
        let _ = udisks_call(conn, path, "org.freedesktop.UDisks2.Drive", "Eject").await;
    }
    match udisks_call(conn, path, "org.freedesktop.UDisks2.Drive", "PowerOff").await {
        Ok(()) => Ok(()),
        Err(e) if is_no_usb_device_error(&e) => {
            unsafe {
                libc::sync();
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

async fn format_block(
    conn: &Connection,
    path: &str,
    fs_type: &str,
    label: &str,
) -> anyhow::Result<()> {
    let mut opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    opts.insert("label", zbus::zvariant::Value::from(label));
    opts.insert("update-partition-type", zbus::zvariant::Value::from(true));

    opts.insert("tear-down", zbus::zvariant::Value::from(true));
    conn.call_method(
        Some(UDISKS_SERVICE),
        path,
        Some("org.freedesktop.UDisks2.Block"),
        "Format",
        &(fs_type, opts),
    )
    .await?;
    Ok(())
}

async fn open_device_fd(conn: &Connection, block_path: &str) -> anyhow::Result<std::fs::File> {
    let mut opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    opts.insert(
        "flags",
        zbus::zvariant::Value::from(libc::O_SYNC | libc::O_EXCL),
    );
    let reply = conn
        .call_method(
            Some(UDISKS_SERVICE),
            block_path,
            Some("org.freedesktop.UDisks2.Block"),
            "OpenDevice",
            &("rw", opts),
        )
        .await?;
    let owned_fd: zbus::zvariant::OwnedFd = reply.body().deserialize()?;
    let std_fd: std::os::fd::OwnedFd = owned_fd.into();
    Ok(std::fs::File::from(std_fd))
}

async fn checksum_task(iso_path: String, tx: EventTx) {
    let path_buf = std::path::PathBuf::from(&iso_path);
    let file = match tokio::fs::File::open(&path_buf).await {
        Ok(f) => f,
        Err(e) => {
            let _ = tx.send(AppEvent::Toast(Toast::error(format!(
                "Falha ao abrir ISO para checksum: {e}"
            ))));
            return;
        }
    };
    let total = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    let mut reader = tokio::io::BufReader::with_capacity(IO_BUFFER_SIZE, file);
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; IO_BUFFER_SIZE];
    let mut read_total: u64 = 0;
    let mut last_emit = tokio::time::Instant::now();

    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = tx.send(AppEvent::Toast(Toast::error(format!(
                    "Falha ao ler ISO: {e}"
                ))));
                return;
            }
        };
        hasher.update(&buf[..n]);
        read_total += n as u64;
        if total > 0 && last_emit.elapsed() >= PROGRESS_THROTTLE {
            let pct = (read_total as f32 / total as f32).clamp(0.0, 1.0);
            let _ = tx.send(AppEvent::StorageChecksumProgress {
                path: path_buf.clone(),
                pct,
            });
            last_emit = tokio::time::Instant::now();
        }
    }

    let sha256 = format!("{:x}", hasher.finalize());
    let _ = tx.send(AppEvent::StorageChecksumDone {
        path: path_buf,
        sha256,
    });
}

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

pub fn is_gzip_file(path: &std::path::Path) -> std::io::Result<bool> {
    let mut f = std::fs::File::open(path)?;
    let mut magic = [0u8; 2];
    match f.read_exact(&mut magic) {
        Ok(()) => Ok(magic == GZIP_MAGIC),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn gzip_uncompressed_size_hint(path: &std::path::Path) -> Option<u64> {
    use std::io::{Seek, SeekFrom};
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    if len < 4 {
        return None;
    }
    f.seek(SeekFrom::End(-4)).ok()?;
    let mut buf = [0u8; 4];
    f.read_exact(&mut buf).ok()?;
    Some(u32::from_le_bytes(buf) as u64)
}

fn probe_image_sync(path: &std::path::Path) -> (bool, u64) {
    let compressed = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let is_gzip = is_gzip_file(path).unwrap_or(false);
    let total = if is_gzip {
        gzip_uncompressed_size_hint(path).unwrap_or(compressed)
    } else {
        compressed
    };
    (is_gzip, total)
}

async fn probe_image(iso_path: &str) -> (bool, u64) {
    let path = std::path::PathBuf::from(iso_path);
    tokio::task::spawn_blocking(move || probe_image_sync(&path))
        .await
        .unwrap_or((false, 0))
}

fn flash_sync(
    iso_path: &std::path::Path,
    dev_node: &str,
    cancel: &Arc<AtomicBool>,
    tx: &EventTx,
) -> anyhow::Result<()> {
    let (is_gzip, total_bytes) = probe_image_sync(iso_path);
    let raw_file = std::fs::File::open(iso_path)?;
    let mut reader: Box<dyn Read> = if is_gzip {
        Box::new(std::io::BufReader::with_capacity(IO_BUFFER_SIZE, GzDecoder::new(raw_file)))
    } else {
        Box::new(raw_file)
    };
    let mut dst = std::fs::OpenOptions::new().write(true).open(dev_node)?;

    let mut buf = vec![0u8; IO_BUFFER_SIZE];
    let mut written: u64 = 0;
    let mut window_started = std::time::Instant::now();
    let mut window_bytes: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("gravação cancelada pelo usuário");
        }
        let mut n = 0;
        while n < IO_BUFFER_SIZE {
            match reader.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
        written += n as u64;
        window_bytes += n as u64;

        if window_started.elapsed() >= PROGRESS_THROTTLE {
            let (speed_mbps, eta_secs) = compute_speed_eta(
                window_bytes,
                window_started.elapsed().as_secs_f64(),
                written,
                total_bytes,
            );
            let _ = tx.send(AppEvent::StorageFlashProgress {
                bytes_written: written,
                total_bytes,
                speed_mbps,
                eta_secs,
            });
            window_started = std::time::Instant::now();
            window_bytes = 0;
        }
    }

    dst.flush()?;
    dst.sync_all()?;

    unsafe {
        libc::sync();
    }

    let _ = tx.send(AppEvent::StorageFlashProgress {
        bytes_written: written,
        total_bytes: written,
        speed_mbps: 0.0,
        eta_secs: 0,
    });
    Ok(())
}

async fn flash_inner(
    iso_path: &str,
    dev_node: &str,
    cancel: &Arc<AtomicBool>,
    tx: &EventTx,
) -> anyhow::Result<()> {
    let path_buf = std::path::PathBuf::from(iso_path);
    let dev_owned = dev_node.to_string();
    let cancel = Arc::clone(cancel);
    let tx = tx.clone();
    tokio::task::spawn_blocking(move || flash_sync(&path_buf, &dev_owned, &cancel, &tx)).await?
}

async fn multiboot_prepare_task(device_id: String, mount_point: String, tx: EventTx) {
    let mp = std::path::PathBuf::from(&mount_point);
    let result = tokio::task::spawn_blocking(move || crate::backend::multiboot::prepare_multiboot(&mp))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r.map_err(|e| e.to_string()));
    let toast = match &result {
        Ok(()) => {
            let n = crate::backend::multiboot::count_isos(&mount_point);
            Toast::info(format!("Multi-boot preparado com sucesso ({n} ISO(s))"))
        }
        Err(e) => Toast::error(format!("Falha ao preparar multi-boot: {e}")),
    };
    tracing::warn!(target: "hal9001::storage", device = %device_id, mount = %mount_point, ok = result.is_ok(), "preparação de multi-boot concluída");
    let _ = tx.send(AppEvent::Toast(toast));
}

async fn format_via_sudo(
    dev_node: &str,
    bin: &str,
    args: &[String],
    sudo_tx: &SudoPasswordTx,
    tx: &EventTx,
) -> Result<(), String> {
    let label = format!("Formatar {dev_node} ({bin})");
    run_sudo_command(&label, bin, args, sudo_tx, tx).await
}

async fn run_sudo_command(
    label: &str,
    program: &str,
    args: &[String],
    sudo_tx: &SudoPasswordTx,
    tx: &EventTx,
) -> Result<(), String> {
    let mut retry_error: Option<String> = None;
    loop {
        let password = match next_sudo_attempt(sudo_tx, label, &mut retry_error).await {
            Ok(pw) => pw,
            Err(msg) => return Err(msg),
        };
        let mut run = spawn_sudo(password, program.to_string(), args.to_vec())
            .map_err(|e| e.to_string())?;
        while let Some(line) = run.lines.recv().await {
            let _ = tx.send(AppEvent::Toast(Toast::info(line)));
        }
        let (status, stderr_text) = run
            .handle
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?;
        if status.success() {
            return Ok(());
        }
        if is_sudo_auth_failure(&stderr_text) {
            retry_error = Some("Senha incorreta".to_string());
            continue;
        }
        return Err(format!("{status}: {stderr_text}"));
    }
}

fn mkfs_vfat_available() -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join("mkfs.vfat").is_file()))
        .unwrap_or(false)
}

async fn format_fat32_elevated(
    dev_node: &str,
    label: &str,
    sudo_tx: &SudoPasswordTx,
    tx: &EventTx,
) -> Result<(), String> {
    let original_mode = std::fs::metadata(dev_node)
        .map(|m| m.permissions().mode() & 0o777)
        .map_err(|e| e.to_string())?;
    let sudo_label = format!("Formatar {dev_node} (FAT32, sem mkfs.vfat)");
    run_sudo_command(
        &sudo_label,
        "chmod",
        &["666".to_string(), dev_node.to_string()],
        sudo_tx,
        tx,
    )
    .await?;

    let dev_owned = dev_node.to_string();
    let label_owned = label.to_string();
    let format_result =
        tokio::task::spawn_blocking(move || format_fat32_pure_rust(&dev_owned, &label_owned))
            .await
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()));

    let restore_args = vec![format!("{original_mode:o}"), dev_node.to_string()];
    let _ = run_sudo_command(&sudo_label, "chmod", &restore_args, sudo_tx, tx).await;

    format_result
}

async fn try_pure_rust_fat32(conn: &Connection, block_path: &str, label: &str) -> Result<(), String> {
    let file = open_device_fd(conn, block_path)
        .await
        .map_err(|e| e.to_string())?;
    let label_owned = label.to_string();
    tokio::task::spawn_blocking(move || format_fat32_on_file(file, &label_owned))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn format_with_sudo_fallback(
    conn: &Connection,
    snap: &StorageSnapshot,
    block_path: &str,
    device_id: &str,
    fs_type: &str,
    label: &str,
    original_err: &anyhow::Error,
    sudo_tx: &SudoPasswordTx,
    tx: &EventTx,
) -> Toast {
    let Some(dev_node) = snap.dev_node_for_block_path(block_path) else {
        return Toast::error(format_error_message(fs_type, original_err));
    };
    if is_fat_fs_type(fs_type) && !mkfs_vfat_available() {

        tracing::warn!(target: "hal9001::storage", device = %device_id, "mkfs.vfat ausente no host — formatando FAT32 via fatfs com permissão elevada (sudo chmod)");
        return match format_fat32_elevated(&dev_node, label, sudo_tx, tx).await {
            Ok(()) => {
                let _ = udisks_call(conn, block_path, "org.freedesktop.UDisks2.Block", "Rescan")
                    .await;
                Toast::info("Formatação concluída (FAT32 Rust puro, sudo)")
            }
            Err(msg) => Toast::error(format!("Falha ao formatar FAT32: {msg}")),
        };
    }
    let Some((bin, args)) = mkfs_command(fs_type, label, &dev_node) else {
        return Toast::error(format_error_message(fs_type, original_err));
    };
    tracing::warn!(target: "hal9001::storage", device = %device_id, "Block.Format/OpenDevice recusado — usando fallback de mkfs via sudo");
    match format_via_sudo(&dev_node, &bin, &args, sudo_tx, tx).await {
        Ok(()) => {
            let _ = udisks_call(conn, block_path, "org.freedesktop.UDisks2.Block", "Rescan").await;
            Toast::info(format!("Formatação concluída ({bin}, sudo)"))
        }
        Err(msg) => Toast::error(format!("Falha ao formatar via {bin}: {msg}")),
    }
}

async fn flash_elevated(
    iso_path: &str,
    dev_node: &str,
    total_bytes: u64,
    sudo_tx: &SudoPasswordTx,
    tx: &EventTx,
) -> anyhow::Result<()> {
    let label = format!("Gravar ISO em {dev_node}");
    let args = vec![
        format!("if={iso_path}"),
        format!("of={dev_node}"),
        "bs=4M".to_string(),
        "status=progress".to_string(),
        "conv=fsync".to_string(),
    ];
    let mut retry_error: Option<String> = None;

    loop {
        let password = match next_sudo_attempt(sudo_tx, &label, &mut retry_error).await {
            Ok(pw) => pw,
            Err(msg) => anyhow::bail!(msg),
        };
        let mut run = spawn_sudo(password, "dd".to_string(), args.clone())?;

        let mut last_bytes = 0u64;
        let mut window_started = tokio::time::Instant::now();
        let mut last_emit = tokio::time::Instant::now();
        while let Some(line) = run.lines.recv().await {
            let Some(bytes) = parse_dd_bytes_copied(&line) else {
                continue;
            };
            if last_emit.elapsed() >= PROGRESS_THROTTLE {
                let (speed_mbps, eta_secs) = compute_speed_eta(
                    bytes.saturating_sub(last_bytes),
                    window_started.elapsed().as_secs_f64(),
                    bytes,
                    total_bytes,
                );
                let _ = tx.send(AppEvent::StorageFlashProgress {
                    bytes_written: bytes,
                    total_bytes,
                    speed_mbps,
                    eta_secs,
                });
                last_bytes = bytes;
                window_started = tokio::time::Instant::now();
                last_emit = tokio::time::Instant::now();
            }
        }
        let (status, stderr_text) = run.handle.await??;
        if status.success() {
            break;
        }
        if is_sudo_auth_failure(&stderr_text) {
            retry_error = Some("Senha incorreta".to_string());
            continue;
        }
        anyhow::bail!("dd elevado terminou com {status}: {stderr_text}");
    }

    let _ = tx.send(AppEvent::StorageFlashProgress {
        bytes_written: total_bytes,
        total_bytes,
        speed_mbps: 0.0,
        eta_secs: 0,
    });

    unsafe {
        libc::sync();
    }
    Ok(())
}

fn flash_elevated_gzip_sync(
    iso_path: &str,
    dev_node: &str,
    password: Option<String>,
    cancel: &Arc<AtomicBool>,
    tx: &EventTx,
    total_bytes: u64,
) -> anyhow::Result<(std::process::ExitStatus, String)> {
    let mut cmd = std::process::Command::new("sudo");
    if password.is_some() {
        cmd.arg("-S").arg("-k").arg("--");
    } else {
        cmd.arg("-n").arg("--");
    }
    cmd.arg("dd")
        .arg(format!("of={dev_node}"))
        .arg("bs=4M")
        .arg("status=progress")
        .arg("conv=fsync");
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn()?;

    let mut stdin = child.stdin.take().expect("stdin piped");
    if let Some(pw) = &password {
        stdin.write_all(pw.as_bytes())?;
        stdin.write_all(b"\n")?;
    }
    drop(child.stdout.take());

    let mut stderr = child.stderr.take().expect("stderr piped");
    let tx_err = tx.clone();
    let stderr_thread = std::thread::spawn(move || -> String {
        let mut buf = [0u8; 4096];
        let mut partial: Vec<u8> = Vec::new();
        let mut full = String::new();
        loop {
            let n = match stderr.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            for &b in &buf[..n] {
                if b == b'\n' || b == b'\r' {
                    if !partial.is_empty() {
                        let line = String::from_utf8_lossy(&partial).to_string();
                        full.push_str(&line);
                        full.push('\n');
                        let _ = tx_err.send(AppEvent::Toast(Toast::info(line)));
                        partial.clear();
                    }
                } else {
                    partial.push(b);
                }
            }
        }
        full
    });

    let write_result: anyhow::Result<()> = (|| {
        let raw_file = std::fs::File::open(iso_path)?;
        let mut decoder = std::io::BufReader::with_capacity(IO_BUFFER_SIZE, GzDecoder::new(raw_file));
        let mut buf = vec![0u8; IO_BUFFER_SIZE];
        let mut written: u64 = 0;
        let mut window_started = std::time::Instant::now();
        let mut window_bytes: u64 = 0;
        loop {
            if cancel.load(Ordering::Relaxed) {
                anyhow::bail!("gravação cancelada pelo usuário");
            }
            let mut n = 0;
            while n < IO_BUFFER_SIZE {
                match decoder.read(&mut buf[n..]) {
                    Ok(0) => break,
                    Ok(k) => n += k,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e.into()),
                }
            }
            if n == 0 {
                break;
            }
            stdin.write_all(&buf[..n])?;
            written += n as u64;
            window_bytes += n as u64;

            if window_started.elapsed() >= PROGRESS_THROTTLE {
                let (speed_mbps, eta_secs) = compute_speed_eta(
                    window_bytes,
                    window_started.elapsed().as_secs_f64(),
                    written,
                    total_bytes,
                );
                let _ = tx.send(AppEvent::StorageFlashProgress {
                    bytes_written: written,
                    total_bytes,
                    speed_mbps,
                    eta_secs,
                });
                window_started = std::time::Instant::now();
                window_bytes = 0;
            }
        }
        Ok(())
    })();

    drop(stdin);

    if let Err(e) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        let _ = stderr_thread.join();
        return Err(e);
    }

    let status = child.wait()?;
    let stderr_text = stderr_thread.join().unwrap_or_default();
    Ok((status, stderr_text))
}

async fn flash_elevated_gzip(
    iso_path: &str,
    dev_node: &str,
    total_bytes: u64,
    cancel: &Arc<AtomicBool>,
    sudo_tx: &SudoPasswordTx,
    tx: &EventTx,
) -> anyhow::Result<()> {
    let label = format!("Gravar ISO comprimida em {dev_node}");
    let mut retry_error: Option<String> = None;

    loop {
        let password = match next_sudo_attempt(sudo_tx, &label, &mut retry_error).await {
            Ok(pw) => pw,
            Err(msg) => anyhow::bail!(msg),
        };
        let iso_owned = iso_path.to_string();
        let dev_owned = dev_node.to_string();
        let cancel_owned = Arc::clone(cancel);
        let tx_owned = tx.clone();
        let (status, stderr_text) = tokio::task::spawn_blocking(move || {
            flash_elevated_gzip_sync(&iso_owned, &dev_owned, password, &cancel_owned, &tx_owned, total_bytes)
        })
        .await??;
        if status.success() {
            break;
        }
        if is_sudo_auth_failure(&stderr_text) {
            retry_error = Some("Senha incorreta".to_string());
            continue;
        }
        anyhow::bail!("dd elevado (streaming) terminou com {status}: {stderr_text}");
    }

    let _ = tx.send(AppEvent::StorageFlashProgress {
        bytes_written: total_bytes,
        total_bytes,
        speed_mbps: 0.0,
        eta_secs: 0,
    });

    unsafe {
        libc::sync();
    }
    Ok(())
}

async fn flash_task(
    device_id: String,
    iso_path: String,
    dev_node: String,
    cancel: Arc<AtomicBool>,
    tx: EventTx,
    sudo_tx: SudoPasswordTx,
) {
    let (is_gzip, total_bytes) = probe_image(&iso_path).await;
    let result = match flash_inner(&iso_path, &dev_node, &cancel, &tx).await {
        Ok(()) => Ok("gravação concluída com sucesso".to_string()),
        Err(e) if is_permission_denied_error(&e) => {
            tracing::warn!(target: "hal9001::storage", device = %device_id, "permissão negada ao abrir dispositivo de bloco — usando fallback de dd elevado");
            let _ = tx.send(AppEvent::Toast(Toast::info(
                "permissão negada — solicitando elevação (sudo) para gravar o dispositivo",
            )));
            if is_gzip {
                flash_elevated_gzip(&iso_path, &dev_node, total_bytes, &cancel, &sudo_tx, &tx)
                    .await
                    .map(|()| "gravação concluída com sucesso (elevado, streaming)".to_string())
                    .map_err(|e| e.to_string())
            } else {
                flash_elevated(&iso_path, &dev_node, total_bytes, &sudo_tx, &tx)
                    .await
                    .map(|()| "gravação concluída com sucesso (elevado)".to_string())
                    .map_err(|e| e.to_string())
            }
        }
        Err(e) => Err(e.to_string()),
    };
    let _ = tx.send(AppEvent::StorageFlashDone { device_id, result });
}

fn isos_subdir(mount_point: &str) -> String {
    format!("{}/ISOs", mount_point.trim_end_matches('/'))
}

async fn list_and_emit(mount_point: &str, device_id: String, tx: &EventTx) {
    let isos_dir = isos_subdir(mount_point);
    let mut raw = Vec::new();
    if let Ok(mut rd) = tokio::fs::read_dir(&isos_dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            raw.push((name, meta.len(), meta.modified().ok()));
        }
    }
    let entries = build_ventoy_entries(raw);
    let free_bytes = free_bytes_for_mount(mount_point);
    let _ = tx.send(AppEvent::StorageMultibootIsoList {
        device_id,
        entries,
        free_bytes,
    });
}

async fn multiboot_list_isos_task(
    conn: Connection,
    device_id: String,
    part_path: String,
    existing_mount: Option<String>,
    tx: EventTx,
) {
    match ensure_mounted(&conn, &part_path, existing_mount).await {
        Ok(mount_point) => list_and_emit(&mount_point, device_id, &tx).await,
        Err(e) => {
            let _ = tx.send(AppEvent::Toast(Toast::error(format!(
                "Falha ao montar partição de dados: {e}"
            ))));
            let _ = tx.send(AppEvent::StorageMultibootIsoList {
                device_id,
                entries: Vec::new(),
                free_bytes: None,
            });
        }
    }
}

async fn copy_iso_inner(
    src_path: &str,
    dst_path: &str,
    device_id: &str,
    tx: &EventTx,
) -> anyhow::Result<()> {
    let mut src = tokio::fs::File::open(src_path).await?;
    let total_bytes = src.metadata().await?.len();
    let mut dst = tokio::fs::File::create(dst_path).await?;

    let mut buf = vec![0u8; IO_BUFFER_SIZE];
    let mut written: u64 = 0;
    let mut last_emit = tokio::time::Instant::now();

    loop {
        let n = src.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await?;
        written += n as u64;

        if last_emit.elapsed() >= PROGRESS_THROTTLE {
            let _ = tx.send(AppEvent::StorageMultibootIsoCopyProgress {
                device_id: device_id.to_string(),
                bytes_written: written,
                total_bytes,
            });
            last_emit = tokio::time::Instant::now();
        }
    }

    dst.flush().await?;
    dst.sync_all().await?;

    let _ = tx.send(AppEvent::StorageMultibootIsoCopyProgress {
        device_id: device_id.to_string(),
        bytes_written: total_bytes,
        total_bytes,
    });
    Ok(())
}

async fn multiboot_add_iso_task(
    conn: Connection,
    device_id: String,
    part_path: String,
    existing_mount: Option<String>,
    src_path: String,
    tx: EventTx,
) {
    let mount_point = match ensure_mounted(&conn, &part_path, existing_mount).await {
        Ok(mp) => mp,
        Err(e) => {
            let _ = tx.send(AppEvent::StorageMultibootIsoCopyDone {
                device_id,
                result: Err(format!("falha ao montar partição de dados: {e}")),
            });
            return;
        }
    };
    let isos_dir = isos_subdir(&mount_point);
    if let Err(e) = tokio::fs::create_dir_all(&isos_dir).await {
        let _ = tx.send(AppEvent::StorageMultibootIsoCopyDone {
            device_id,
            result: Err(format!("falha ao criar diretório ISOs/: {e}")),
        });
        return;
    }
    let file_name = std::path::Path::new(&src_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.iso".to_string());
    let dst_path = format!("{isos_dir}/{file_name}");

    let result = copy_iso_inner(&src_path, &dst_path, &device_id, &tx)
        .await
        .map(|()| file_name.clone())
        .map_err(|e| e.to_string());
    let _ = tx.send(AppEvent::StorageMultibootIsoCopyDone {
        device_id: device_id.clone(),
        result,
    });
    list_and_emit(&mount_point, device_id, &tx).await;
}

async fn multiboot_remove_iso_task(
    conn: Connection,
    device_id: String,
    part_path: String,
    existing_mount: Option<String>,
    file_name: String,
    tx: EventTx,
) {
    let mount_point = match ensure_mounted(&conn, &part_path, existing_mount).await {
        Ok(mp) => mp,
        Err(e) => {
            let _ = tx.send(AppEvent::StorageMultibootIsoRemoveDone {
                device_id,
                result: Err(format!("falha ao montar partição de dados: {e}")),
            });
            return;
        }
    };
    let target = format!("{}/{}", isos_subdir(&mount_point), file_name);
    let result = tokio::fs::remove_file(&target)
        .await
        .map(|()| file_name.clone())
        .map_err(|e| e.to_string());
    let _ = tx.send(AppEvent::StorageMultibootIsoRemoveDone {
        device_id: device_id.clone(),
        result,
    });
    list_and_emit(&mount_point, device_id, &tx).await;
}

async fn refresh_snapshot(conn: &Option<Connection>) -> anyhow::Result<StorageSnapshot> {
    let Some(c) = conn else {
        return Err(anyhow::anyhow!("sem conexão D-Bus"));
    };
    let objects = get_managed_objects(c).await?;
    let swaps = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    let disks = Disks::new_with_refreshed_list();
    Ok(build_snapshot(&objects, &swaps, &disks))
}

pub async fn run(
    poll_ms: u64,
    tx: EventTx,
    mut actions: broadcast::Receiver<Action>,
    sudo_tx: SudoPasswordTx,
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms.max(1000)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut conn: Option<Connection> = Connection::system().await.ok();
    let mut last_snapshot: Option<StorageSnapshot> = None;

    let mut flash_cancels: HashMap<String, Arc<AtomicBool>> = HashMap::new();

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if conn.is_none() {
                    conn = Connection::system().await.ok();
                }
                match refresh_snapshot(&conn).await {
                    Ok(snap) => {
                        last_snapshot = Some(snap.clone());
                        if tx.send(AppEvent::Storage(Box::new(snap))).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        conn = None;
                        let _ = tx.send(AppEvent::ServiceDegraded {
                            name: "storage",
                            reason: format!("UDisks2 indisponível: {e}"),
                        });
                    }
                }
            }
            res = actions.recv() => match res {
                Ok(Action::StorageAnalyzerScan(path)) => {

                    tokio::spawn(crate::backend::disk_analyzer::scan(path, tx.clone()));
                }
                Ok(action) => {
                    if let Some(c) = &conn {
                        handle_action(c, action, &last_snapshot, &tx, &mut flash_cancels, &sudo_tx).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    Ok(())
}

async fn handle_action(
    conn: &Connection,
    action: Action,
    snapshot: &Option<StorageSnapshot>,
    tx: &EventTx,
    flash_cancels: &mut HashMap<String, Arc<AtomicBool>>,
    sudo_tx: &SudoPasswordTx,
) {
    match action {
        Action::StorageMount(id) => {

            let toast = if let Some(drive) = snapshot.as_ref().and_then(|s| s.drive_by_id(&id)) {
                let mut mount_points: Vec<String> = Vec::new();
                let mut last_err: Option<anyhow::Error> = None;
                for part in &drive.partitions {
                    if part.is_mounted() {
                        continue;
                    }
                    match mount_and_get_path(conn, &part.id.0).await {
                        Ok(mp) => mount_points.push(mp),
                        Err(e) => last_err = Some(e),
                    }
                }
                match (mount_points.as_slice(), last_err) {
                    ([], Some(e)) => Toast::error(format!("Falha ao montar: {e}")),
                    ([], None) => Toast::error(
                        "Dispositivo sem partição montável — formate com [f] primeiro",
                    ),
                    ([mp], _) => Toast::info(format!("Partição montada em {mp}")),
                    (mps, _) => Toast::info(format!(
                        "{} partição(ões) montada(s) em: {}",
                        mps.len(),
                        mps.join(", ")
                    )),
                }
            } else {
                match mount_and_get_path(conn, &id.0).await {
                    Ok(mp) => Toast::info(format!("Partição montada em {mp}")),
                    Err(e) => Toast::error(format!("Falha ao montar: {e}")),
                }
            };
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageUnmount(id) => {

            let toast = if let Some(drive) = snapshot.as_ref().and_then(|s| s.drive_by_id(&id)) {
                let mut unmounted = 0usize;
                let mut last_err: Option<anyhow::Error> = None;
                for part in &drive.partitions {
                    if !part.is_mounted() {
                        continue;
                    }
                    match unmount(conn, &part.id.0).await {
                        Ok(()) => unmounted += 1,
                        Err(e) => last_err = Some(e),
                    }
                }
                match (unmounted, last_err) {
                    (0, Some(e)) => {
                        Toast::error(format!("Falha ao desmontar (dispositivo em uso?): {e}"))
                    }
                    (0, None) => Toast::info("nenhuma partição montada"),
                    (n, _) => Toast::info(format!("{n} partição(ões) desmontada(s)")),
                }
            } else {
                match unmount(conn, &id.0).await {
                    Ok(()) => Toast::info("Dispositivo desmontado"),
                    Err(e) => {
                        Toast::error(format!("Falha ao desmontar (dispositivo em uso?): {e}"))
                    }
                }
            };
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageEject(id) => {

            let Some(drive) = snapshot
                .as_ref()
                .and_then(|snap| snap.drives.iter().find(|d| d.id == id))
                .cloned()
            else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "dispositivo não encontrado para ejeção",
                )));
                return;
            };
            if drive.is_system {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                tracing::warn!(target: "hal9001::storage", drive = %id.0, "ejeção de disco de sistema recusada");
                return;
            }

            let mut unmount_err: Option<anyhow::Error> = None;
            for part in &drive.partitions {
                if part.is_mounted() {
                    if let Err(e) = unmount(conn, &part.id.0).await {
                        unmount_err = Some(e);
                    }
                }
            }
            if let Some(e) = unmount_err {
                let _ = tx.send(AppEvent::Toast(Toast::error(format!(
                    "Falha ao desmontar (dispositivo em uso?): {e}"
                ))));
                return;
            }

            let toast = match eject(conn, &drive).await {
                Ok(()) => Toast::success("[DISCO] Ejeção segura concluída".to_string()),
                Err(e) => Toast::error(format!("Falha ao ejetar: {e}")),
            };
            tracing::warn!(target: "hal9001::storage", drive = %id.0, "ejeção de dispositivo executada");
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageRefresh => {

        }
        Action::StorageFormat {
            device_id,
            fs_type,
            label,
        } => {

            let id = DeviceId(device_id.clone());
            if snapshot
                .as_ref()
                .map(|s| s.is_system_target(&id))
                .unwrap_or(true)
            {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "formatação de disco de sistema recusada");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                return;
            }

            let Some(snap) = snapshot.as_ref() else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };

            let Some(block_path) = resolve_block_object_path(snap, &id) else {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "bloco de dispositivo não encontrado para formatação");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "dispositivo de bloco não encontrado para formatação",
                )));
                return;
            };

            for part_path in mounted_partition_paths(snap, &id) {
                let _ = unmount(conn, &part_path).await;
            }
            tracing::warn!(target: "hal9001::storage", device = %device_id, block = %block_path, fs = %fs_type, label = %label, "formatação solicitada");
            let toast = match format_block(conn, &block_path, &fs_type, &label).await {
                Ok(()) => Toast::info("Formatação concluída"),
                Err(e) if is_fat_fs_type(&fs_type) && is_missing_mkfs_error(&e) => {

                    tracing::warn!(target: "hal9001::storage", device = %device_id, "mkfs.vfat ausente no host — tentando formatador FAT32 Rust puro via Block.OpenDevice");
                    match try_pure_rust_fat32(conn, &block_path, &label).await {
                        Ok(()) => {

                            let _ = udisks_call(
                                conn,
                                &block_path,
                                "org.freedesktop.UDisks2.Block",
                                "Rescan",
                            )
                            .await;
                            Toast::info("Formatação concluída (FAT32 Rust puro)")
                        }
                        Err(_) => {
                            format_with_sudo_fallback(
                                conn, snap, &block_path, &device_id, &fs_type, &label, &e,
                                sudo_tx, tx,
                            )
                            .await
                        }
                    }
                }
                Err(e) => {

                    format_with_sudo_fallback(
                        conn, snap, &block_path, &device_id, &fs_type, &label, &e, sudo_tx, tx,
                    )
                    .await
                }
            };
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageChecksumIso(iso_path) => {
            let txc = tx.clone();
            tokio::spawn(checksum_task(iso_path, txc));
        }
        Action::StorageFlashIso {
            device_id,
            iso_path,
        } => {
            let id = DeviceId(device_id.clone());
            let Some(snap) = snapshot else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            if snap.is_system_target(&id) {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "gravação de ISO em disco de sistema recusada");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                return;
            }
            let Some(dev_node) = snap.drive_dev_node(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "dispositivo alvo não encontrado",
                )));
                return;
            };
            for part_path in mounted_partition_paths(snap, &id) {
                let _ = unmount(conn, &part_path).await;
            }
            tracing::warn!(target: "hal9001::storage", device = %device_id, dev_node = %dev_node, iso = %iso_path, "gravação de ISO solicitada");
            let cancel = Arc::new(AtomicBool::new(false));
            flash_cancels.insert(device_id.clone(), cancel.clone());
            let txc = tx.clone();
            let sudo_txc = sudo_tx.clone();
            tokio::spawn(flash_task(
                device_id, iso_path, dev_node, cancel, txc, sudo_txc,
            ));
        }
        Action::StorageFlashCancel { device_id } => {
            if let Some(cancel) = flash_cancels.get(&device_id) {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        Action::StorageMultibootPrepare { device_id } => {

            let id = DeviceId(device_id.clone());
            let Some(snap) = snapshot else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            if snap.is_system_target(&id) {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "preparação de multi-boot em disco de sistema recusada");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                return;
            }
            let Some(part) = snap.partition_by_id(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "partição alvo não encontrada",
                )));
                return;
            };
            if !matches!(part.fs, FsKind::Vfat) {
                let _ = tx.send(AppEvent::Toast(Toast::error(format!(
                    "a partição precisa estar formatada como FAT32 para multi-boot (atual: {}) — formate primeiro com [f]",
                    part.fs.label()
                ))));
                return;
            }
            let part_path = part.id.0.clone();
            let existing_mount = part.mount_points.first().cloned();
            let mount_point = match ensure_mounted(conn, &part_path, existing_mount).await {
                Ok(mp) => mp,
                Err(e) => {
                    let _ = tx.send(AppEvent::Toast(Toast::error(format!(
                        "falha ao montar partição de dados: {e}"
                    ))));
                    return;
                }
            };
            tracing::warn!(target: "hal9001::storage", device = %device_id, mount = %mount_point, "preparação de multi-boot solicitada");
            let txc = tx.clone();
            tokio::spawn(multiboot_prepare_task(device_id, mount_point, txc));
        }
        Action::StorageMultibootListIsos { device_id } => {
            let id = DeviceId(device_id.clone());
            let Some(snap) = snapshot else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            let Some(part) = snap.partition_by_id(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "partição alvo não encontrada",
                )));
                return;
            };
            let part_path = part.id.0.clone();
            let existing_mount = part.mount_points.first().cloned();
            let conn2 = conn.clone();
            let txc = tx.clone();
            tokio::spawn(multiboot_list_isos_task(
                conn2,
                device_id,
                part_path,
                existing_mount,
                txc,
            ));
        }
        Action::StorageMultibootAddIso {
            device_id,
            src_path,
        } => {
            let id = DeviceId(device_id.clone());
            let Some(snap) = snapshot else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            if snap.is_system_target(&id) {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "escrita de ISO em disco de sistema recusada (gerenciador multi-boot)");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                return;
            }
            let Some(part) = snap.partition_by_id(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "partição alvo não encontrada",
                )));
                return;
            };
            let part_path = part.id.0.clone();
            let existing_mount = part.mount_points.first().cloned();
            tracing::warn!(target: "hal9001::storage", device = %device_id, src = %src_path, "cópia de ISO para o multi-boot solicitada");
            let conn2 = conn.clone();
            let txc = tx.clone();
            tokio::spawn(multiboot_add_iso_task(
                conn2,
                device_id,
                part_path,
                existing_mount,
                src_path,
                txc,
            ));
        }
        Action::StorageMultibootRemoveIso {
            device_id,
            file_name,
        } => {
            let id = DeviceId(device_id.clone());
            let Some(snap) = snapshot else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            if snap.is_system_target(&id) {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "remoção de ISO em disco de sistema recusada (gerenciador multi-boot)");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                return;
            }
            let Some(part) = snap.partition_by_id(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "partição alvo não encontrada",
                )));
                return;
            };
            let part_path = part.id.0.clone();
            let existing_mount = part.mount_points.first().cloned();
            tracing::warn!(target: "hal9001::storage", device = %device_id, file = %file_name, "remoção de ISO do multi-boot solicitada");
            let conn2 = conn.clone();
            let txc = tx.clone();
            tokio::spawn(multiboot_remove_iso_task(
                conn2,
                device_id,
                part_path,
                existing_mount,
                file_name,
                txc,
            ));
        }
        _ => {}
    }
}
