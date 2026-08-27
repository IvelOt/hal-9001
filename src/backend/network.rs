use std::collections::HashMap;
use std::time::{Duration, Instant};

use tokio::sync::broadcast;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

use crate::events::{Action, AppEvent, DeviceId, EventTx, Toast};
use crate::i18n::SharedLang;

const NM_SERVICE: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const NM_IFACE: &str = "org.freedesktop.NetworkManager";
const NM_DEVICE_IFACE: &str = "org.freedesktop.NetworkManager.Device";
const NM_WIRELESS_IFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const NM_AP_IFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";
const NM_SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const NM_SETTINGS_IFACE: &str = "org.freedesktop.NetworkManager.Settings";
const NM_CONN_IFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const NM_ACTIVE_CONN_IFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const NM_IP4_IFACE: &str = "org.freedesktop.NetworkManager.IP4Config";

const NM_DEVICE_TYPE_WIFI: u32 = 2;

const NM_802_11_AP_FLAGS_PRIVACY: u32 = 0x1;
const NM_SEC_KEY_MGMT_PSK: u32 = 0x100;
const NM_SEC_KEY_MGMT_8021X: u32 = 0x200;
const NM_SEC_KEY_MGMT_SAE: u32 = 0x400;
const NM_SEC_KEY_MGMT_OWE: u32 = 0x800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    Open,
    Wep,
    Wpa,
    Wpa2,
    Wpa3,
    Wpa2Enterprise,
    Owe,
}

impl Security {
    pub fn label(self) -> &'static str {
        match self {
            Security::Open => "Aberta",
            Security::Wep => "WEP",
            Security::Wpa => "WPA",
            Security::Wpa2 => "WPA2-PSK",
            Security::Wpa3 => "WPA3-SAE",
            Security::Wpa2Enterprise => "WPA2-Ent",
            Security::Owe => "OWE",
        }
    }

    pub fn needs_password(self) -> bool {
        matches!(
            self,
            Security::Wep | Security::Wpa | Security::Wpa2 | Security::Wpa3
        )
    }

    pub fn key_mgmt(self) -> Option<&'static str> {
        match self {
            Security::Wpa | Security::Wpa2 => Some("wpa-psk"),
            Security::Wpa3 => Some("sae"),
            Security::Wep => Some("none"),
            Security::Owe => Some("owe"),
            _ => None,
        }
    }
}

pub fn derive_security(flags: u32, wpa: u32, rsn: u32) -> Security {
    let has = |bits: u32, m: u32| bits & m != 0;
    if has(rsn, NM_SEC_KEY_MGMT_SAE) {
        return Security::Wpa3;
    }
    if has(rsn, NM_SEC_KEY_MGMT_OWE) {
        return Security::Owe;
    }
    if has(rsn, NM_SEC_KEY_MGMT_8021X) || has(wpa, NM_SEC_KEY_MGMT_8021X) {
        return Security::Wpa2Enterprise;
    }
    if has(rsn, NM_SEC_KEY_MGMT_PSK) {
        return Security::Wpa2;
    }
    if wpa != 0 {
        return Security::Wpa;
    }
    if has(flags, NM_802_11_AP_FLAGS_PRIVACY) {
        return Security::Wep;
    }
    Security::Open
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WifiBand {
    Ghz24,
    Ghz5,
    Ghz6,
    Unknown,
}

impl WifiBand {
    pub fn label(self) -> &'static str {
        match self {
            WifiBand::Ghz24 => "2.4 GHz",
            WifiBand::Ghz5 => "5 GHz",
            WifiBand::Ghz6 => "6 GHz",
            WifiBand::Unknown => "Wi-Fi",
        }
    }
}

pub fn band_of(freq_mhz: u32) -> WifiBand {
    match freq_mhz {
        2400..=2500 => WifiBand::Ghz24,
        4900..=5900 => WifiBand::Ghz5,
        5925..=7125 => WifiBand::Ghz6,
        _ => WifiBand::Unknown,
    }
}

#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub nm_available: bool,
    pub networking_enabled: bool,
    pub wireless_enabled: bool,
    pub wireless_hw_enabled: bool,
    pub wifi_device: Option<WifiDevice>,
    pub access_points: Vec<AccessPoint>,
    pub active: Option<ActiveConnectionInfo>,
    pub telemetry: NetTelemetry,
}

