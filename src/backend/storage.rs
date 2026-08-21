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

use crate::events::{
    Action, AppEvent, DeviceId, EventTx, SudoPasswordRequest, SudoPasswordTx, Toast,
};

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
    /// Caminho de objeto D-Bus (`/org/freedesktop/UDisks2/block_devices/sdX`)
    /// do bloco "raiz" (disco inteiro) deste drive, quando conhecido. Só a
    /// interface `Block` deste objeto existe — a interface `Block` nunca
    /// existe no caminho de objeto do `Drive` em si (ver
    /// [`resolve_block_object_path`]).
    pub block_path: Option<String>,
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
    /// `true` quando o drive é um pendrive Ventoy (layout de duas partições
    /// com uma pequena `VTOYEFI` + a partição de dados onde ficam as ISOs).
    /// Ver [`detect_ventoy`].
    pub is_ventoy: bool,
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

    /// Busca um drive pelo `DeviceId` (caminho do objeto D-Bus).
    pub fn drive_by_id<'a>(&'a self, id: &DeviceId) -> Option<&'a DriveInfo> {
        self.drives.iter().find(|d| &d.id == id)
    }

    /// Nó de dispositivo (`/dev/sdX`) do bloco no caminho de objeto
    /// `block_path` (o disco inteiro de um drive, ou uma de suas partições)
    /// — usado para abrir o dispositivo diretamente no formatador FAT32 em
    /// Rust puro, que não passa pelo `Block.Format` do UDisks2.
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

/// Prefixo de caminho de objeto D-Bus de um dispositivo de bloco do UDisks2 —
/// a única localização onde a interface `org.freedesktop.UDisks2.Block`
/// realmente existe.
const BLOCK_DEVICE_PREFIX: &str = "/org/freedesktop/UDisks2/block_devices/";
/// Prefixo de caminho de objeto D-Bus de um `Drive` do UDisks2 — a interface
/// `Block` NUNCA existe neste caminho.
const DRIVE_PREFIX: &str = "/org/freedesktop/UDisks2/drives/";

/// Resolve `target_id` (que pode ser o caminho de objeto de um `Drive` ou já
/// de um `block_device`) para o caminho de objeto de bloco correto onde a
/// interface `org.freedesktop.UDisks2.Block` existe de fato.
///
/// Esta é a correção central do bug de formatação: `Block.Format` só pode ser
/// chamado num caminho de `block_devices/...`. Quando o usuário seleciona um
/// `Drive` (`/org/freedesktop/UDisks2/drives/...`), é preciso localizar o
/// bloco "raiz" correspondente (o disco inteiro, sem entrada de partição) via
/// [`DriveInfo::block_path`].
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

/// Lista os caminhos de objeto de todas as partições atualmente montadas que
/// precisam ser desmontadas antes de formatar `target_id`: todas as
/// partições montadas do drive (quando `target_id` é um `Drive`), ou apenas a
/// própria partição (quando `target_id` já é uma partição montada).
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

/// Binário `mkfs.*` e pacote que o fornece, por tipo de sistema de arquivos —
/// usado para transformar um erro genérico do UDisks2 ("comando não
/// encontrado") numa instrução acionável para o usuário.
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

/// `true` quando o erro cru do `Block.Format` (D-Bus) indica que o `mkfs.*`
/// correspondente não está instalado no host (executável ausente do PATH do
/// UDisks2/`udisksd`).
fn is_missing_mkfs_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_ascii_lowercase();
    lower.contains("not found")
        || lower.contains("no such file or directory")
        || lower.contains("command not found")
        || lower.contains("failed to execute")
}

/// Traduz o erro cru do `Block.Format` (D-Bus) numa mensagem de toast clara.
/// Quando o erro indica que o `mkfs.*` correspondente não está instalado no
/// host, devolve uma instrução acionável em vez do erro D-Bus bruto.
fn format_error_message(fs_type: &str, err: &anyhow::Error) -> String {
    if is_missing_mkfs_error(err) {
        if let Some((bin, pkg)) = mkfs_hint(fs_type) {
            return format!("{bin} ausente — instale {pkg}");
        }
    }
    format!("Falha ao formatar: {err}")
}

/// `true` quando o erro cru (D-Bus ou I/O) indica que a operação foi negada
/// por falta de permissão do processo local (não pertence ao grupo `disk`,
/// nem é root) — o gatilho para cair para o fluxo de elevação interativa
/// (`pkexec`/`sudo`) em vez de expor um `Permission denied` cru ao usuário.
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

