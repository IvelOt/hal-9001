//! Backend de Armazenamento & Discos via UDisks2 (D-Bus / `zbus`).
//!
//! Constrói um [`StorageSnapshot`] periódico (drives → partições) a partir do
//! `org.freedesktop.DBus.ObjectManager` do UDisks2, enriquece com uso de disco
//! via `sysinfo`/`/proc/mounts`, e aplica a trava de segurança [`is_system_disk`]
//! que impede que discos de sistema virem alvo de operações destrutivas.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use sysinfo::Disks;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue};
use zbus::Connection;

use crate::events::{Action, AppEvent, DeviceId, EventTx, Toast};

/// Tamanho do buffer de I/O para checksum e gravação de blocos (4 MiB),
/// conforme especificado no Épico H.
const IO_BUFFER_SIZE: usize = 4 * 1024 * 1024;
/// Intervalo mínimo entre emissões de progresso (evita inundar o canal).
const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);

const UDISKS_SERVICE: &str = "org.freedesktop.UDisks2";
const UDISKS_ROOT: &str = "/org/freedesktop/UDisks2";

/// Sistema de arquivos reportado por `Block.IdType`.
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

    /// Rótulo curto exibido na árvore/detalhes.
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

/// Barramento de conexão do drive (`Drive.ConnectionBus`).
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

/// Uma partição (ou o filesystem bruto de um disco não particionado).
#[derive(Debug, Clone, PartialEq)]
pub struct PartitionInfo {
    pub id: DeviceId,
    pub dev_node: String,
    pub label: String,
    pub fs: FsKind,
    pub size: u64,
    /// Bytes usados na montagem, quando disponível (via `sysinfo`).
    pub used: Option<u64>,
    pub mount_points: Vec<String>,
    /// `true` quando o nó de dispositivo aparece em `/proc/swaps` como ativo.
    pub is_swap: bool,
    /// Trava de segurança: `true` quando esta partição NUNCA pode ser alvo de
    /// operação destrutiva (ver [`is_system_disk`]).
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

/// Um disco físico/lógico (`Drive`) e suas partições.
#[derive(Debug, Clone, PartialEq)]
pub struct DriveInfo {
    pub id: DeviceId,
    pub dev_node: String,
    pub model: String,
    pub vendor: String,
    pub size: u64,
    pub removable: bool,
    pub ejectable: bool,
    pub bus: BusType,
    /// `true` para HDD (mídia rotacional); `false` para SSD/NVMe.
    pub rotational: bool,
    /// Trava de segurança: `true` quando o drive hospeda qualquer partição de
    /// sistema, ou é um disco fixo interno (ver [`is_system_disk`]).
    pub is_system: bool,
    pub partitions: Vec<PartitionInfo>,
}

/// Snapshot completo da árvore de discos, emitido a cada refresh do backend.
#[derive(Debug, Clone, PartialEq)]
pub struct StorageSnapshot {
    pub udisks_available: bool,
    pub drives: Vec<DriveInfo>,
}

/// Uma linha "achatada" da árvore drive→partição, usada pela navegação da UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageRow {
    Drive(usize),
    Partition(usize, usize),
}

impl StorageSnapshot {
    /// Achata a árvore em linhas navegáveis (drive, depois suas partições).
    pub fn rows(&self) -> Vec<StorageRow> {
        let mut rows = Vec::new();
        for (di, drive) in self.drives.iter().enumerate() {
            rows.push(StorageRow::Drive(di));
            for pi in 0..drive.partitions.len() {
                rows.push(StorageRow::Partition(di, pi));
            }
        }
        rows
    }

    pub fn drive(&self, idx: usize) -> Option<&DriveInfo> {
        self.drives.get(idx)
    }

    pub fn partition(&self, drive_idx: usize, part_idx: usize) -> Option<&PartitionInfo> {
        self.drives.get(drive_idx)?.partitions.get(part_idx)
    }