#[derive(Debug, Clone)]
pub struct WifiDevice {
    pub id: DeviceId,
    pub iface: String,
    pub hw_address: String,
    pub state: u32,
    pub bitrate_kbps: u32,
    pub active_ap: Option<DeviceId>,
    pub last_scan_ms: i64,
}

#[derive(Debug, Clone)]
pub struct AccessPoint {
    pub id: DeviceId,
    pub ssid: String,
    pub ssid_raw: Vec<u8>,
    pub bssid: String,
    pub strength: u8,
    pub frequency: u32,
    pub band: WifiBand,
    pub max_bitrate_kbps: u32,
    pub security: Security,
    pub is_active: bool,
    pub is_saved: bool,
    pub saved_conn_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ActiveConnectionInfo {
    pub id: DeviceId,
    pub ssid: String,
    pub state: u32,
    pub connection_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct NetTelemetry {
    pub ipv4: Option<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub rx_rate_kbps: f64,
    pub tx_rate_kbps: f64,
    pub total_rx_bytes: u64,
    pub total_tx_bytes: u64,
}

struct ThroughputTracker {
    last_rx: u64,
    last_tx: u64,
    last_time: Instant,
}

impl ThroughputTracker {
    fn new() -> Self {
        Self {
            last_rx: 0,
            last_tx: 0,
            last_time: Instant::now(),
        }
    }

    fn update(&mut self, rx: u64, tx: u64) -> (f64, f64) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_time).as_secs_f64();
        if elapsed <= 0.001 {
            return (0.0, 0.0);
        }

        let rx_diff = rx.saturating_sub(self.last_rx);
        let tx_diff = tx.saturating_sub(self.last_tx);

        self.last_rx = rx;
        self.last_tx = tx;
        self.last_time = now;

        let rx_kbps = (rx_diff as f64 / 1024.0) / elapsed;
        let tx_kbps = (tx_diff as f64 / 1024.0) / elapsed;

        (rx_kbps, tx_kbps)
    }
}

