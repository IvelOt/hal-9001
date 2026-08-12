//! Network Wi-Fi (NetworkManager D-Bus) — SSID ativo e força do sinal.
//!
//! Conforme seção 1.2 de `docs/backend_architecture.md`. Usa o daemon
//! `org.freedesktop.NetworkManager` para identificar a rede sem fio ativa
//! (SSID e força) e ligar/desligar o Wi-Fi globalmente (`WirelessEnabled`).

use anyhow::{Context, Result};
use serde::Serialize;
use zbus::zvariant::OwnedObjectPath;
use zbus::{proxy, Connection};

/// Tipos de dispositivos de rede reportados pela propriedade `DeviceType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum NetworkDeviceType {
    Unknown = 0,
    Ethernet = 1,
    Wifi = 2,
    Bluetooth = 4,
    Modem = 7,
    Infiniband = 8,
    Bond = 9,
    Vlan = 10,
    Bridge = 12,
    Generic = 13,
    Tunnel = 15,
    Tun = 16,
    MacVlan = 17,
    Macsec = 21,
    Vxlan = 18,
    Dummy = 22,
    WifiP2p = 27,
    Vrf = 28,
}

impl From<u32> for NetworkDeviceType {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Ethernet,
            2 => Self::Wifi,
            4 => Self::Bluetooth,
            7 => Self::Modem,
            8 => Self::Infiniband,
            9 => Self::Bond,
            10 => Self::Vlan,
            12 => Self::Bridge,
            13 => Self::Generic,
            15 => Self::Tunnel,
            16 => Self::Tun,
            17 => Self::MacVlan,
            18 => Self::Vxlan,
            21 => Self::Macsec,
            22 => Self::Dummy,
            27 => Self::WifiP2p,
            28 => Self::Vrf,
            _ => Self::Unknown,
        }
    }
}

/// Informações da rede sem fio atualmente ativa.
#[derive(Debug, Clone, Serialize)]
pub struct WifiInfo {
    /// SSID da rede ativa (bytes decodificados como UTF-8).
    pub ssid: String,
    /// Força do sinal em percentual (0 a 100).
    pub strength: u8,
    /// Caminho de objeto D-Bus do dispositivo sem fio.
    pub device_path: String,
    /// Caminho de objeto D-Bus do ponto de acesso ativo.
    pub access_point_path: String,
}

/// Um ponto de acesso sem fio visível pelo dispositivo.
#[derive(Debug, Clone, Serialize)]
pub struct AccessPointInfo {
    /// SSID da rede (bytes decodificados como UTF-8).
    pub ssid: String,
    /// Força do sinal em percentual (0 a 100).
    pub strength: u8,
    /// Nome da interface de rede associada (ex.: `wlan0`).
    pub interface: String,
    /// Caminho de objeto D-Bus do ponto de acesso.
    pub object_path: String,
    /// `true` se este é o ponto de acesso atualmente ativo.
    pub is_active: bool,
}

/// Proxy para a interface principal `org.freedesktop.NetworkManager`.
#[proxy(
    interface = "org.freedesktop.NetworkManager",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager"
)]
trait NetworkManager {
    /// Caminhos de objeto das conexões ativas.
    #[zbus(property)]
    fn active_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// Retorna os caminhos de objeto de todos os dispositivos de rede.
    fn get_all_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// `true` se o Wi-Fi está habilitado no momento.
    #[zbus(property)]
    fn wireless_enabled(&self) -> zbus::Result<bool>;

    /// `true` se o hardware sem fio está presente e habilitado.
    #[zbus(property)]
    fn wireless_hardware_enabled(&self) -> zbus::Result<bool>;

    /// Habilita/desabilita o Wi-Fi de forma global.
    #[zbus(property)]
    fn set_wireless_enabled(&self, value: bool) -> zbus::Result<()>;
}