    /// `true` quando `id` identifica um drive ou partição marcados como
    /// disco de sistema — usado como camada 2/3 da trava de segurança em
    /// `App::dispatch` e em `handle_action` (revalidação TOCTOU).
    pub fn is_system_target(&self, id: &DeviceId) -> bool {
        self.drives.iter().any(|d| &d.id == id && d.is_system)
            || self
                .drives
                .iter()
                .flat_map(|d| &d.partitions)
                .any(|p| &p.id == id && p.is_system)
    }

    /// Nó de dispositivo (`/dev/sdX`) do drive identificado, usado para abrir
    /// o dispositivo de bloco na gravação de ISO.
    pub fn drive_dev_node(&self, id: &DeviceId) -> Option<String> {
        self.drives
            .iter()
            .find(|d| &d.id == id)
            .map(|d| d.dev_node.clone())
    }

    /// Capacidade total (bytes) do drive identificado.
    pub fn drive_size(&self, id: &DeviceId) -> Option<u64> {
        self.drives.iter().find(|d| &d.id == id).map(|d| d.size)
    }
}

/// Calcula taxa de transferência (MB/s) e ETA (segundos) a partir dos bytes
/// transferidos numa janela de tempo curta e do total restante. Função pura,
/// testável sem I/O real.
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

/// Trava de segurança inegociável: decide se `partition` (pertencente a
/// `drive`) é um alvo protegido, isto é, **nunca** pode ser formatado,
/// particionado, ejetado ou gravado por cima.
///
/// Critérios (qualquer um basta):
/// 1. A partição está montada em `/`, `/boot`, `/boot/efi` ou `/home`.
/// 2. A partição é uma partição de swap ativa (`/proc/swaps`).
/// 3. O drive é um disco fixo interno (não removível e não-USB) — heurística
///    conservadora: discos internos nunca são alvo por padrão.
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

// ---------------------------------------------------------------------------
// Parsers puros (testáveis)
// ---------------------------------------------------------------------------

/// Parseia `/proc/swaps`, devolvendo os nós de dispositivo (`/dev/...`) de
/// swaps ativas (ignora a linha de cabeçalho e swaps em arquivo).
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

/// Parseia `/proc/mounts`, devolvendo pares `(dispositivo, ponto_de_montagem)`.
/// Usado como reconciliação/fallback quando UDisks2 não reporta `MountPoints`.
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

/// `/proc/mounts` escapa espaços/tabs/etc como `\040` etc.; desfaz o escape.
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

// ---------------------------------------------------------------------------
// Extração de propriedades D-Bus (ObjectManager)
// ---------------------------------------------------------------------------

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

/// Converte um `ay` (bytes NUL-terminados) num `String`, usado por
/// `Block.Device` (nó `/dev/...`).
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

