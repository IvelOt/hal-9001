//! Network Wi-Fi (NetworkManager D-Bus) — SSID ativo e força do sinal.
//!
//! Conforme seção 1.2 de `docs/backend_architecture.md`. Usa o daemon
//! `org.freedesktop.NetworkManager` para identificar a rede sem fio ativa
//! (SSID e força) e ligar/desligar o Wi-Fi globalmente (`WirelessEnabled`).

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use zbus::zvariant::{OwnedObjectPath, Value};
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
    /// `true` se a rede exige autenticação (WEP/WPA/WPA2/WPA3).
    pub is_secured: bool,
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

    /// Adiciona e ativa uma conexão a partir de um dicionário de configuração
    /// (`a{sa{sv}}`). Retorna `(connection_path, active_connection_path)`.
    fn add_and_activate_connection(
        &self,
        connection: HashMap<String, HashMap<String, Value<'_>>>,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> zbus::Result<(OwnedObjectPath, OwnedObjectPath)>;

    /// Ativa uma conexão já salva em `Settings` (reconexão sem nova senha).
    fn activate_connection(
        &self,
        connection: OwnedObjectPath,
        device: OwnedObjectPath,
        specific_object: OwnedObjectPath,
    ) -> zbus::Result<OwnedObjectPath>;

    /// Desativa uma conexão ativa (desconecta).
    fn deactivate_connection(&self, active_connection: OwnedObjectPath) -> zbus::Result<()>;

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

    /// Flags de capacidade do AP (bit 0x1 = privacidade/WEP).
    #[zbus(property)]
    fn flags(&self) -> zbus::Result<u32>;

    /// Flags de segurança WPA (0 quando a rede não usa WPA).
    #[zbus(property)]
    fn wpa_flags(&self) -> zbus::Result<u32>;

    /// Flags de segurança RSN/WPA2/WPA3 (0 quando a rede não usa RSN).
    #[zbus(property)]
    fn rsn_flags(&self) -> zbus::Result<u32>;
}

/// Proxy para `org.freedesktop.NetworkManager.Settings` (perfis salvos).
#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings",
    default_service = "org.freedesktop.NetworkManager",
    default_path = "/org/freedesktop/NetworkManager/Settings"
)]
trait Settings {
    /// Caminhos de objeto de todos os perfis de conexão salvos.
    fn list_connections(&self) -> zbus::Result<Vec<OwnedObjectPath>>;
}

/// Proxy para `org.freedesktop.NetworkManager.Settings.Connection` (um perfil salvo).
#[proxy(
    interface = "org.freedesktop.NetworkManager.Settings.Connection",
    default_service = "org.freedesktop.NetworkManager"
)]
trait SettingsConnection {
    /// Lê as configurações (`a{sa{sv}}`) do perfil salvo.
    fn get_settings(&self) -> zbus::Result<HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>>;