/// Proxy para `org.freedesktop.NetworkManager.Device` (dispositivo de rede).
#[proxy(
    interface = "org.freedesktop.NetworkManager.Device",
    default_service = "org.freedesktop.NetworkManager"
)]
trait Device {
    /// Tipo do dispositivo (`u32`, ver [`NetworkDeviceType`]).
    #[zbus(property)]
    fn device_type(&self) -> zbus::Result<u32>;

    /// Estado operacional atual (`u32`).
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// Nome da interface de rede (ex.: `wlan0`).
    #[zbus(property)]
    fn interface(&self) -> zbus::Result<String>;
}

/// Proxy para `org.freedesktop.NetworkManager.Device.Wireless`.
#[proxy(
    interface = "org.freedesktop.NetworkManager.Device.Wireless",
    default_service = "org.freedesktop.NetworkManager"
)]
trait WirelessDevice {
    /// Ponto de acesso atualmente ativo (caminho `/` quando desconectado).
    #[zbus(property)]
    fn active_access_point(&self) -> zbus::Result<OwnedObjectPath>;

    /// Retorna os caminhos de objeto de todos os pontos de acesso visíveis.
    fn get_all_access_points(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

/// Proxy para `org.freedesktop.NetworkManager.AccessPoint`.
#[proxy(
    interface = "org.freedesktop.NetworkManager.AccessPoint",
    default_service = "org.freedesktop.NetworkManager"
)]
trait AccessPoint {
    /// SSID da rede (`ay`), a ser decodificado como UTF-8.
    #[zbus(property)]
    fn ssid(&self) -> zbus::Result<Vec<u8>>;

    /// Força do sinal em percentual (0 a 100).
    #[zbus(property)]
    fn strength(&self) -> zbus::Result<u8>;
}

/// Proxy para `org.freedesktop.NetworkManager.Connection.Active`.
#[proxy(
    interface = "org.freedesktop.NetworkManager.Connection.Active",
    default_service = "org.freedesktop.NetworkManager"
)]
trait ConnectionActive {
    /// Dispositivos de rede associados à conexão ativa.
    #[zbus(property)]
    fn devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

/// Backend de rede que encapsula a conexão D-Bus ao NetworkManager.
pub struct Network {
    connection: Connection,
}

impl Network {
    /// Abre uma conexão com o barramento do sistema e retorna o backend de rede.
    pub async fn new() -> Result<Self> {
        let connection = Connection::system()
            .await
            .context("falha ao conectar ao barramento D-Bus do sistema")?;
        Ok(Self { connection })
    }

    /// Retorna as informações da rede sem fio ativa, se houver alguma.
    pub async fn active_wifi(&self) -> Result<Option<WifiInfo>> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;
        let active_connections = nm
            .active_connections()
            .await
            .context("falha ao ler ActiveConnections do NetworkManager")?;

        for conn_path in active_connections {
            let active = ConnectionActiveProxy::new(&self.connection, conn_path.clone())
                .await
                .with_context(|| format!("falha ao criar proxy Connection.Active para {conn_path}"))?;

            let device_paths = active.devices().await.unwrap_or_default();
            for dev_path in device_paths {
                let device = DeviceProxy::new(&self.connection, dev_path.clone())
                    .await
                    .with_context(|| format!("falha ao criar proxy Device para {dev_path}"))?;

                if NetworkDeviceType::from(device.device_type().await.unwrap_or(0))
                    != NetworkDeviceType::Wifi
                {
                    continue;
                }

                let wireless = WirelessDeviceProxy::new(&self.connection, dev_path.clone())
                    .await
                    .with_context(|| format!("falha ao criar proxy Wireless para {dev_path}"))?;

                let ap_path = wireless.active_access_point().await.unwrap_or_default();
                if ap_path.as_str() == "/" || ap_path.as_str().is_empty() {
                    continue;
                }

                let ap = AccessPointProxy::new(&self.connection, ap_path.clone())
                    .await
                    .with_context(|| format!("falha ao criar proxy AccessPoint para {ap_path}"))?;

                return Ok(Some(WifiInfo {
                    ssid: decode_ssid(&ap.ssid().await.unwrap_or_default()),
                    strength: ap.strength().await.unwrap_or(0),
                    device_path: dev_path.to_string(),
                    access_point_path: ap_path.to_string(),
                }));
            }
        }