/// `true` quando o erro cru do `Block.Format`/`Block.OpenDevice` (D-Bus)
/// indica que a chamada foi recusada pelo Polkit por falta de um agente de
/// autenticação gráfico ativo na sessão (`NotAuthorized` /
/// "No polkit agent available to authenticate"), tipicamente numa sessão TTY
/// pura sem `polkit-gnome`/`polkit-kde` rodando — o gatilho para cair para o
/// fluxo de elevação interativa via `pkexec`/`sudo` num terminal suspenso.
pub fn is_not_authorized_error(err: &anyhow::Error) -> bool {
    let lower = err.to_string().to_ascii_lowercase();
    lower.contains("notauthorized")
        || lower.contains("not authorized")
        || lower.contains("no polkit agent")
        || lower.contains("authentication is required")
}

/// Monta o binário `mkfs.*` e os argumentos necessários para formatar
/// `dev_node` como `fs_type` com o rótulo `label`, para uso pelo fallback de
/// elevação via `sudo -S`/`sudo -n` (ver [`format_via_sudo`]) quando o
/// `Block.Format` do UDisks2 é recusado por falta de agente Polkit. Devolve
/// `None` quando `fs_type` não tem um `mkfs.*` mapeado (ver [`mkfs_hint`]).
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

// ---------------------------------------------------------------------------
// Elevação via `sudo -S` (senha pelo modal nativo da TUI, sem suspender o
// terminal) — substitui o antigo fluxo de `pkexec`/`sudo` com stdio herdado.
// ---------------------------------------------------------------------------

/// `true` quando `sudo -n true` passa sem exigir senha — cache de
/// autenticação válido, `NOPASSWD` no sudoers, ou processo já rodando como
/// root. Usado para decidir se o modal de senha precisa ser exibido.
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

/// Pede a senha de sudo ao usuário via modal nativo da TUI (canal
/// `sudo_tx`), devolvendo `None` quando o usuário cancela (`Esc`).
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

/// `true` quando o texto de stderr do `sudo` indica senha incorreta/recusada
/// — o gatilho para o `App` reabrir o modal com "Senha incorreta" e permitir
/// nova tentativa, em vez de tratar como falha definitiva do comando.
pub fn is_sudo_auth_failure(stderr_text: &str) -> bool {
    let lower = stderr_text.to_ascii_lowercase();
    lower.contains("incorrect password")
        || lower.contains("sorry, try again")
        || lower.contains("senha incorreta")
        || lower.contains("no password was provided")
        || lower.contains("a password is required")
}

/// Resultado (linhas de progresso + status final) de um comando rodado sob
/// `sudo` via [`spawn_sudo`].
struct SudoRun {
    lines: tokio::sync::mpsc::UnboundedReceiver<String>,
    handle: tokio::task::JoinHandle<anyhow::Result<(std::process::ExitStatus, String)>>,
}

/// Lê `reader` byte a byte, quebrando em "linhas" tanto por `\n` quanto por
/// `\r` (necessário para acompanhar saídas como `dd status=progress`, que
/// atualiza a mesma linha via `\r`), encaminhando cada uma para `line_tx` e
/// devolvendo o texto completo acumulado (usado para detectar falha de
/// autenticação em stderr).
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

/// Roda `program` com `args` sob `sudo`, devolvendo progresso em streaming
/// (linha a linha, `\n` ou `\r`) via `SudoRun::lines` e o status final +
/// texto de stderr (para detecção de senha incorreta) via `SudoRun::handle`.
///
/// Quando `password` é `Some`, executa `sudo -S -k -- program args...`,
/// escrevendo `senha\n` no stdin do processo (nunca herda o stdin/stdout/
/// stderr do HAL-9001 — a TUI nunca é suspensa). Quando `password` é `None`
/// (cache de sudo válido, ver [`sudo_cached`]), executa `sudo -n -- program
/// args...`, que nunca imprime prompt.
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

/// Monta os argumentos de invocação do `sudo` para `program`/`args`, de
/// acordo com a disponibilidade de cache (`cached`) — função pura, usada por
/// [`spawn_sudo`] e testável isoladamente.
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

/// Extrai o total de bytes já copiados de uma linha de progresso do `dd`
/// (`status=progress`), ex.: `"104857600 bytes (105 MB, 100 MiB) copied, 1 s,
/// 100 MB/s"` → `Some(104857600)`. Função pura, testável sem I/O real.
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

