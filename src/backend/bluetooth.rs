use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

use crate::events::{Action, AppEvent, DeviceId, EventTx};

pub const BLUEZ_SERVICE: &str = "org.bluez";
pub const BLUEZ_ADAPTER_IFACE: &str = "org.bluez.Adapter1";
pub const BLUEZ_DEVICE_IFACE: &str = "org.bluez.Device1";
pub const BLUEZ_BATTERY_IFACE: &str = "org.bluez.Battery1";
pub const DBUS_OBJ_MANAGER_IFACE: &str = "org.freedesktop.DBus.ObjectManager";
pub const DBUS_PROPERTIES_IFACE: &str = "org.freedesktop.DBus.Properties";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BluetoothDeviceType {
    Audio,

    Gamepad,

    Keyboard,

    Mouse,

    Phone,

    Computer,

    Wearable,

    Other,
}

impl BluetoothDeviceType {
    pub fn ascii_label(&self) -> &'static str {
        match self {
            Self::Audio => "[FONE]",
            Self::Gamepad => "[PAD ]",
            Self::Keyboard => "[TECL]",
            Self::Mouse => "[MOUS]",
            Self::Phone => "[CEL ]",
            Self::Computer => "[PC  ]",
            Self::Wearable => "[RELO]",
            Self::Other => "[DEV ]",
        }
    }

    pub fn nerd_glyph(&self) -> &'static str {
        match self {
            Self::Audio => "󰋋",
            Self::Gamepad => "󰊴",
            Self::Keyboard => "󰌌",
            Self::Mouse => "󰍽",
            Self::Phone => "󰄜",
            Self::Computer => "󰌢",
            Self::Wearable => "󰂯",
            Self::Other => "󰂯",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothDevice {
    pub id: DeviceId,

    pub address: String,

    pub name: String,

    pub alias: String,

    pub device_type: BluetoothDeviceType,

    pub connected: bool,

    pub paired: bool,

    pub trusted: bool,

    pub blocked: bool,

    pub rssi: Option<i16>,

    pub battery_percentage: Option<u8>,

    pub icon: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothAdapter {
    pub id: DeviceId,

    pub address: String,

    pub name: String,

    pub powered: bool,

    pub discovering: bool,

    pub discoverable: bool,

    pub pairable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BluetoothSnapshot {
    pub bluez_available: bool,

    pub adapter: Option<BluetoothAdapter>,

    pub devices: Vec<BluetoothDevice>,
}

pub fn derive_device_type(
    icon: Option<&str>,
    class_of_device: Option<u32>,
    appearance: Option<u16>,
    uuids: &[String],
) -> BluetoothDeviceType {
    if let Some(ic) = icon {
        let ic_lower = ic.to_lowercase();
        if ic_lower.contains("headset")
            || ic_lower.contains("audio")
            || ic_lower.contains("headphones")
            || ic_lower.contains("speaker")
        {
            return BluetoothDeviceType::Audio;
        }
        if ic_lower.contains("gamepad")
            || ic_lower.contains("joystick")
            || ic_lower.contains("gaming")
        {
            return BluetoothDeviceType::Gamepad;
        }
        if ic_lower.contains("keyboard") {
            return BluetoothDeviceType::Keyboard;
        }
        if ic_lower.contains("mouse") || ic_lower.contains("input-mouse") {
            return BluetoothDeviceType::Mouse;
        }
        if ic_lower.contains("phone") {
            return BluetoothDeviceType::Phone;
        }
        if ic_lower.contains("computer") {
            return BluetoothDeviceType::Computer;
        }
    }

    if let Some(cod) = class_of_device {
        let major = (cod >> 8) & 0x1F;
        let minor = (cod >> 2) & 0x3F;

        match major {
            0x01 => return BluetoothDeviceType::Computer,
            0x02 => return BluetoothDeviceType::Phone,
            0x04 => return BluetoothDeviceType::Audio,
            0x05 => {
                if (minor & 0x10) != 0 {
                    return BluetoothDeviceType::Keyboard;
                } else if (minor & 0x20) != 0 {
                    return BluetoothDeviceType::Mouse;
                } else if (minor & 0x01) != 0 || (minor & 0x02) != 0 {
                    return BluetoothDeviceType::Gamepad;
                }
            }
            0x07 => return BluetoothDeviceType::Wearable,
            _ => {}
        }
    }

    if let Some(app) = appearance {
        match app >> 6 {
            15 => return BluetoothDeviceType::Gamepad,
            14 => return BluetoothDeviceType::Keyboard,
            16 => return BluetoothDeviceType::Mouse,
            10 => return BluetoothDeviceType::Audio,
            3 => return BluetoothDeviceType::Wearable,
            _ => {}
        }
    }

    for uuid in uuids {
        let u_lower = uuid.to_lowercase();

        if u_lower.contains("110b")
            || u_lower.contains("110a")
            || u_lower.contains("111e")
            || u_lower.contains("1108")
        {
            return BluetoothDeviceType::Audio;
        }

        if u_lower.contains("1124") || u_lower.contains("1812") {
            return BluetoothDeviceType::Keyboard;
        }
    }

    BluetoothDeviceType::Other
}

pub async fn run(poll_interval_ms: u64, tx: EventTx, mut action_rx: broadcast::Receiver<Action>) {
    let mut interval = tokio::time::interval(Duration::from_millis(poll_interval_ms.max(500)));
    let mut scan_start_time: Option<Instant> = None;

    loop {
        tokio::select! {
            _ = interval.tick() => {

                if let Some(start) = scan_start_time {
                    if start.elapsed() > Duration::from_secs(30) {
                        scan_start_time = None;
                        let _ = tx.send(AppEvent::BluetoothScanning(false));
                        if let Ok(conn) = Connection::system().await {
                            let _ = stop_discovery(&conn).await;
                        }
                    }
                }

                if let Ok(snap) = fetch_bluetooth_snapshot().await {
                    let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                }
            }

            Ok(action) = action_rx.recv() => {
                match action {
                    Action::BluetoothRescan => {
                        if let Ok(conn) = Connection::system().await {
                            if scan_start_time.is_some() {

                                let _ = stop_discovery(&conn).await;
                                scan_start_time = None;
                                let _ = tx.send(AppEvent::BluetoothScanning(false));
                            } else {

                                if start_discovery(&conn).await.is_ok() {
                                    scan_start_time = Some(Instant::now());
                                    let _ = tx.send(AppEvent::BluetoothScanning(true));
                                }
                            }
                            if let Ok(snap) = fetch_bluetooth_snapshot().await {
                                let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                            }
                        }
                    }

                    Action::BluetoothToggleRadio => {
                        if let Ok(conn) = Connection::system().await {
                            let _ = toggle_radio(&conn).await;
                            if let Ok(snap) = fetch_bluetooth_snapshot().await {
                                let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                            }
                        }
                    }

                    Action::BluetoothConnect(dev_id) => {
                        if let Ok(conn) = Connection::system().await {
                            let _ = connect_device(&conn, &dev_id.0).await;
                            if let Ok(snap) = fetch_bluetooth_snapshot().await {
                                let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                            }
                        }
                    }

                    Action::BluetoothDisconnect(dev_id) => {
                        if let Ok(conn) = Connection::system().await {
                            let _ = disconnect_device(&conn, &dev_id.0).await;
                            if let Ok(snap) = fetch_bluetooth_snapshot().await {
                                let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                            }
                        }
                    }

                    Action::BluetoothPair(dev_id) => {
                        if let Ok(conn) = Connection::system().await {
                            let _ = pair_device(&conn, &dev_id.0).await;
                            if let Ok(snap) = fetch_bluetooth_snapshot().await {
                                let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                            }
                        }
                    }

                    Action::BluetoothForget(dev_id) => {
                        if let Ok(conn) = Connection::system().await {
                            let _ = forget_device(&conn, &dev_id.0).await;
                            if let Ok(snap) = fetch_bluetooth_snapshot().await {
                                let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                            }
                        }
                    }

                    Action::BluetoothToggleBlock(dev_id) => {
                        if let Ok(conn) = Connection::system().await {
                            let _ = toggle_block_device(&conn, &dev_id.0).await;
                            if let Ok(snap) = fetch_bluetooth_snapshot().await {
                                let _ = tx.send(AppEvent::Bluetooth(Box::new(snap)));
                            }
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}

pub async fn fetch_bluetooth_snapshot() -> anyhow::Result<BluetoothSnapshot> {
    let Ok(conn) = Connection::system().await else {
        return Ok(BluetoothSnapshot {
            bluez_available: false,
            adapter: None,
            devices: Vec::new(),
        });
    };

    let obj_manager = zbus::Proxy::new(&conn, BLUEZ_SERVICE, "/", DBUS_OBJ_MANAGER_IFACE).await?;

    type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
    let res: Result<ManagedObjects, _> = obj_manager.call("GetManagedObjects", &()).await;

    let Ok(objects) = res else {
        return Ok(BluetoothSnapshot {
            bluez_available: false,
            adapter: None,
            devices: Vec::new(),
        });
    };

    let mut adapter: Option<BluetoothAdapter> = None;
    let mut devices_map: HashMap<String, BluetoothDevice> = HashMap::new();
    let mut batteries_map: HashMap<String, u8> = HashMap::new();

    for (path, ifaces) in &objects {
        let path_str = path.as_str();

        if let Some(adapter_props) = ifaces.get(BLUEZ_ADAPTER_IFACE) {
            let address = prop_string(adapter_props, "Address").unwrap_or_default();
            let name = prop_string(adapter_props, "Name")
                .or_else(|| prop_string(adapter_props, "Alias"))
                .unwrap_or_else(|| "Bluetooth Adapter".to_string());
            let powered = prop_bool(adapter_props, "Powered").unwrap_or(false);
            let discovering = prop_bool(adapter_props, "Discovering").unwrap_or(false);
            let discoverable = prop_bool(adapter_props, "Discoverable").unwrap_or(false);
            let pairable = prop_bool(adapter_props, "Pairable").unwrap_or(false);

            if adapter.is_none() {
                adapter = Some(BluetoothAdapter {
                    id: DeviceId(path_str.to_string()),
                    address,
                    name,
                    powered,
                    discovering,
                    discoverable,
                    pairable,
                });
            }
        }

        if let Some(bat_props) = ifaces.get(BLUEZ_BATTERY_IFACE) {
            if let Some(pct) = prop_u8(bat_props, "Percentage") {
                batteries_map.insert(path_str.to_string(), pct);
            }
        }

        if let Some(dev_props) = ifaces.get(BLUEZ_DEVICE_IFACE) {
            let address = prop_string(dev_props, "Address").unwrap_or_default();
            let alias = prop_string(dev_props, "Alias").unwrap_or_else(|| address.clone());
            let name = prop_string(dev_props, "Name").unwrap_or_else(|| alias.clone());
            let connected = prop_bool(dev_props, "Connected").unwrap_or(false);
            let paired = prop_bool(dev_props, "Paired").unwrap_or(false);
            let trusted = prop_bool(dev_props, "Trusted").unwrap_or(false);
            let blocked = prop_bool(dev_props, "Blocked").unwrap_or(false);
            let rssi = prop_i16(dev_props, "RSSI");
            let icon = prop_string(dev_props, "Icon");
            let class_of_device = prop_u32(dev_props, "Class");
            let appearance = prop_u16(dev_props, "Appearance");
            let uuids = prop_string_vec(dev_props, "UUIDs");

            let device_type =
                derive_device_type(icon.as_deref(), class_of_device, appearance, &uuids);

            devices_map.insert(
                path_str.to_string(),
                BluetoothDevice {
                    id: DeviceId(path_str.to_string()),
                    address,
                    name,
                    alias,
                    device_type,
                    connected,
                    paired,
                    trusted,
                    blocked,
                    rssi,
                    battery_percentage: None,
                    icon,
                },
            );
        }
    }

    for (path_str, percentage) in batteries_map {
        if let Some(dev) = devices_map.get_mut(&path_str) {
            dev.battery_percentage = Some(percentage);
        }
    }

    let mut devices: Vec<BluetoothDevice> = devices_map.into_values().collect();

    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| b.paired.cmp(&a.paired))
            .then_with(|| b.rssi.unwrap_or(i16::MIN).cmp(&a.rssi.unwrap_or(i16::MIN)))
            .then_with(|| a.alias.cmp(&b.alias))
    });

    Ok(BluetoothSnapshot {
        bluez_available: true,
        adapter,
        devices,
    })
}

pub async fn start_discovery(conn: &Connection) -> anyhow::Result<()> {
    let adapter_proxy =
        zbus::Proxy::new(conn, BLUEZ_SERVICE, "/org/bluez/hci0", BLUEZ_ADAPTER_IFACE).await?;
    let _: () = adapter_proxy.call("StartDiscovery", &()).await?;
    Ok(())
}

pub async fn stop_discovery(conn: &Connection) -> anyhow::Result<()> {
    let adapter_proxy =
        zbus::Proxy::new(conn, BLUEZ_SERVICE, "/org/bluez/hci0", BLUEZ_ADAPTER_IFACE).await?;
    let _: () = adapter_proxy.call("StopDiscovery", &()).await?;
    Ok(())
}

pub async fn toggle_radio(conn: &Connection) -> anyhow::Result<()> {
    let prop_proxy = zbus::Proxy::new(
        conn,
        BLUEZ_SERVICE,
        "/org/bluez/hci0",
        DBUS_PROPERTIES_IFACE,
    )
    .await?;
    let current_powered_val: OwnedValue = prop_proxy
        .call("Get", &(BLUEZ_ADAPTER_IFACE, "Powered"))
        .await?;
    let current_powered = bool::try_from(current_powered_val).unwrap_or(false);
    let new_val = Value::from(!current_powered);
    let _: () = prop_proxy
        .call("Set", &(BLUEZ_ADAPTER_IFACE, "Powered", new_val))
        .await?;
    Ok(())
}

pub async fn connect_device(conn: &Connection, dev_path: &str) -> anyhow::Result<()> {
    let dev_proxy = zbus::Proxy::new(conn, BLUEZ_SERVICE, dev_path, BLUEZ_DEVICE_IFACE).await?;
    let _: () = dev_proxy.call("Connect", &()).await?;
    Ok(())
}

pub async fn disconnect_device(conn: &Connection, dev_path: &str) -> anyhow::Result<()> {
    let dev_proxy = zbus::Proxy::new(conn, BLUEZ_SERVICE, dev_path, BLUEZ_DEVICE_IFACE).await?;
    let _: () = dev_proxy.call("Disconnect", &()).await?;
    Ok(())
}

pub async fn pair_device(conn: &Connection, dev_path: &str) -> anyhow::Result<()> {
    let dev_proxy = zbus::Proxy::new(conn, BLUEZ_SERVICE, dev_path, BLUEZ_DEVICE_IFACE).await?;
    let _: () = dev_proxy.call("Pair", &()).await?;

    let prop_proxy = zbus::Proxy::new(conn, BLUEZ_SERVICE, dev_path, DBUS_PROPERTIES_IFACE).await?;
    let _: () = prop_proxy
        .call("Set", &(BLUEZ_DEVICE_IFACE, "Trusted", Value::from(true)))
        .await?;
    Ok(())
}

pub async fn forget_device(conn: &Connection, dev_path: &str) -> anyhow::Result<()> {
    let adapter_proxy =
        zbus::Proxy::new(conn, BLUEZ_SERVICE, "/org/bluez/hci0", BLUEZ_ADAPTER_IFACE).await?;
    let dev_obj = ObjectPath::try_from(dev_path).context("Invalid device path")?;
    let _: () = adapter_proxy.call("RemoveDevice", &(dev_obj,)).await?;
    Ok(())
}

pub async fn toggle_block_device(conn: &Connection, dev_path: &str) -> anyhow::Result<()> {
    let prop_proxy = zbus::Proxy::new(conn, BLUEZ_SERVICE, dev_path, DBUS_PROPERTIES_IFACE).await?;
    let current_blocked_val: OwnedValue = prop_proxy
        .call("Get", &(BLUEZ_DEVICE_IFACE, "Blocked"))
        .await?;
    let current_blocked = bool::try_from(current_blocked_val).unwrap_or(false);
    let new_val = Value::from(!current_blocked);
    let _: () = prop_proxy
        .call("Set", &(BLUEZ_DEVICE_IFACE, "Blocked", new_val))
        .await?;
    Ok(())
}

fn prop_string(props: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
}

fn prop_bool(props: &HashMap<String, OwnedValue>, key: &str) -> Option<bool> {
    props.get(key).and_then(|v| bool::try_from(v.clone()).ok())
}

fn prop_u8(props: &HashMap<String, OwnedValue>, key: &str) -> Option<u8> {
    props.get(key).and_then(|v| u8::try_from(v.clone()).ok())
}

fn prop_u16(props: &HashMap<String, OwnedValue>, key: &str) -> Option<u16> {
    props.get(key).and_then(|v| u16::try_from(v.clone()).ok())
}

fn prop_i16(props: &HashMap<String, OwnedValue>, key: &str) -> Option<i16> {
    props.get(key).and_then(|v| i16::try_from(v.clone()).ok())
}

fn prop_u32(props: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| u32::try_from(v.clone()).ok())
}

fn prop_string_vec(props: &HashMap<String, OwnedValue>, key: &str) -> Vec<String> {
    props
        .get(key)
        .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
        .unwrap_or_default()
}