        Ok(None)
    }

    /// Retorna `true` se o Wi-Fi está habilitado globalmente.
    pub async fn wireless_enabled(&self) -> Result<bool> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;
        nm.wireless_enabled()
            .await
            .context("falha ao ler WirelessEnabled do NetworkManager")
    }

    /// Lista todos os pontos de acesso visíveis em todos os dispositivos sem fio,
    /// com SSID e força do sinal. Marca o ponto de acesso ativo (`is_active`).
    pub async fn access_points(&self) -> Result<Vec<AccessPointInfo>> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;
        let device_paths = nm
            .get_all_devices()
            .await
            .context("falha ao chamar NetworkManager.GetAllDevices")?;

        let active = self.active_wifi().await.ok().flatten();

        let mut access_points = Vec::new();
        for dev_path in device_paths {
            let device = DeviceProxy::new(&self.connection, dev_path.clone())
                .await
                .with_context(|| format!("falha ao criar proxy Device para {dev_path}"))?;

            if NetworkDeviceType::from(device.device_type().await.unwrap_or(0))
                != NetworkDeviceType::Wifi
            {
                continue;
            }

            let interface = device.interface().await.unwrap_or_default();
            let wireless = WirelessDeviceProxy::new(&self.connection, dev_path.clone())
                .await
                .with_context(|| format!("falha ao criar proxy Wireless para {dev_path}"))?;

            for ap_path in wireless.get_all_access_points().await.unwrap_or_default() {
                let ap = AccessPointProxy::new(&self.connection, ap_path.clone())
                    .await
                    .with_context(|| format!("falha ao criar proxy AccessPoint para {ap_path}"))?;

                let is_active = active
                    .as_ref()
                    .map(|a| a.access_point_path == ap_path.as_str())
                    .unwrap_or(false);
                access_points.push(AccessPointInfo {
                    ssid: decode_ssid(&ap.ssid().await.unwrap_or_default()),
                    strength: ap.strength().await.unwrap_or(0),
                    interface: interface.clone(),
                    object_path: ap_path.to_string(),
                    is_active,
                });
            }
        }

        // Ordena por força do sinal (decrescente) para destacar as melhores redes.
        access_points.sort_by(|a, b| b.strength.cmp(&a.strength));
        Ok(access_points)
    }

    /// Retorna `true` se o hardware sem fio está presente e habilitado.
    #[allow(dead_code)]
    pub async fn wireless_hardware_enabled(&self) -> Result<bool> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;
        nm.wireless_hardware_enabled()
            .await
            .context("falha ao ler WirelessHardwareEnabled do NetworkManager")
    }

    /// Habilita ou desabilita o Wi-Fi de forma global (`WirelessEnabled`).
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn set_wireless_enabled(&self, enabled: bool) -> Result<()> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;
        nm.set_wireless_enabled(enabled)
            .await
            .context("falha ao alterar WirelessEnabled do NetworkManager")
    }
}

/// Decodifica o SSID (array de bytes) como UTF-8, tolerante a bytes inválidos
/// e a bytes nulos de preenchimento.
fn decode_ssid(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\0')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ssid_without_trailing_nulls() {
        assert_eq!(decode_ssid(b"Minha-Rede\0\0"), "Minha-Rede");
        assert_eq!(decode_ssid(b"Rede"), "Rede");
        assert_eq!(decode_ssid(b""), "");
    }

    #[test]
    fn decodes_ssid_with_invalid_bytes_lossy() {
        assert_eq!(decode_ssid(&[0x62, 0x61, 0xFF, 0x64]), "ba\u{FFFD}d");
    }

    #[test]
    fn maps_known_device_types() {
        assert_eq!(NetworkDeviceType::from(2), NetworkDeviceType::Wifi);
        assert_eq!(NetworkDeviceType::from(1), NetworkDeviceType::Ethernet);
        assert_eq!(NetworkDeviceType::from(999), NetworkDeviceType::Unknown);
    }
}
