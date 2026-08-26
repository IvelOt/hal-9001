use hal9001::app::App;
use hal9001::backend::bluetooth::{BluetoothDevice, BluetoothDeviceType, BluetoothSnapshot};
use hal9001::backend::network::{ActiveConnectionInfo, NetTelemetry, NetworkSnapshot};
use hal9001::backend::storage::{DriveInfo, StorageSnapshot};
use hal9001::backend::system::{Battery, BatteryStatus, SystemSnapshot};
use hal9001::config::Config;
use hal9001::events::AppEvent;

#[test]
fn test_battery_toasts() {
    let config = Config::default();
    let mut app = App::new(config);

    let mut snap = SystemSnapshot {
        host: "test".into(),
        user: "test".into(),
        shell: "test".into(),
        os: "test".into(),
        kernel: "test".into(),
        uptime_secs: 0,
        cpu_name: "test".into(),
        cpu_usage: 0.0,
        mem_used: 0,
        mem_total: 0,
        host_model: None,
        packages: None,
        brightness: None,
        kbd_backlight: None,
        volume: None,
        battery: Some(Battery {
            percent: 20.0,
            status: BatteryStatus::Discharging,
            power_watts: None,
            health: None,
            cycle_count: None,
            technology: None,
        }),
        disk_used: None,
        disk_total: None,
        power_profile: None,
        detail: hal9001::backend::system::DetailInfo::default(),
    };
    app.handle_event(AppEvent::System(Box::new(snap.clone())));
    assert!(app.toast.is_none());

    snap.battery.as_mut().unwrap().percent = 14.0;
    app.handle_event(AppEvent::System(Box::new(snap.clone())));
    assert!(app
        .toast
        .as_ref()
        .unwrap()
        .0
        .text
        .contains("Nível crítico: 14%"));

    app.toast = None;
    snap.battery.as_mut().unwrap().percent = 13.0;
    app.handle_event(AppEvent::System(Box::new(snap.clone())));
    assert!(app.toast.is_none());

    snap.battery.as_mut().unwrap().status = BatteryStatus::Charging;
    app.handle_event(AppEvent::System(Box::new(snap.clone())));
    assert!(app
        .toast
        .as_ref()
        .unwrap()
        .0
        .text
        .contains("Carregador conectado"));

    snap.battery.as_mut().unwrap().percent = 20.0;
    snap.battery.as_mut().unwrap().status = BatteryStatus::Discharging;
    app.handle_event(AppEvent::System(Box::new(snap.clone())));
    assert!(app.toast.as_ref().unwrap().0.text.contains("Em bateria"));
}

#[test]
fn test_storage_toasts() {
    let config = Config::default();
    let mut app = App::new(config);

    let mut snap = StorageSnapshot {
        udisks_available: true,
        drives: vec![],
    };
    app.handle_event(AppEvent::Storage(Box::new(snap.clone())));
    assert!(app.toast.is_none());

    snap.drives.push(DriveInfo {
        id: hal9001::events::DeviceId("test".into()),
        dev_node: "/dev/sdb".into(),
        block_path: None,
        model: "test".into(),
        vendor: "test".into(),
        size: 0,
        removable: true,
        ejectable: true,
        can_power_off: true,
        bus: hal9001::backend::storage::BusType::Usb,
        rotational: false,
        is_system: false,
        is_ventoy: false,
        partitions: vec![],
    });
    app.handle_event(AppEvent::Storage(Box::new(snap.clone())));
    assert!(app
        .toast
        .as_ref()
        .unwrap()
        .0
        .text
        .contains("Dispositivo conectado: /dev/sdb"));
}

#[test]
fn test_network_toasts() {
    let config = Config::default();
    let mut app = App::new(config);

    let mut snap = NetworkSnapshot {
        nm_available: true,
        networking_enabled: true,
        wireless_enabled: true,
        wireless_hw_enabled: true,
        wifi_device: None,
        access_points: vec![],
        active: None,
        telemetry: NetTelemetry::default(),
    };
    app.handle_event(AppEvent::Network(Box::new(snap.clone())));
    assert!(app.toast.is_none());

    snap.active = Some(ActiveConnectionInfo {
        id: hal9001::events::DeviceId("test".into()),
        ssid: "MyNet".into(),
        state: 0,
        connection_path: None,
    });
    snap.telemetry.ipv4 = Some("192.168.0.2".into());
    app.handle_event(AppEvent::Network(Box::new(snap.clone())));
    assert!(app
        .toast
        .as_ref()
        .unwrap()
        .0
        .text
        .contains("Conectado em 'MyNet' (IP: 192.168.0.2)"));

    snap.active = None;
    snap.telemetry.ipv4 = None;
    app.handle_event(AppEvent::Network(Box::new(snap.clone())));
    assert!(app.toast.as_ref().unwrap().0.text.contains("Desconectado"));
}

#[test]
fn test_bluetooth_toasts() {
    let config = Config::default();
    let mut app = App::new(config);

    let mut dev = BluetoothDevice {
        id: hal9001::events::DeviceId("test".into()),
        address: "AA".into(),
        name: "MyHeadset".into(),
        alias: "MyHeadset".into(),
        device_type: BluetoothDeviceType::Audio,
        connected: false,
        paired: true,
        trusted: true,
        blocked: false,
        rssi: None,
        battery_percentage: None,
        icon: None,
    };

    let mut snap = BluetoothSnapshot {
        bluez_available: true,
        adapter: None,
        devices: vec![dev.clone()],
    };
    app.handle_event(AppEvent::Bluetooth(Box::new(snap.clone())));
    assert!(app.toast.is_none());

    dev.connected = true;
    dev.battery_percentage = Some(80);
    snap.devices = vec![dev.clone()];
    app.handle_event(AppEvent::Bluetooth(Box::new(snap.clone())));
    assert!(app
        .toast
        .as_ref()
        .unwrap()
        .0
        .text
        .contains("Conectado: MyHeadset (Bateria: 80%)"));

    dev.connected = false;
    snap.devices = vec![dev.clone()];
    app.handle_event(AppEvent::Bluetooth(Box::new(snap.clone())));
    assert!(app
        .toast
        .as_ref()
        .unwrap()
        .0
        .text
        .contains("Desconectado: MyHeadset"));
}