/// Converte `aay` (`MountPoints`) numa lista de `String`.
fn prop_mount_points(props: &PropMap, key: &str) -> Vec<String> {
    props
        .get(key)
        .and_then(|v| Vec::<Vec<u8>>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .iter()
        .map(|b| bytes_to_path(b))
        .collect()
}

// ---------------------------------------------------------------------------
// Montagem do snapshot a partir de `GetManagedObjects`
// ---------------------------------------------------------------------------

/// Constrói o [`StorageSnapshot`] a partir dos objetos gerenciados do UDisks2,
/// enriquecendo com `/proc/swaps` (detecção de swap ativa) e uso de disco via
/// `sysinfo` para partições montadas.
fn build_snapshot(objects: &ManagedObjects, swaps_text: &str, disks: &Disks) -> StorageSnapshot {
    let active_swaps = parse_proc_swaps(swaps_text);

    // Primeiro passo: monta o esqueleto de cada Drive (sem partições ainda).
    let mut drives: Vec<DriveInfo> = Vec::new();
    let mut drive_path_index: HashMap<String, usize> = HashMap::new();

    for (path, ifaces) in objects.iter() {
        let Some(drive_props) = ifaces.get("org.freedesktop.UDisks2.Drive") else {
            continue;
        };
        let rotation_rate = prop_i32(drive_props, "RotationRate").unwrap_or(-1);
        let drive = DriveInfo {
            id: DeviceId(path.as_str().to_string()),
            dev_node: String::new(), // preenchido a partir do bloco raiz, se houver.
            model: prop_string(drive_props, "Model").unwrap_or_default(),
            vendor: prop_string(drive_props, "Vendor").unwrap_or_default(),
            size: prop_u64(drive_props, "Size").unwrap_or(0),
            removable: prop_bool(drive_props, "Removable").unwrap_or(false),
            ejectable: prop_bool(drive_props, "Ejectable").unwrap_or(false),
            bus: BusType::parse(&prop_string(drive_props, "ConnectionBus").unwrap_or_default()),
            rotational: rotation_rate > 0,
            is_system: false,
            partitions: Vec::new(),
        };
        drive_path_index.insert(path.as_str().to_string(), drives.len());
        drives.push(drive);
    }

    // Segundo passo: cada Block pertencente a um Drive vira uma PartitionInfo
    // (partição de fato, ou o filesystem bruto do disco não particionado).
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

        // O bloco "raiz" (sem entrada de partição) só descreve o nó /dev do
        // drive; só vira uma linha de partição se tiver filesystem próprio
        // (disco não particionado, ex.: pendrive gravado direto com `dd`).
        let id_type = prop_string(block_props, "IdType").unwrap_or_default();
        if is_whole_disk_block {
            drives[drive_idx].dev_node = dev_node.clone();
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

    // Terceiro passo: aplica a trava de segurança (por partição e agregada ao
    // drive) e ordena partições pelo nó de dispositivo para exibição estável.
    for drive in drives.iter_mut() {
        drive.partitions.sort_by(|a, b| a.dev_node.cmp(&b.dev_node));
        let mut drive_is_system = false;
        for idx in 0..drive.partitions.len() {
            let system =
                drive.partitions[idx].is_system || is_system_disk(drive, &drive.partitions[idx]);
            drive.partitions[idx].is_system = system;
            drive_is_system |= system;
        }
        // Discos fixos internos (heurística #3) são sistema mesmo sem
        // partições reconhecidas ainda (ex.: sysfs incompleto no boot).
        drive_is_system |= !drive.removable && drive.bus != BusType::Usb;
        drive.is_system = drive_is_system;
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

// ---------------------------------------------------------------------------
// Cliente D-Bus (métodos)
// ---------------------------------------------------------------------------

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

async fn mount(conn: &Connection, path: &str) -> anyhow::Result<()> {
    udisks_call(conn, path, "org.freedesktop.UDisks2.Filesystem", "Mount").await
}

async fn unmount(conn: &Connection, path: &str) -> anyhow::Result<()> {
    udisks_call(conn, path, "org.freedesktop.UDisks2.Filesystem", "Unmount").await
}

async fn eject(conn: &Connection, path: &str) -> anyhow::Result<()> {
    // Tenta `Eject`; alguns drivers só suportam `PowerOff` (corta a energia do
    // controlador USB) — tenta como sequência de segurança do "safe to remove".
    let r = udisks_call(conn, path, "org.freedesktop.UDisks2.Drive", "Eject").await;
    if r.is_ok() {
        return Ok(());
    }
    udisks_call(conn, path, "org.freedesktop.UDisks2.Drive", "PowerOff").await
}

/// Formata o bloco em `path` (drive ou partição) com `fs_type` (`vfat`,
/// `exfat`, `ext4`, `ntfs`, `btrfs`) via `Block.Format` do UDisks2, que
/// encapsula o `mkfs.*` correspondente.
async fn format_block(conn: &Connection, path: &str, fs_type: &str, label: &str) -> anyhow::Result<()> {
    let mut opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    opts.insert("label", zbus::zvariant::Value::from(label));
    opts.insert("update-partition-type", zbus::zvariant::Value::from(true));
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

// ---------------------------------------------------------------------------
// ISO Flasher — checksum SHA256 e gravação de blocos em streaming (Épico H)
// ---------------------------------------------------------------------------

/// Lê `iso_path` em blocos de 4 MiB calculando o SHA256, emitindo progresso
/// throttled a cada ~200ms. Roda inteiramente numa task Tokio — nunca bloqueia
/// a thread de render.
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

/// Grava `iso_path` no dispositivo de bloco `dev_node` em blocos de 4 MiB,
/// emitindo progresso (%, MB/s, ETA) a cada ~200ms. Ao final, garante
/// `fsync` + `sync()` de kernel antes de reportar sucesso — 100% escrito
/// **não** é sucesso até o sync retornar (evita a armadilha clássica do
/// cache de página).
async fn flash_inner(
    iso_path: &str,
    dev_node: &str,
    cancel: &Arc<AtomicBool>,
    tx: &EventTx,
) -> anyhow::Result<()> {
    let mut src = tokio::fs::File::open(iso_path).await?;
    let total_bytes = src.metadata().await?.len();
    let mut dst = tokio::fs::OpenOptions::new()
        .write(true)
        .open(dev_node)
        .await?;

    let mut buf = vec![0u8; IO_BUFFER_SIZE];
    let mut written: u64 = 0;
    let mut window_started = tokio::time::Instant::now();
    let mut window_bytes: u64 = 0;

    loop {
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("gravação cancelada pelo usuário");
        }
        let n = src.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n]).await?;
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
            window_started = tokio::time::Instant::now();
            window_bytes = 0;
        }
    }

    // Syncing: fsync do FD + sync() global de kernel — só então o "100%" vira
    // sucesso de fato (H4).
    dst.flush().await?;
    dst.sync_all().await?;
    // SAFETY: `sync(2)` não recebe ponteiros e não pode falhar de forma
    // insegura; apenas força o flush de todos os buffers do kernel.
    unsafe {
        libc::sync();
    }

    let _ = tx.send(AppEvent::StorageFlashProgress {
        bytes_written: total_bytes,
        total_bytes,
        speed_mbps: 0.0,
        eta_secs: 0,
    });
    Ok(())
}

async fn flash_task(
    device_id: String,
    iso_path: String,
    dev_node: String,
    cancel: Arc<AtomicBool>,
    tx: EventTx,
) {
    let result = flash_inner(&iso_path, &dev_node, &cancel, &tx)
        .await
        .map(|()| "gravação concluída com sucesso".to_string())
        .map_err(|e| e.to_string());
    let _ = tx.send(AppEvent::StorageFlashDone { device_id, result });
}

// ---------------------------------------------------------------------------
// Task de polling / dispatcher
// ---------------------------------------------------------------------------

/// Reconecta/consulta o UDisks2 e monta o snapshot, ou devolve o erro de
/// conexão/consulta para que o chamador degrade graciosamente.
async fn refresh_snapshot(conn: &Option<Connection>) -> anyhow::Result<StorageSnapshot> {
    let Some(c) = conn else {
        return Err(anyhow::anyhow!("sem conexão D-Bus"));
    };
    let objects = get_managed_objects(c).await?;
    let swaps = std::fs::read_to_string("/proc/swaps").unwrap_or_default();
    let disks = Disks::new_with_refreshed_list();
    Ok(build_snapshot(&objects, &swaps, &disks))
}

/// Task raiz do Módulo 4: conecta ao system bus, publica snapshots periódicos
/// e despacha ações de montagem/desmontagem/ejeção. Degrada graciosamente
/// (via `AppEvent::ServiceDegraded`) quando o UDisks2 não está disponível,
/// tentando reconectar a cada tick.
pub async fn run(
    poll_ms: u64,
    tx: EventTx,
    mut actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms.max(1000)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut conn: Option<Connection> = Connection::system().await.ok();
    let mut last_snapshot: Option<StorageSnapshot> = None;
    // Tokens de cancelamento das gravações de ISO em curso, por `device_id`.
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
                Ok(action) => {
                    if let Some(c) = &conn {
                        handle_action(c, action, &last_snapshot, &tx, &mut flash_cancels).await;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
        }
    }
    Ok(())
}

/// Revalida a trava de segurança e despacha a ação D-Bus correspondente,
/// emitindo um toast de confirmação/erro.
async fn handle_action(
    conn: &Connection,
    action: Action,
    snapshot: &Option<StorageSnapshot>,
    tx: &EventTx,
    flash_cancels: &mut HashMap<String, Arc<AtomicBool>>,
) {
    match action {
        Action::StorageMount(id) => {
            let toast = match mount(conn, &id.0).await {
                Ok(()) => Toast::info("Dispositivo montado"),
                Err(e) => Toast::error(format!("Falha ao montar: {e}")),
            };
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageUnmount(id) => {
            let toast = match unmount(conn, &id.0).await {
                Ok(()) => Toast::info("Dispositivo desmontado"),
                Err(e) => Toast::error(format!("Falha ao desmontar (dispositivo em uso?): {e}")),
            };
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageEject(id) => {
            // Guarda no backend (camada 3, TOCTOU): revalida contra o último
            // snapshot conhecido antes de tocar o D-Bus.
            if let Some(snap) = snapshot {
                if let Some(drive) = snap.drives.iter().find(|d| d.id == id) {
                    if drive.is_system {
                        let _ = tx.send(AppEvent::Toast(Toast::error(
                            "operação bloqueada: disco de sistema",
                        )));
                        tracing::warn!(target: "hal9001::storage", drive = %id.0, "ejeção de disco de sistema recusada");
                        return;
                    }
                    // Desmonta todas as partições montadas antes de ejetar.
                    for part in &drive.partitions {
                        if part.is_mounted() {
                            let _ = unmount(conn, &part.id.0).await;
                        }
                    }
                }
            }
            let toast = match eject(conn, &id.0).await {
                Ok(()) => Toast::info("Seguro remover o dispositivo"),
                Err(e) => Toast::error(format!("Falha ao ejetar: {e}")),
            };
            tracing::warn!(target: "hal9001::storage", drive = %id.0, "ejeção de dispositivo executada");
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageRefresh => {
            // O próximo tick já republica o snapshot; nada a fazer aqui além
            // de reservar o braço para clareza do fluxo.
        }
        Action::StorageFormat {
            device_id,
            fs_type,
            label,
        } => {
            // Camada 3 (TOCTOU): revalida contra o último snapshot conhecido
            // imediatamente antes de tocar o D-Bus — o alvo pode ter mudado
            // entre a UI e a execução da ação.
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
            tracing::warn!(target: "hal9001::storage", device = %device_id, fs = %fs_type, label = %label, "formatação solicitada");
            let toast = match format_block(conn, &device_id, &fs_type, &label).await {
                Ok(()) => Toast::info("Formatação concluída"),
                Err(e) => Toast::error(format!("Falha ao formatar: {e}")),
            };
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageChecksumIso(iso_path) => {
            let txc = tx.clone();
            tokio::spawn(checksum_task(iso_path, txc));
        }
        Action::StorageFlashIso { device_id, iso_path } => {
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
            tracing::warn!(target: "hal9001::storage", device = %device_id, dev_node = %dev_node, iso = %iso_path, "gravação de ISO solicitada");
            let cancel = Arc::new(AtomicBool::new(false));
            flash_cancels.insert(device_id.clone(), cancel.clone());
            let txc = tx.clone();
            tokio::spawn(flash_task(device_id, iso_path, dev_node, cancel, txc));
        }
        Action::StorageFlashCancel { device_id } => {
            if let Some(cancel) = flash_cancels.get(&device_id) {
                cancel.store(true, Ordering::Relaxed);
            }
        }
        _ => {}
    }
}