/// Pede a senha (a menos que o cache de `sudo -n true` já seja válido na
/// primeira tentativa) e devolve `Some(senha)`/`None` (cache válido) para a
/// próxima chamada de [`spawn_sudo`], ou `Err` quando o usuário cancela
/// (`Esc`) — propagada pelos laços de repetição de `format_via_sudo`/
/// `flash_elevated`/`ventoy_task` como falha definitiva da operação.
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

/// `true` quando `fs_type` (como enviado ao `Block.Format`) identifica um
/// sistema de arquivos FAT — o único formato para o qual o HAL-9001 tem um
/// formatador 100% Rust puro (`fatfs`) como fallback ao `mkfs.vfat` do host.
fn is_fat_fs_type(fs_type: &str) -> bool {
    matches!(
        fs_type.trim().to_ascii_lowercase().as_str(),
        "vfat" | "fat32" | "fat"
    )
}

/// Converte um rótulo arbitrário no formato de 11 bytes exigido pelo campo
/// `VolumeLabel` da BPB do FAT: maiúsculas ASCII, truncado/preenchido com
/// espaços.
fn fat_volume_label(label: &str) -> [u8; 11] {
    let mut buf = [b' '; 11];
    for (i, b) in label.bytes().take(11).enumerate() {
        buf[i] = b.to_ascii_uppercase();
    }
    buf
}

/// Executa `fatfs::format_volume` sobre um arquivo/descritor já aberto para
/// leitura+escrita, sincronizando o conteúdo em disco (`sync_all`) antes de
/// devolver o controle — núcleo compartilhado por [`format_fat32_pure_rust`]
/// (testes, abre o caminho diretamente) e pelo fluxo de produção, que abre o
/// descritor via `Block.OpenDevice` do UDisks2 (ver `open_device_fd`).
fn format_fat32_on_file(mut file: std::fs::File, label: &str) -> anyhow::Result<()> {
    let options = fatfs::FormatVolumeOptions::new()
        .fat_type(fatfs::FatType::Fat32)
        .volume_label(fat_volume_label(label));
    fatfs::format_volume(&mut file, options)?;
    file.sync_all()?;
    Ok(())
}

/// Formata `dev_node` (nó de bloco, ex.: `/dev/sdz` ou `/dev/sdz1`) como
/// FAT32 usando exclusivamente a crate `fatfs` — 100% Rust puro, sem invocar
/// nenhum binário externo do host (`mkfs.vfat`/`dosfstools`).
///
/// Formata o volume diretamente no nó de bloco recebido (disco inteiro ou
/// partição já existente) em vez de escrever uma tabela de partição MBR
/// própria: o nó já foi resolvido pelo UDisks2/`resolve_block_object_path`
/// exatamente como o `mkfs.vfat` do host o receberia, então o mesmo alvo é
/// reaproveitado aqui sem duplicar a lógica de particionamento.
///
/// Abre o caminho diretamente via `std::fs::OpenOptions` — usado pelos
/// testes (que apontam para arquivos regulares). O fluxo de produção usa
/// [`open_device_fd`] + [`format_fat32_on_file`] para obter permissão via
/// Polkit/D-Bus em vez de depender do grupo `disk` do processo local.
pub fn format_fat32_pure_rust(dev_node: &str, label: &str) -> anyhow::Result<()> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev_node)?;
    format_fat32_on_file(file, label)
}

/// Uma entrada `.iso`/`.img` encontrada na raiz da partição de dados de um
/// pendrive Ventoy.
#[derive(Debug, Clone, PartialEq)]
pub struct VentoyIsoEntry {
    pub name: String,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

/// Reconhece o rótulo (`IdLabel`) de qualquer partição criada pelo instalador
/// do Ventoy — `VTOYEFI` (pequena partição EFI de boot) ou `Ventoy` (partição
/// de dados). A checagem por rótulo é suficiente e evita heurísticas frágeis
/// por tipo de filesystem.
pub fn detect_ventoy(partitions: &[PartitionInfo]) -> bool {
    partitions
        .iter()
        .any(|p| p.label.eq_ignore_ascii_case("ventoy") || p.label.eq_ignore_ascii_case("vtoyefi"))
}

/// Partição de dados do Ventoy (onde ficam as ISOs) — a partição do drive
/// rotulada `Ventoy`, ou, na ausência desse rótulo exato, a maior partição
/// que não seja a `VTOYEFI`. Retorna `None` quando o drive não é Ventoy.
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

/// Decide se `name` tem extensão de imagem que o gerenciador de ISOs do
/// Ventoy reconhece (`.iso`/`.img`, sem diferenciar maiúsculas/minúsculas).
pub fn is_iso_or_img(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".iso") || lower.ends_with(".img")
}