pub async fn run(
    polling_ms: u64,
    lang: SharedLang,
    tx: EventTx,
    mut actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    let mut interval = tokio::time::interval(Duration::from_millis(polling_ms.max(2000)));
    let mut throughput_interval = tokio::time::interval(Duration::from_millis(1000));
    let mut throughput = ThroughputTracker::new();

    let mut last_snapshot: Option<NetworkSnapshot> = None;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                match collect_snapshot(&mut throughput).await {
                    Ok(snap) => {
                        let _ = tx.send(AppEvent::Network(Box::new(snap.clone())));
                        last_snapshot = Some(snap);
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::ServiceDegraded {
                            name: "network",
                            reason: format!("NetworkManager D-Bus: {e}"),
                        });
                    }
                }
            }
            _ = throughput_interval.tick() => {
                if let Some(mut snap) = last_snapshot.clone() {
                    if let Some(dev) = &snap.wifi_device {
                        let (rx, tx_b) = read_sysfs_stats(&dev.iface);
                        let (rx_rate, tx_rate) = throughput.update(rx, tx_b);
                        snap.telemetry.rx_rate_kbps = rx_rate;
                        snap.telemetry.tx_rate_kbps = tx_rate;
                        snap.telemetry.total_rx_bytes = rx;
                        snap.telemetry.total_tx_bytes = tx_b;
                        let _ = tx.send(AppEvent::Network(Box::new(snap.clone())));
                        last_snapshot = Some(snap);
                    }
                }
            }
            Ok(action) = actions.recv() => {
                let m = lang.messages();
                match action {
                    Action::NetworkRescan => {
                        let _ = tx.send(AppEvent::NetworkScanning(true));
                        if let Err(e) = trigger_rescan().await {
                            let _ = tx.send(AppEvent::Toast(Toast::error(format!("{}: {e}", m.net_err_rescan_failed))));
                        } else {
                            let _ = tx.send(AppEvent::Toast(Toast::info(m.net_toast_scan_started)));
                        }
                        tokio::time::sleep(Duration::from_millis(1500)).await;
                        if let Ok(snap) = collect_snapshot(&mut throughput).await {
                            let _ = tx.send(AppEvent::Network(Box::new(snap.clone())));
                            last_snapshot = Some(snap);
                        }
                        let _ = tx.send(AppEvent::NetworkScanning(false));
                    }
                    Action::NetworkToggleRadio => {
                        match toggle_wireless_radio().await {
                            Ok(new_state) => {
                                let label = if new_state { m.net_toast_radio_on_state } else { m.net_toast_radio_off_state };
                                let _ = tx.send(AppEvent::Toast(Toast::info(format!("{} {label}", m.net_toast_radio_prefix))));
                                if let Ok(snap) = collect_snapshot(&mut throughput).await {
                                    let _ = tx.send(AppEvent::Network(Box::new(snap.clone())));
                                    last_snapshot = Some(snap);
                                }
                            }
                            Err(e) => {
                                let _ = tx.send(AppEvent::Toast(Toast::error(format!("{}: {e}", m.net_err_radio_toggle))));
                            }
                        }
                    }
                    Action::NetworkConnect { ap_id, ssid, password } => {
                        let _ = tx.send(AppEvent::Toast(Toast::info(format!("{} {ssid}...", m.net_toast_connecting))));
                        match connect_network(&ap_id, &ssid, password.as_deref(), lang.get()).await {
                            Ok(_) => {
                                let _ = tx.send(AppEvent::Toast(Toast::info(format!("{} {ssid}", m.net_toast_connect_started))));
                            }
                            Err(e) => {
                                let _ = tx.send(AppEvent::Toast(Toast::error(format!("{}: {e}", m.net_err_connect))));
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(1000)).await;
                        if let Ok(snap) = collect_snapshot(&mut throughput).await {
                            let _ = tx.send(AppEvent::Network(Box::new(snap.clone())));
                            last_snapshot = Some(snap);
                        }
                    }
                    Action::NetworkDisconnect(dev_id) => {
                        if let Err(e) = disconnect_network(&dev_id.0).await {
                            let _ = tx.send(AppEvent::Toast(Toast::error(format!("{}: {e}", m.net_err_disconnect))));
                        } else {
                            let _ = tx.send(AppEvent::Toast(Toast::info(m.net_toast_disconnected)));
                        }
                        if let Ok(snap) = collect_snapshot(&mut throughput).await {
                            let _ = tx.send(AppEvent::Network(Box::new(snap.clone())));
                            last_snapshot = Some(snap);
                        }
                    }
                    Action::NetworkForget(conn_path) => {
                        if let Err(e) = forget_network(&conn_path).await {
                            let _ = tx.send(AppEvent::Toast(Toast::error(format!("{}: {e}", m.net_err_forget))));
                        } else {
                            let _ = tx.send(AppEvent::Toast(Toast::info(m.net_toast_forgotten)));
                        }
                        if let Ok(snap) = collect_snapshot(&mut throughput).await {
                            let _ = tx.send(AppEvent::Network(Box::new(snap.clone())));
                            last_snapshot = Some(snap);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn collect_snapshot(throughput: &mut ThroughputTracker) -> anyhow::Result<NetworkSnapshot> {
    let conn = Connection::system().await?;

    let nm_proxy = zbus::Proxy::new(&conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;

    let wireless_enabled: bool = nm_proxy
        .get_property("WirelessEnabled")
        .await
        .unwrap_or(false);
    let wireless_hw_enabled: bool = nm_proxy
        .get_property("WirelessHardwareEnabled")
        .await
        .unwrap_or(false);
    let networking_enabled: bool = nm_proxy
        .get_property("NetworkingEnabled")
        .await
        .unwrap_or(false);

    let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;

    let mut wifi_device: Option<WifiDevice> = None;
    let mut raw_aps: Vec<AccessPoint> = Vec::new();
    let mut active_conn_info: Option<ActiveConnectionInfo> = None;
    let mut telemetry = NetTelemetry::default();

    let saved_profiles = list_saved_profiles(&conn).await.unwrap_or_default();

    for dev_path in devices {
        let dev_proxy =
            zbus::Proxy::new(&conn, NM_SERVICE, dev_path.as_str(), NM_DEVICE_IFACE).await?;
        let dev_type: u32 = dev_proxy.get_property("DeviceType").await.unwrap_or(0);

        if dev_type == NM_DEVICE_TYPE_WIFI {
            let iface: String = dev_proxy
                .get_property("Interface")
                .await
                .unwrap_or_default();
            let hw_address: String = dev_proxy
                .get_property("HwAddress")
                .await
                .unwrap_or_default();
            let state: u32 = dev_proxy.get_property("State").await.unwrap_or(0);
            let active_conn_path: OwnedObjectPath = dev_proxy
                .get_property("ActiveConnection")
                .await
                .unwrap_or_else(|_| OwnedObjectPath::try_from("/").unwrap());
            let ip4_config_path: OwnedObjectPath = dev_proxy
                .get_property("Ip4Config")
                .await
                .unwrap_or_else(|_| OwnedObjectPath::try_from("/").unwrap());

            let wifi_proxy =
                zbus::Proxy::new(&conn, NM_SERVICE, dev_path.as_str(), NM_WIRELESS_IFACE).await?;
            let bitrate: u32 = wifi_proxy.get_property("Bitrate").await.unwrap_or(0);
            let active_ap: OwnedObjectPath = wifi_proxy
                .get_property("ActiveAccessPoint")
                .await
                .unwrap_or_else(|_| OwnedObjectPath::try_from("/").unwrap());
            let last_scan: i64 = wifi_proxy.get_property("LastScan").await.unwrap_or(-1);

            let active_ap_id = if active_ap.as_str() != "/" {
                Some(DeviceId(active_ap.as_str().to_string()))
            } else {
                None
            };

            wifi_device = Some(WifiDevice {
                id: DeviceId(dev_path.as_str().to_string()),
                iface: iface.clone(),
                hw_address,
                state,
                bitrate_kbps: bitrate,
                active_ap: active_ap_id.clone(),
                last_scan_ms: last_scan,
            });

            let ap_paths: Vec<OwnedObjectPath> = wifi_proxy
                .call("GetAllAccessPoints", &())
                .await
                .unwrap_or_default();

            for ap_path in ap_paths {
                let ap_proxy =
                    zbus::Proxy::new(&conn, NM_SERVICE, ap_path.as_str(), NM_AP_IFACE).await?;
                let ssid_raw: Vec<u8> = ap_proxy.get_property("Ssid").await.unwrap_or_default();
                let ssid = String::from_utf8_lossy(&ssid_raw).to_string();
                if ssid.is_empty() {
                    continue;
                }

                let strength: u8 = ap_proxy.get_property("Strength").await.unwrap_or(0);
                let frequency: u32 = ap_proxy.get_property("Frequency").await.unwrap_or(0);
                let bssid: String = ap_proxy.get_property("HwAddress").await.unwrap_or_default();
                let max_bitrate: u32 = ap_proxy.get_property("MaxBitrate").await.unwrap_or(0);
                let flags: u32 = ap_proxy.get_property("Flags").await.unwrap_or(0);
                let wpa_flags: u32 = ap_proxy.get_property("WpaFlags").await.unwrap_or(0);
                let rsn_flags: u32 = ap_proxy.get_property("RsnFlags").await.unwrap_or(0);

                let security = derive_security(flags, wpa_flags, rsn_flags);
                let band = band_of(frequency);

                let is_active = active_ap.as_str() == ap_path.as_str();
                let saved_conn_path = saved_profiles.get(&ssid).cloned();
                let is_saved = saved_conn_path.is_some();

                raw_aps.push(AccessPoint {
                    id: DeviceId(ap_path.as_str().to_string()),
                    ssid,
                    ssid_raw,
                    bssid,
                    strength,
                    frequency,
                    band,
                    max_bitrate_kbps: max_bitrate,
                    security,
                    is_active,
                    is_saved,
                    saved_conn_path,
                });
            }

            if ip4_config_path.as_str() != "/" {
                let ip_proxy =
                    zbus::Proxy::new(&conn, NM_SERVICE, ip4_config_path.as_str(), NM_IP4_IFACE)
                        .await?;
                let gateway: String = ip_proxy.get_property("Gateway").await.unwrap_or_default();
                if !gateway.is_empty() {
                    telemetry.gateway = Some(gateway);
                }

                if let Ok(addr_data) = ip_proxy
                    .get_property::<Vec<HashMap<String, OwnedValue>>>("AddressData")
                    .await
                {
                    if let Some(first) = addr_data.first() {
                        if let Some(addr_val) = first.get("address") {
                            if let Ok(ip_str) = <&str>::try_from(addr_val) {
                                telemetry.ipv4 = Some(ip_str.to_string());
                            }
                        }
                    }
                }
            }

            let (rx, tx_b) = read_sysfs_stats(&iface);
            let (rx_rate, tx_rate) = throughput.update(rx, tx_b);
            telemetry.rx_rate_kbps = rx_rate;
            telemetry.tx_rate_kbps = tx_rate;
            telemetry.total_rx_bytes = rx;
            telemetry.total_tx_bytes = tx_b;

            if active_conn_path.as_str() != "/" {
                let act_proxy = zbus::Proxy::new(
                    &conn,
                    NM_SERVICE,
                    active_conn_path.as_str(),
                    NM_ACTIVE_CONN_IFACE,
                )
                .await?;
                let id_str: String = act_proxy.get_property("Id").await.unwrap_or_default();
                let act_state: u32 = act_proxy.get_property("State").await.unwrap_or(0);
                let conn_path: OwnedObjectPath = act_proxy
                    .get_property("Connection")
                    .await
                    .unwrap_or_else(|_| OwnedObjectPath::try_from("/").unwrap());

                active_conn_info = Some(ActiveConnectionInfo {
                    id: DeviceId(active_conn_path.as_str().to_string()),
                    ssid: id_str,
                    state: act_state,
                    connection_path: if conn_path.as_str() != "/" {
                        Some(conn_path.as_str().to_string())
                    } else {
                        None
                    },
                });
            }

            break;
        }
    }

    let mut deduplicated: HashMap<String, AccessPoint> = HashMap::new();
    for ap in raw_aps {
        match deduplicated.get(&ap.ssid) {
            Some(existing) => {
                if ap.is_active || (!existing.is_active && ap.strength > existing.strength) {
                    deduplicated.insert(ap.ssid.clone(), ap);
                }
            }
            None => {
                deduplicated.insert(ap.ssid.clone(), ap);
            }
        }
    }

    let mut access_points: Vec<AccessPoint> = deduplicated.into_values().collect();
    access_points.sort_by(|a, b| {
        if a.is_active != b.is_active {
            b.is_active.cmp(&a.is_active)
        } else if a.is_saved != b.is_saved {
            b.is_saved.cmp(&a.is_saved)
        } else {
            b.strength
                .cmp(&a.strength)
                .then_with(|| a.ssid.cmp(&b.ssid))
        }
    });

    Ok(NetworkSnapshot {
        nm_available: true,
        networking_enabled,
        wireless_enabled,
        wireless_hw_enabled,
        wifi_device,
        access_points,
        active: active_conn_info,
        telemetry,
    })
}

async fn list_saved_profiles(conn: &Connection) -> anyhow::Result<HashMap<String, String>> {
    let settings_proxy =
        zbus::Proxy::new(conn, NM_SERVICE, NM_SETTINGS_PATH, NM_SETTINGS_IFACE).await?;
    let connections: Vec<OwnedObjectPath> = settings_proxy.call("ListConnections", &()).await?;

    let mut map = HashMap::new();
    for conn_path in connections {
        let Ok(conn_proxy) =
            zbus::Proxy::new(conn, NM_SERVICE, conn_path.as_str(), NM_CONN_IFACE).await
        else {
            continue;
        };
        let res: Result<HashMap<String, HashMap<String, OwnedValue>>, _> =
            conn_proxy.call("GetSettings", &()).await;
        if let Ok(settings) = res {
            if let Some(wireless_map) = settings.get("802-11-wireless") {
                if let Some(ssid_val) = wireless_map.get("ssid") {
                    if let Ok(ssid_bytes) = Vec::<u8>::try_from(ssid_val.clone()) {
                        let ssid = String::from_utf8_lossy(&ssid_bytes).to_string();
                        map.insert(ssid, conn_path.as_str().to_string());
                    }
                }
            } else if let Some(conn_sec) = settings.get("connection") {
                if let Some(id_val) = conn_sec.get("id") {
                    if let Ok(id_str) = String::try_from(id_val.clone()) {
                        map.insert(id_str, conn_path.as_str().to_string());
                    }
                }
            }
        }
    }

    Ok(map)
}

fn read_sysfs_stats(iface: &str) -> (u64, u64) {
    let rx_path = format!("/sys/class/net/{iface}/statistics/rx_bytes");
    let tx_path = format!("/sys/class/net/{iface}/statistics/tx_bytes");

    let rx = std::fs::read_to_string(rx_path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let tx = std::fs::read_to_string(tx_path)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);

    (rx, tx)
}

async fn trigger_rescan() -> anyhow::Result<()> {
    let conn = Connection::system().await?;
    let nm_proxy = zbus::Proxy::new(&conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;
    let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;

    for dev_path in devices {
        let dev_proxy =
            zbus::Proxy::new(&conn, NM_SERVICE, dev_path.as_str(), NM_DEVICE_IFACE).await?;
        let dev_type: u32 = dev_proxy.get_property("DeviceType").await.unwrap_or(0);
        if dev_type == NM_DEVICE_TYPE_WIFI {
            let wifi_proxy =
                zbus::Proxy::new(&conn, NM_SERVICE, dev_path.as_str(), NM_WIRELESS_IFACE).await?;
            let empty_opts: HashMap<&str, Value> = HashMap::new();
            let _: () = wifi_proxy.call("RequestScan", &(empty_opts,)).await?;
            break;
        }
    }
    Ok(())
}

async fn toggle_wireless_radio() -> anyhow::Result<bool> {
    let conn = Connection::system().await?;
    let nm_proxy = zbus::Proxy::new(&conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;
    let current: bool = nm_proxy.get_property("WirelessEnabled").await?;
    let next = !current;
    nm_proxy.set_property("WirelessEnabled", next).await?;
    Ok(next)
}

async fn disconnect_network(device_path: &str) -> anyhow::Result<()> {
    let conn = Connection::system().await?;
    let dev_proxy = zbus::Proxy::new(&conn, NM_SERVICE, device_path, NM_DEVICE_IFACE).await?;
    let _: () = dev_proxy.call("Disconnect", &()).await?;
    Ok(())
}

async fn forget_network(conn_path: &str) -> anyhow::Result<()> {
    let conn = Connection::system().await?;
    let conn_proxy = zbus::Proxy::new(&conn, NM_SERVICE, conn_path, NM_CONN_IFACE).await?;
    let _: () = conn_proxy.call("Delete", &()).await?;
    Ok(())
}

async fn connect_network(
    ap_path: &str,
    ssid: &str,
    password: Option<&str>,
    lang: crate::i18n::Language,
) -> anyhow::Result<()> {
    let conn = Connection::system().await?;
    let nm_proxy = zbus::Proxy::new(&conn, NM_SERVICE, NM_PATH, NM_IFACE).await?;
    let ap_obj = ObjectPath::try_from(ap_path)?;

    let saved = list_saved_profiles(&conn).await.unwrap_or_default();
    if let Some(saved_path) = saved.get(ssid) {
        let conn_obj = ObjectPath::try_from(saved_path.as_str())?;

        let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;
        for dev_path in devices {
            let dev_proxy =
                zbus::Proxy::new(&conn, NM_SERVICE, dev_path.as_str(), NM_DEVICE_IFACE).await?;
            let dev_type: u32 = dev_proxy.get_property("DeviceType").await.unwrap_or(0);
            if dev_type == NM_DEVICE_TYPE_WIFI {
                let _: (OwnedObjectPath, OwnedObjectPath) = nm_proxy
                    .call("ActivateConnection", &(conn_obj, dev_path, ap_obj))
                    .await?;
                return Ok(());
            }
        }
    }

    let mut connection_settings: HashMap<&str, HashMap<&str, Value>> = HashMap::new();

    let mut conn_sec: HashMap<&str, Value> = HashMap::new();
    conn_sec.insert("id", Value::from(ssid));
    conn_sec.insert("type", Value::from("802-11-wireless"));
    connection_settings.insert("connection", conn_sec);

    let mut wireless_sec: HashMap<&str, Value> = HashMap::new();
    wireless_sec.insert("ssid", Value::from(ssid.as_bytes()));
    wireless_sec.insert("mode", Value::from("infrastructure"));
    connection_settings.insert("802-11-wireless", wireless_sec);

    if let Some(pwd) = password {
        let mut sec_sec: HashMap<&str, Value> = HashMap::new();
        sec_sec.insert("key-mgmt", Value::from("wpa-psk"));
        sec_sec.insert("psk", Value::from(pwd));
        connection_settings.insert("802-11-wireless-security", sec_sec);
    }

    let devices: Vec<OwnedObjectPath> = nm_proxy.call("GetDevices", &()).await?;
    for dev_path in devices {
        let dev_proxy =
            zbus::Proxy::new(&conn, NM_SERVICE, dev_path.as_str(), NM_DEVICE_IFACE).await?;
        let dev_type: u32 = dev_proxy.get_property("DeviceType").await.unwrap_or(0);
        if dev_type == NM_DEVICE_TYPE_WIFI {
            let _: (OwnedObjectPath, OwnedObjectPath) = nm_proxy
                .call(
                    "AddAndActivateConnection",
                    &(connection_settings, dev_path, ap_obj),
                )
                .await?;
            return Ok(());
        }
    }

    anyhow::bail!(lang.messages().net_err_device_not_found)
}
