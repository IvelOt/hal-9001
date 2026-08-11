//! Montagem e desmontagem de mídias de armazenamento via daemon **UDisks2**
//! (D-Bus `org.freedesktop.UDisks2`) com autorização **Polkit** — sem `sudo`.
//!
//! Conforme seção 1.3 de `docs/backend_architecture.md`.

use std::collections::HashMap;

use anyhow::{Context, Result};
use serde::Serialize;
use zbus::zvariant::{OwnedObjectPath, Value};
use zbus::{proxy, Connection};

/// Representa um dispositivo de bloco exposto pelo UDisks2.
#[derive(Debug, Clone, Serialize)]
pub struct BlockDevice {
    pub object_path: String,
    /// Caminho do device node (ex.: `/dev/sdb1`).
    pub device: String,
    /// Rótulo da partição (ex.: `USB_STICK`), vazio se ausente.
    pub label: String,
    /// UUID da partição.
    pub uuid: String,
    /// Tamanho em bytes.
    pub size: u64,
    /// `true` se o dispositivo deve ser ignorado (mídia do sistema / interno).
    pub hint_ignore: bool,
    /// `true` se o dispositivo pertence ao sistema.
    pub hint_system: bool,
}

impl BlockDevice {}

/// Proxy para a interface `org.freedesktop.UDisks2.Manager`.
#[proxy(
    interface = "org.freedesktop.UDisks2.Manager",
    default_service = "org.freedesktop.UDisks2",
    default_path = "/org/freedesktop/UDisks2/Manager"
)]
trait UDisks2Manager {
    /// Retorna os caminhos de objeto de todos os dispositivos de bloco.
    fn get_block_devices(&self, options: HashMap<String, Value<'_>>) -> zbus::Result<Vec<OwnedObjectPath>>;
}

/// Proxy para a interface `org.freedesktop.UDisks2.Block`.
#[proxy(
    interface = "org.freedesktop.UDisks2.Block",
    default_service = "org.freedesktop.UDisks2"
)]
trait UDisks2Block {
    /// Device node principal (array de bytes).
    #[zbus(property)]
    fn device(&self) -> zbus::Result<Vec<u8>>;

    /// Rótulo da partição.
    #[zbus(property)]
    fn id_label(&self) -> zbus::Result<String>;

    /// UUID da partição.
    #[zbus(property)]
    fn id_uuid(&self) -> zbus::Result<String>;

    /// Tamanho em bytes.
    #[zbus(property)]
    fn size(&self) -> zbus::Result<u64>;

    /// `true` se o dispositivo deve ser ignorado.
    #[zbus(property)]
    fn hint_ignore(&self) -> zbus::Result<bool>;

    /// `true` se o dispositivo é do sistema.
    #[zbus(property)]
    fn hint_system(&self) -> zbus::Result<bool>;
}

/// Proxy para a interface `org.freedesktop.UDisks2.Filesystem`.
#[proxy(
    interface = "org.freedesktop.UDisks2.Filesystem",
    default_service = "org.freedesktop.UDisks2"
)]
trait UDisks2Filesystem {
    /// Monta o filesystem retornando o ponto de montagem (ex.: `/run/media/$USER/...`).
    fn mount(&self, options: HashMap<String, Value<'_>>) -> zbus::Result<String>;

    /// Desmonta o filesystem com segurança.
    fn unmount(&self, options: HashMap<String, Value<'_>>) -> zbus::Result<()>;

    /// Pontos de montagem atuais (`aay`).
    #[zbus(property)]
    fn mount_points(&self) -> zbus::Result<Vec<Vec<u8>>>;
}

/// Backend de armazenamento que encapsula a conexão D-Bus ao UDisks2.
pub struct Storage {
    connection: Connection,
}

impl Storage {
    /// Abre uma conexão com o barramento do sistema e retorna o backend de armazenamento.
    pub async fn new() -> Result<Self> {
        let connection = Connection::system()
            .await
            .context("falha ao conectar ao barramento D-Bus do sistema")?;
        Ok(Self { connection })
    }

    /// Lista todos os dispositivos de bloco removíveis (não-sistema, não-ignorados).
    pub async fn block_devices(&self) -> Result<Vec<BlockDevice>> {
        let manager = UDisks2ManagerProxy::new(&self.connection).await?;
        let paths = manager
            .get_block_devices(HashMap::new())
            .await
            .context("falha ao chamar UDisks2.GetBlockDevices")?;

        let mut devices = Vec::new();
        for path in paths {
            let block = UDisks2BlockProxy::new(&self.connection, path.clone())
                .await
                .with_context(|| format!("falha ao criar proxy Block para {path}"))?;

            let hint_ignore = block.hint_ignore().await.unwrap_or(false);
            let hint_system = block.hint_system().await.unwrap_or(false);
            if hint_ignore || hint_system {
                continue;
            }

            let device_bytes = block.device().await.unwrap_or_default();
            devices.push(BlockDevice {
                object_path: path.to_string(),
                device: String::from_utf8_lossy(&device_bytes).into_owned(),
                label: block.id_label().await.unwrap_or_default(),
                uuid: block.id_uuid().await.unwrap_or_default(),
                size: block.size().await.unwrap_or(0),
                hint_ignore,
                hint_system,
            });
        }

        Ok(devices)
    }

    /// Monta o filesystem de um dispositivo e retorna o ponto de montagem.
    ///
    /// O caminho do objeto deve corresponder a um dispositivo com interface
    /// `Filesystem` (ex.: `/org/freedesktop/UDisks2/block_devices/sdb1`).
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn mount(&self, object_path: &str) -> Result<String> {
        let object_path: OwnedObjectPath = object_path
            .try_into()
            .with_context(|| format!("caminho de objeto D-Bus inválido: {object_path}"))?;
        let fs = UDisks2FilesystemProxy::new(&self.connection, object_path)
            .await
            .context("falha ao criar proxy Filesystem para dispositivo")?;

        fs.mount(HashMap::new())
            .await
            .context("falha ao montar dispositivo via UDisks2 (Polkit)")
    }

    /// Desmonta o filesystem de um dispositivo com segurança.
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn unmount(&self, object_path: &str) -> Result<()> {
        let object_path: OwnedObjectPath = object_path
            .try_into()
            .with_context(|| format!("caminho de objeto D-Bus inválido: {object_path}"))?;
        let fs = UDisks2FilesystemProxy::new(&self.connection, object_path)
            .await
            .context("falha ao criar proxy Filesystem para dispositivo")?;

        fs.unmount(HashMap::new())
            .await
            .context("falha ao desmontar dispositivo via UDisks2 (Polkit)")
    }

    /// Retorna o ponto de montagem atual, se montado.
    pub async fn is_mounted(&self, object_path: &str) -> Result<bool> {
        let object_path: OwnedObjectPath = object_path
            .try_into()
            .with_context(|| format!("caminho de objeto D-Bus inválido: {object_path}"))?;
        let fs = UDisks2FilesystemProxy::new(&self.connection, object_path)
            .await
            .context("falha ao criar proxy Filesystem para dispositivo")?;

        Ok(!fs.mount_points().await.unwrap_or_default().is_empty())
    }
}