/// Filtra e ordena (alfabeticamente, sem diferenciar caixa) entradas brutas
/// de diretório em [`VentoyIsoEntry`]s — função pura, testável sem tocar o
/// filesystem real.
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
            block_path: None,        // idem.
            model: prop_string(drive_props, "Model").unwrap_or_default(),
            vendor: prop_string(drive_props, "Vendor").unwrap_or_default(),
            size: prop_u64(drive_props, "Size").unwrap_or(0),
            removable: prop_bool(drive_props, "Removable").unwrap_or(false),
            ejectable: prop_bool(drive_props, "Ejectable").unwrap_or(false),
            bus: BusType::parse(&prop_string(drive_props, "ConnectionBus").unwrap_or_default()),
            rotational: rotation_rate > 0,
            is_system: false,
            is_ventoy: false,
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

/// Espaço livre (bytes) no ponto de montagem informado, consultado via
/// `sysinfo` — usado para exibir o espaço restante no gerenciador de ISOs do
/// Ventoy antes de copiar uma nova imagem.
fn free_bytes_for_mount(mount_point: &str) -> Option<u64> {
    let disks = Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .find(|d| d.mount_point().to_string_lossy() == mount_point)
        .map(|d| d.available_space())
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

/// Monta `path` e devolve o ponto de montagem escolhido pelo UDisks2 (o
/// método `Filesystem.Mount` retorna a string do caminho). Usado pelo
/// gerenciador de ISOs do Ventoy, que precisa do caminho real para ler/
/// escrever arquivos na raiz da partição de dados.
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

/// Garante que a partição em `part_path` esteja montada, reaproveitando
/// `existing_mount` (do último snapshot conhecido) quando disponível, ou
/// montando-a via D-Bus caso contrário.
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
async fn format_block(
    conn: &Connection,
    path: &str,
    fs_type: &str,
    label: &str,
) -> anyhow::Result<()> {
    let mut opts: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
    opts.insert("label", zbus::zvariant::Value::from(label));
    opts.insert("update-partition-type", zbus::zvariant::Value::from(true));
    // Permite que o UDisks2 limpe tabelas de partição/assinaturas
    // preexistentes (ex.: imagens Ventoy/ISO gravadas via `dd`) antes de
    // criar o novo filesystem.
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

/// Abre `block_path` via `Block.OpenDevice("rw", {"flags": O_SYNC|O_EXCL})`
/// do UDisks2, devolvendo um `std::fs::File` já construído a partir do
/// descritor recebido por D-Bus (fd-passing/`SCM_RIGHTS`).
///
/// A autorização passa pelo Polkit do UDisks2 (`udisksd` roda como root), o
/// que garante permissão total de leitura/escrita sobre o nó de bloco sem
/// depender do processo do HAL-9001 pertencer ao grupo `disk` nem de
/// heurísticas de userspace — elimina a classe inteira de erros
/// `Permission denied` ao formatar via `fatfs` em vez do `mkfs.vfat` do host.
/// `O_SYNC` garante que cada escrita do `fatfs` seja persistida
/// imediatamente; `O_EXCL` recusa a abertura se outro processo já tiver o
/// dispositivo aberto em modo exclusivo.
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

// ---------------------------------------------------------------------------
// Ventoy — instalação via `scripts/ventoy.sh` (Gadget/Script)
// ---------------------------------------------------------------------------

/// Resolve o caminho do `scripts/ventoy.sh`, na ordem: `$HAL9001_VENTOY_SCRIPT`
/// (override explícito), diretório do binário em execução, e — apenas em
/// builds de desenvolvimento — o diretório-fonte do crate.
fn resolve_ventoy_script_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("HAL9001_VENTOY_SCRIPT") {
        let p = std::path::PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("scripts/ventoy.sh");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    for prefix in ["/usr/local/share/hal9001", "/usr/share/hal9001"] {
        let candidate = std::path::Path::new(prefix).join("scripts/ventoy.sh");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    #[cfg(debug_assertions)]
    {
        let candidate = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/ventoy.sh");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Executa `sudo -S -k -- bash scripts/ventoy.sh <dev_node>` (ou `sudo -n --`
/// quando o cache de autenticação já é válido), repassando cada linha de
/// saída (stdout/stderr) como progresso via `StorageVentoyProgress` e pedindo
/// a senha ao usuário pelo modal nativo da TUI quando necessário — nunca
/// suspende o terminal nem herda seu stdio, então nunca corrompe a grade do
/// Ratatui.
async fn ventoy_task(device_id: String, dev_node: String, tx: EventTx, sudo_tx: SudoPasswordTx) {
    let Some(script) = resolve_ventoy_script_path() else {
        let _ = tx.send(AppEvent::StorageVentoyDone {
            device_id,
            result: Err("scripts/ventoy.sh não encontrado".to_string()),
        });
        return;
    };
    let label = format!("Instalar Ventoy em {dev_node}");
    let args = vec![script.to_string_lossy().to_string(), dev_node];
    let mut retry_error: Option<String> = None;

    let result = loop {
        let password = match next_sudo_attempt(&sudo_tx, &label, &mut retry_error).await {
            Ok(pw) => pw,
            Err(msg) => break Err(msg),
        };
        let mut run = match spawn_sudo(password, "bash".to_string(), args.clone()) {
            Ok(r) => r,
            Err(e) => break Err(format!("falha ao executar ventoy.sh: {e}")),
        };
        while let Some(line) = run.lines.recv().await {
            let _ = tx.send(AppEvent::StorageVentoyProgress {
                device_id: device_id.clone(),
                line,
            });
        }
        match run.handle.await {
            Ok(Ok((status, _stderr_text))) if status.success() => {
                break Ok("Ventoy instalado com sucesso".to_string())
            }
            Ok(Ok((_status, stderr_text))) if is_sudo_auth_failure(&stderr_text) => {
                retry_error = Some("Senha incorreta".to_string());
                continue;
            }
            Ok(Ok((status, stderr_text))) => {
                break Err(format!("ventoy.sh terminou com {status}: {stderr_text}"))
            }
            Ok(Err(e)) => break Err(format!("falha ao executar ventoy.sh: {e}")),
            Err(e) => break Err(format!("falha ao executar ventoy.sh: {e}")),
        }
    };
    let _ = tx.send(AppEvent::StorageVentoyDone { device_id, result });
}

/// Roda `bin` (um `mkfs.*` resolvido por [`mkfs_command`]) sobre `dev_node`
/// via `sudo -S`/`sudo -n`, pedindo a senha pelo modal nativo da TUI quando
/// necessário e repetindo em caso de senha incorreta.
async fn format_via_sudo(
    dev_node: &str,
    bin: &str,
    args: &[String],
    sudo_tx: &SudoPasswordTx,
    tx: &EventTx,
) -> Result<(), String> {
    let label = format!("Formatar {dev_node} ({bin})");
    let mut retry_error: Option<String> = None;
    loop {
        let password = match next_sudo_attempt(sudo_tx, &label, &mut retry_error).await {
            Ok(pw) => pw,
            Err(msg) => return Err(msg),
        };
        let mut run =
            spawn_sudo(password, bin.to_string(), args.to_vec()).map_err(|e| e.to_string())?;
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

/// Fallback de gravação de ISO usado quando o processo do HAL-9001 não tem
/// permissão direta sobre `dev_node` (não é root nem pertence ao grupo
/// `disk`): roda `dd` sob `sudo -S`/`sudo -n`, com a senha pedida via modal
/// nativo da TUI quando necessário, convertendo cada linha de
/// `status=progress` em `AppEvent::StorageFlashProgress`.
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
    // SAFETY: `sync(2)` não recebe ponteiros e não pode falhar de forma
    // insegura; apenas força o flush de todos os buffers do kernel.
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
    let total_bytes = tokio::fs::metadata(&iso_path)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    let result = match flash_inner(&iso_path, &dev_node, &cancel, &tx).await {
        Ok(()) => Ok("gravação concluída com sucesso".to_string()),
        Err(e) if is_permission_denied_error(&e) => {
            tracing::warn!(target: "hal9001::storage", device = %device_id, "permissão negada ao abrir dispositivo de bloco — usando fallback de dd elevado");
            let _ = tx.send(AppEvent::Toast(Toast::info(
                "permissão negada — solicitando elevação (sudo) para gravar o dispositivo",
            )));
            flash_elevated(&iso_path, &dev_node, total_bytes, &sudo_tx, &tx)
                .await
                .map(|()| "gravação concluída com sucesso (elevado)".to_string())
                .map_err(|e| e.to_string())
        }
        Err(e) => Err(e.to_string()),
    };
    let _ = tx.send(AppEvent::StorageFlashDone { device_id, result });
}

// ---------------------------------------------------------------------------
// Gerenciador de ISOs do Ventoy — listar/adicionar/remover arquivos na
// partição de dados de um pendrive Ventoy já configurado.
// ---------------------------------------------------------------------------

/// Lê a raiz de `mount_point`, filtra `.iso`/`.img` e emite a listagem
/// (ordenada) junto do espaço livre restante na partição.
async fn list_and_emit(mount_point: &str, device_id: String, tx: &EventTx) {
    let mut raw = Vec::new();
    if let Ok(mut rd) = tokio::fs::read_dir(mount_point).await {
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
    let _ = tx.send(AppEvent::StorageVentoyIsoList {
        device_id,
        entries,
        free_bytes,
    });
}

/// Garante a montagem da partição de dados do Ventoy e publica a listagem
/// atual de ISOs. Falha graciosamente (lista vazia + toast) se a montagem
/// falhar — nunca deixa o gerenciador travado em `Loading`.
async fn ventoy_list_isos_task(
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
                "Falha ao montar partição de dados do Ventoy: {e}"
            ))));
            let _ = tx.send(AppEvent::StorageVentoyIsoList {
                device_id,
                entries: Vec::new(),
                free_bytes: None,
            });
        }
    }
}

/// Copia `src_path` para dentro de `dst_path` em blocos de 4 MiB, emitindo
/// progresso throttled — mesma disciplina do `flash_inner`, mas sem `libc::
/// sync()` global (arquivo regular, não bloco de dispositivo). Garante
/// `flush`/`sync_all` do destino antes de reportar sucesso: uma ISO truncada
/// num pendrive de boot é uma falha real, não cosmética.
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
            let _ = tx.send(AppEvent::StorageVentoyIsoCopyProgress {
                device_id: device_id.to_string(),
                bytes_written: written,
                total_bytes,
            });
            last_emit = tokio::time::Instant::now();
        }
    }

    dst.flush().await?;
    dst.sync_all().await?;

    let _ = tx.send(AppEvent::StorageVentoyIsoCopyProgress {
        device_id: device_id.to_string(),
        bytes_written: total_bytes,
        total_bytes,
    });
    Ok(())
}