    /// Apaga permanentemente o perfil salvo.
    fn delete(&self) -> zbus::Result<()>;
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
                let flags = ap.flags().await.unwrap_or(0);
                let wpa_flags = ap.wpa_flags().await.unwrap_or(0);
                let rsn_flags = ap.rsn_flags().await.unwrap_or(0);
                // Bit 0x1 de `Flags` = NM_802_11_AP_FLAGS_PRIVACY (WEP); qualquer
                // WPA/RSN flag != 0 indica WPA/WPA2/WPA3.
                let is_secured = (flags & 0x1) != 0 || wpa_flags != 0 || rsn_flags != 0;
                access_points.push(AccessPointInfo {
                    ssid: decode_ssid(&ap.ssid().await.unwrap_or_default()),
                    strength: ap.strength().await.unwrap_or(0),
                    interface: interface.clone(),
                    object_path: ap_path.to_string(),
                    is_active,
                    is_secured,
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

    /// Conecta a um ponto de acesso Wi-Fi identificado por `ap_path`
    /// (via NetworkManager `AddAndActivateConnection`).
    ///
    /// Ação mutável — o resultado é refletido como Toast na TUI.
    pub async fn connect_access_point(&self, ap_path: &str) -> Result<()> {
        self.connect_access_point_with_password(ap_path, None).await
    }

    /// Conecta a um ponto de acesso Wi-Fi, opcionalmente enviando uma
    /// passphrase WPA-PSK. Quando já existe um perfil salvo para o SSID do
    /// AP, reativa esse perfil (`ActivateConnection`) em vez de criar um novo
    /// — dispensando reenviar a senha de redes já conhecidas.
    ///
    /// Ação mutável — o resultado é refletido como Toast na TUI.
    pub async fn connect_access_point_with_password(
        &self,
        ap_path: &str,
        passphrase: Option<&str>,
    ) -> Result<()> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;

        // Localiza o dispositivo sem fio que hospeda o ponto de acesso.
        let device_path = self.wireless_device_for_ap(ap_path).await?;
        let ssid = self
            .ssid_for_ap(ap_path)
            .await
            .with_context(|| format!("falha ao ler SSID de {ap_path}"))?;

        let ap: OwnedObjectPath = ap_path
            .try_into()
            .with_context(|| format!("caminho de AP inválido: {ap_path}"))?;
        let device: OwnedObjectPath = device_path
            .clone()
            .try_into()
            .with_context(|| format!("caminho de dispositivo inválido: {device_path}"))?;

        if let Some(saved_path) = self.saved_connection_for_ssid(&ssid).await? {
            nm.activate_connection(saved_path, device, ap)
                .await
                .context("falha ao reativar perfil salvo via NetworkManager")?;
            return Ok(());
        }

        let connection = build_wireless_connection(&ssid, passphrase);
        nm.add_and_activate_connection(connection, device, ap)
            .await
            .context("falha ao conectar ao ponto de acesso via NetworkManager")?;
        Ok(())
    }

    /// `true` quando já existe um perfil de conexão salvo para o SSID.
    pub async fn has_saved_connection(&self, ssid: &str) -> Result<bool> {
        Ok(self.saved_connection_for_ssid(ssid).await?.is_some())
    }

    /// Apaga o perfil de conexão salvo para o SSID informado, se existir.
    ///
    /// Ação mutável — o resultado é refletido como Toast na TUI.
    pub async fn forget_connection(&self, ssid: &str) -> Result<()> {
        let Some(conn_path) = self.saved_connection_for_ssid(ssid).await? else {
            anyhow::bail!("nenhum perfil salvo encontrado para {ssid}");
        };
        let conn = SettingsConnectionProxy::new(&self.connection, conn_path)
            .await
            .context("falha ao criar proxy Settings.Connection")?;
        conn.delete()
            .await
            .context("falha ao apagar perfil de conexão salvo")
    }

    /// Procura, entre os perfis salvos em `Settings`, um cujo SSID Wi-Fi
    /// corresponda ao informado.
    async fn saved_connection_for_ssid(&self, ssid: &str) -> Result<Option<OwnedObjectPath>> {
        let settings = SettingsProxy::new(&self.connection).await?;
        let connections = settings
            .list_connections()
            .await
            .context("falha ao listar perfis salvos do NetworkManager")?;

        for conn_path in connections {
            let conn = SettingsConnectionProxy::new(&self.connection, conn_path.clone())
                .await
                .with_context(|| format!("falha ao criar proxy Settings.Connection para {conn_path}"))?;
            let Ok(mut settings_map) = conn.get_settings().await else {
                continue;
            };
            let Some(mut wireless) = settings_map.remove("802-11-wireless") else {
                continue;
            };
            let Some(raw_ssid) = wireless.remove("ssid") else {
                continue;
            };
            let Ok(bytes) = Vec::<u8>::try_from(raw_ssid) else {
                continue;
            };
            if decode_ssid(&bytes) == ssid {
                return Ok(Some(conn_path));
            }
        }
        Ok(None)
    }

    /// Desconecta o Wi-Fi ativo desativando a conexão ativa atual.
    ///
    /// Ação mutável — o resultado é refletido como Toast na TUI.
    pub async fn disconnect_wireless(&self) -> Result<()> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;
        let active_connections = nm.active_connections().await?;

        for conn_path in active_connections {
            let active = ConnectionActiveProxy::new(&self.connection, conn_path.clone())
                .await
                .with_context(|| format!("falha ao criar proxy Connection.Active para {conn_path}"))?;
            let device_paths = active.devices().await.unwrap_or_default();
            for dev_path in device_paths {
                let device = DeviceProxy::new(&self.connection, dev_path)
                    .await
                    .with_context(|| "falha ao criar proxy Device".to_string())?;
                if NetworkDeviceType::from(device.device_type().await.unwrap_or(0))
                    == NetworkDeviceType::Wifi
                {
                    nm.deactivate_connection(conn_path).await?;
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Encontra o caminho do dispositivo sem fio que hospeda o AP informado.
    async fn wireless_device_for_ap(&self, ap_path: &str) -> Result<String> {
        let nm = NetworkManagerProxy::new(&self.connection).await?;
        let device_paths = nm
            .get_all_devices()
            .await
            .context("falha ao chamar NetworkManager.GetAllDevices")?;

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
            for candidate in wireless.get_all_access_points().await.unwrap_or_default() {
                if candidate.as_str() == ap_path {
                    return Ok(dev_path.to_string());
                }
            }
        }
        Err(anyhow::anyhow!("nenhum dispositivo sem fio com o AP {ap_path}"))
    }

    /// Lê o SSID (bytes decodificados) de um ponto de acesso.
    async fn ssid_for_ap(&self, ap_path: &str) -> Result<String> {
        let ap_path: OwnedObjectPath = ap_path
            .try_into()
            .with_context(|| format!("caminho de AP inválido: {ap_path}"))?;
        let ap = AccessPointProxy::new(&self.connection, ap_path)
            .await
            .context("falha ao criar proxy AccessPoint")?;
        Ok(decode_ssid(&ap.ssid().await.unwrap_or_default()))
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

/// Monta o dicionário de configuração D-Bus (`a{sa{sv}}`) para uma conexão
/// Wi-Fi, usada por `AddAndActivateConnection`. Quando `passphrase` é
/// informada, adiciona a seção `802-11-wireless-security` (WPA-PSK).
fn build_wireless_connection(
    ssid: &str,
    passphrase: Option<&str>,
) -> HashMap<String, HashMap<String, Value<'static>>> {
    let mut connection = HashMap::new();
    connection.insert("type".to_string(), Value::new("802-11-wireless"));

    let mut wireless = HashMap::new();
    wireless.insert(
        "ssid".to_string(),
        Value::new(ssid.bytes().collect::<Vec<_>>()),
    );
    wireless.insert("mode".to_string(), Value::new("infrastructure"));

    let mut ipv4 = HashMap::new();
    ipv4.insert("method".to_string(), Value::new("auto"));
    let mut ipv6 = HashMap::new();
    ipv6.insert("method".to_string(), Value::new("auto"));

    let mut root = HashMap::new();
    root.insert("connection".to_string(), connection);
    root.insert("802-11-wireless".to_string(), wireless);
    root.insert("ipv4".to_string(), ipv4);
    root.insert("ipv6".to_string(), ipv6);

    if let Some(passphrase) = passphrase.filter(|p| !p.is_empty()) {
        let mut security = HashMap::new();
        security.insert("key-mgmt".to_string(), Value::new("wpa-psk"));
        security.insert("psk".to_string(), Value::new(passphrase.to_string()));
        root.insert("802-11-wireless-security".to_string(), security);
    }
    root
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

    #[test]
    fn build_wireless_connection_without_passphrase_has_no_security_section() {
        let conn = build_wireless_connection("MinhaRede", None);
        assert!(!conn.contains_key("802-11-wireless-security"));
        assert!(conn.contains_key("802-11-wireless"));
    }

    #[test]
    fn build_wireless_connection_with_passphrase_adds_wpa_psk_section() {
        let conn = build_wireless_connection("MinhaRede", Some("segredo123"));
        let security = conn
            .get("802-11-wireless-security")
            .expect("seção de segurança ausente");
        assert!(security.contains_key("psk"));
        assert!(security.contains_key("key-mgmt"));
    }

    #[test]
    fn build_wireless_connection_ignores_empty_passphrase() {
        let conn = build_wireless_connection("MinhaRede", Some(""));
        assert!(!conn.contains_key("802-11-wireless-security"));
    }
}