async fn ventoy_add_iso_task(
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
            let _ = tx.send(AppEvent::StorageVentoyIsoCopyDone {
                device_id,
                result: Err(format!("falha ao montar partição de dados: {e}")),
            });
            return;
        }
    };
    let file_name = std::path::Path::new(&src_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.iso".to_string());
    let dst_path = format!("{}/{}", mount_point.trim_end_matches('/'), file_name);

    let result = copy_iso_inner(&src_path, &dst_path, &device_id, &tx)
        .await
        .map(|()| file_name.clone())
        .map_err(|e| e.to_string());
    let _ = tx.send(AppEvent::StorageVentoyIsoCopyDone {
        device_id: device_id.clone(),
        result,
    });
    list_and_emit(&mount_point, device_id, &tx).await;
}

async fn ventoy_remove_iso_task(
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
            let _ = tx.send(AppEvent::StorageVentoyIsoRemoveDone {
                device_id,
                result: Err(format!("falha ao montar partição de dados: {e}")),
            });
            return;
        }
    };
    let target = format!("{}/{}", mount_point.trim_end_matches('/'), file_name);
    let result = tokio::fs::remove_file(&target)
        .await
        .map(|()| file_name.clone())
        .map_err(|e| e.to_string());
    let _ = tx.send(AppEvent::StorageVentoyIsoRemoveDone {
        device_id: device_id.clone(),
        result,
    });
    list_and_emit(&mount_point, device_id, &tx).await;
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
    sudo_tx: SudoPasswordTx,
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

/// Revalida a trava de segurança e despacha a ação D-Bus correspondente,
/// emitindo um toast de confirmação/erro.
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
            // `Drive`s do UDisks2 nunca implementam a interface `Filesystem`
            // (só os `block_devices` de partição/disco não particionado a
            // implementam) — chamar `Filesystem.Mount` num caminho de
            // `drives/...` falha sempre. Quando `id` é um drive, monta cada
            // uma de suas partições ainda não montadas; quando já é uma
            // partição, monta-a diretamente.
            let toast = if let Some(drive) = snapshot.as_ref().and_then(|s| s.drive_by_id(&id)) {
                let mut mounted = 0usize;
                let mut last_err: Option<anyhow::Error> = None;
                for part in &drive.partitions {
                    if part.is_mounted() {
                        continue;
                    }
                    match mount(conn, &part.id.0).await {
                        Ok(()) => mounted += 1,
                        Err(e) => last_err = Some(e),
                    }
                }
                match (mounted, last_err) {
                    (0, Some(e)) => Toast::error(format!("Falha ao montar: {e}")),
                    (0, None) => Toast::info("nenhuma partição para montar"),
                    (n, _) => Toast::info(format!("{n} partição(ões) montada(s)")),
                }
            } else {
                match mount(conn, &id.0).await {
                    Ok(()) => Toast::info("Dispositivo montado"),
                    Err(e) => Toast::error(format!("Falha ao montar: {e}")),
                }
            };
            let _ = tx.send(AppEvent::Toast(toast));
        }
        Action::StorageUnmount(id) => {
            // Mesma correção do braço acima, para o sentido inverso.
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
            // `snapshot` está garantidamente `Some` aqui: `is_system_target`
            // acima já teria bloqueado (fail-closed) se fosse `None`.
            let Some(snap) = snapshot.as_ref() else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            // Resolve o alvo (que pode ser um `Drive` ou já um
            // `block_device`) para o caminho de objeto de bloco correto —
            // `Block.Format` só existe em caminhos `block_devices/...`,
            // nunca no caminho de objeto do `Drive`.
            let Some(block_path) = resolve_block_object_path(snap, &id) else {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "bloco de dispositivo não encontrado para formatação");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "dispositivo de bloco não encontrado para formatação",
                )));
                return;
            };
            // Desmonta todas as partições montadas do alvo antes de formatar
            // — o `Block.Format` falha (ou é destrutivo demais) sobre um
            // dispositivo com filesystems montados.
            for part_path in mounted_partition_paths(snap, &id) {
                let _ = unmount(conn, &part_path).await;
            }
            tracing::warn!(target: "hal9001::storage", device = %device_id, block = %block_path, fs = %fs_type, label = %label, "formatação solicitada");
            let toast = match format_block(conn, &block_path, &fs_type, &label).await {
                Ok(()) => Toast::info("Formatação concluída"),
                Err(e) if is_fat_fs_type(&fs_type) && is_missing_mkfs_error(&e) => {
                    // O host não tem `dosfstools` instalado — cai de volta
                    // para o formatador FAT32 100% Rust puro (`fatfs`), sem
                    // NUNCA expor ao usuário um erro pedindo para instalar
                    // pacotes externos no sistema operacional.
                    tracing::warn!(target: "hal9001::storage", device = %device_id, "mkfs.vfat ausente no host — usando formatador FAT32 Rust puro via Block.OpenDevice");
                    match open_device_fd(conn, &block_path).await {
                        Ok(file) => {
                            let label_owned = label.clone();
                            let result = tokio::task::spawn_blocking(move || {
                                format_fat32_on_file(file, &label_owned)
                            })
                            .await;
                            match result {
                                Ok(Ok(())) => {
                                    // Notifica o UDisks2/kernel para
                                    // rescanear o dispositivo — a
                                    // formatação foi feita diretamente no
                                    // descritor aberto via `OpenDevice`,
                                    // por fora do `Block.Format`.
                                    let _ = udisks_call(
                                        conn,
                                        &block_path,
                                        "org.freedesktop.UDisks2.Block",
                                        "Rescan",
                                    )
                                    .await;
                                    Toast::info("Formatação concluída (FAT32 Rust puro)")
                                }
                                Ok(Err(e)) => Toast::error(format!(
                                    "Falha ao formatar (FAT32 Rust puro): {e}"
                                )),
                                Err(e) => Toast::error(format!("Falha ao formatar: {e}")),
                            }
                        }
                        Err(e) => {
                            Toast::error(format!("Falha ao abrir dispositivo via OpenDevice: {e}"))
                        }
                    }
                }
                Err(e) if is_not_authorized_error(&e) => {
                    // Sem agente Polkit gráfico ativo na sessão (TTY pura):
                    // o `Block.Format`/`OpenDevice` do UDisks2 recusa a
                    // chamada com `NotAuthorized`. Cai para um helper
                    // `mkfs.*` executado via `sudo -S`/`sudo -n`, com a senha
                    // pedida pelo modal nativo da TUI quando necessário.
                    tracing::warn!(target: "hal9001::storage", device = %device_id, "Block.Format recusado (NotAuthorized) — usando fallback de mkfs via sudo");
                    match snap.dev_node_for_block_path(&block_path) {
                        Some(dev_node) => match mkfs_command(&fs_type, &label, &dev_node) {
                            Some((bin, args)) => {
                                match format_via_sudo(&dev_node, &bin, &args, sudo_tx, tx).await {
                                    Ok(()) => {
                                        let _ = udisks_call(
                                            conn,
                                            &block_path,
                                            "org.freedesktop.UDisks2.Block",
                                            "Rescan",
                                        )
                                        .await;
                                        Toast::info(format!("Formatação concluída ({bin}, sudo)"))
                                    }
                                    Err(msg) => {
                                        Toast::error(format!("Falha ao formatar via {bin}: {msg}"))
                                    }
                                }
                            }
                            None => Toast::error(format_error_message(&fs_type, &e)),
                        },
                        None => {
                            Toast::error("nó de dispositivo não encontrado para formatação elevada")
                        }
                    }
                }
                Err(e) => Toast::error(format_error_message(&fs_type, &e)),
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
        Action::StorageVentoyInstall { device_id } => {
            let id = DeviceId(device_id.clone());
            let Some(snap) = snapshot else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            if snap.is_system_target(&id) {
                tracing::warn!(target: "hal9001::storage", device = %device_id, "instalação do Ventoy em disco de sistema recusada");
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
            tracing::warn!(target: "hal9001::storage", device = %device_id, dev_node = %dev_node, "instalação do Ventoy solicitada");
            let txc = tx.clone();
            let sudo_txc = sudo_tx.clone();
            tokio::spawn(ventoy_task(device_id, dev_node, txc, sudo_txc));
        }
        Action::StorageVentoyListIsos { device_id } => {
            let id = DeviceId(device_id.clone());
            let Some(snap) = snapshot else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "árvore de discos indisponível",
                )));
                return;
            };
            let Some(drive) = snap.drive_by_id(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "dispositivo alvo não encontrado",
                )));
                return;
            };
            let Some(part) = ventoy_data_partition(drive) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "partição de dados do Ventoy não encontrada",
                )));
                return;
            };
            let part_path = part.id.0.clone();
            let existing_mount = part.mount_points.first().cloned();
            let conn2 = conn.clone();
            let txc = tx.clone();
            tokio::spawn(ventoy_list_isos_task(
                conn2,
                device_id,
                part_path,
                existing_mount,
                txc,
            ));
        }
        Action::StorageVentoyAddIso {
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
                tracing::warn!(target: "hal9001::storage", device = %device_id, "escrita de ISO em disco de sistema recusada (gerenciador Ventoy)");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                return;
            }
            let Some(drive) = snap.drive_by_id(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "dispositivo alvo não encontrado",
                )));
                return;
            };
            let Some(part) = ventoy_data_partition(drive) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "partição de dados do Ventoy não encontrada",
                )));
                return;
            };
            let part_path = part.id.0.clone();
            let existing_mount = part.mount_points.first().cloned();
            tracing::warn!(target: "hal9001::storage", device = %device_id, src = %src_path, "cópia de ISO para o Ventoy solicitada");
            let conn2 = conn.clone();
            let txc = tx.clone();
            tokio::spawn(ventoy_add_iso_task(
                conn2,
                device_id,
                part_path,
                existing_mount,
                src_path,
                txc,
            ));
        }
        Action::StorageVentoyRemoveIso {
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
                tracing::warn!(target: "hal9001::storage", device = %device_id, "remoção de ISO em disco de sistema recusada (gerenciador Ventoy)");
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "operação bloqueada: disco de sistema",
                )));
                return;
            }
            let Some(drive) = snap.drive_by_id(&id) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "dispositivo alvo não encontrado",
                )));
                return;
            };
            let Some(part) = ventoy_data_partition(drive) else {
                let _ = tx.send(AppEvent::Toast(Toast::error(
                    "partição de dados do Ventoy não encontrada",
                )));
                return;
            };
            let part_path = part.id.0.clone();
            let existing_mount = part.mount_points.first().cloned();
            tracing::warn!(target: "hal9001::storage", device = %device_id, file = %file_name, "remoção de ISO do Ventoy solicitada");
            let conn2 = conn.clone();
            let txc = tx.clone();
            tokio::spawn(ventoy_remove_iso_task(
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
