//! Testes unitários e de integração do Módulo 3 (Bluetooth).

use hal9001::app::{App, Tab};
use hal9001::backend::bluetooth::{
    derive_device_type, BluetoothAdapter, BluetoothDevice, BluetoothDeviceType, BluetoothSnapshot,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent, DeviceId};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::broadcast;

#[test]
fn test_derive_device_type_icon_and_cod() {
    // 1. Icon explícito
    assert_eq!(
        derive_device_type(Some("audio-headset"), None, None, &[]),
        BluetoothDeviceType::Audio
    );
    assert_eq!(
        derive_device_type(Some("input-gaming"), None, None, &[]),
        BluetoothDeviceType::Gamepad
    );
    assert_eq!(
        derive_device_type(Some("input-keyboard"), None, None, &[]),
        BluetoothDeviceType::Keyboard
    );
    assert_eq!(
        derive_device_type(Some("input-mouse"), None, None, &[]),
        BluetoothDeviceType::Mouse
    );
    assert_eq!(
        derive_device_type(Some("phone"), None, None, &[]),
        BluetoothDeviceType::Phone
    );
    assert_eq!(
        derive_device_type(Some("computer"), None, None, &[]),
        BluetoothDeviceType::Computer
    );

    // 2. Class of Device (CoD)
    // Major 0x04 = Audio (ex: 0x240404)
    assert_eq!(
        derive_device_type(None, Some(0x240404), None, &[]),
        BluetoothDeviceType::Audio
    );
    // Major 0x05, Minor Keyboard (0x10)
    assert_eq!(
        derive_device_type(None, Some(0x002540), None, &[]),
        BluetoothDeviceType::Keyboard
    );
    // Major 0x05, Minor Gamepad (0x02)
    assert_eq!(
        derive_device_type(None, Some(0x002508), None, &[]),
        BluetoothDeviceType::Gamepad
    );
}

#[test]
fn test_derive_device_type_ble_appearance_and_uuids() {
    // BLE Appearance 960 (0x03C0 >> 6 = 15 -> Gamepad)
    assert_eq!(
        derive_device_type(None, None, Some(960), &[]),
        BluetoothDeviceType::Gamepad
    );
    // BLE Appearance 832 (0x0340 >> 6 = 13.. or Keyboard = 961)
    assert_eq!(
        derive_device_type(None, None, Some(961), &[]),
        BluetoothDeviceType::Gamepad
    );

    // UUIDs A2DP Audio Sink
    let uuids = vec!["0000110b-0000-1000-8000-00805f9b34fb".to_string()];
    assert_eq!(
        derive_device_type(None, None, None, &uuids),
        BluetoothDeviceType::Audio
    );

    // UUIDs HID Service
    let hid_uuids = vec!["00001124-0000-1000-8000-00805f9b34fb".to_string()];
    assert_eq!(
        derive_device_type(None, None, None, &hid_uuids),
        BluetoothDeviceType::Keyboard
    );
}

#[test]
fn test_bluetooth_snapshot_navigation_and_actions() {
    let mut app = App::new(Config::default());
    let (action_tx, mut action_rx) = broadcast::channel(16);

    let dev1 = BluetoothDevice {
        id: DeviceId("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_11".to_string()),
        address: "AA:BB:CC:DD:EE:11".to_string(),
        name: "Sony WH-1000XM4".to_string(),
        alias: "Sony WH-1000XM4".to_string(),
        device_type: BluetoothDeviceType::Audio,
        connected: true,
        paired: true,
        trusted: true,
        blocked: false,
        rssi: Some(-50),
        battery_percentage: Some(90),
        icon: Some("audio-headset".to_string()),
    };

    let dev2 = BluetoothDevice {
        id: DeviceId("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_22".to_string()),
        address: "AA:BB:CC:DD:EE:22".to_string(),
        name: "Xbox Wireless Controller".to_string(),
        alias: "Xbox Controller".to_string(),
        device_type: BluetoothDeviceType::Gamepad,
        connected: false,
        paired: true,
        trusted: true,
        blocked: false,
        rssi: Some(-65),
        battery_percentage: Some(75),
        icon: Some("input-gaming".to_string()),
    };

    let adapter = BluetoothAdapter {
        id: DeviceId("/org/bluez/hci0".to_string()),
        address: "00:11:22:33:44:55".to_string(),
        name: "Host Adapter".to_string(),
        powered: true,
        discovering: false,
        discoverable: false,
        pairable: true,
    };

    let snap = BluetoothSnapshot {
        bluez_available: true,
        adapter: Some(adapter),
        devices: vec![dev1, dev2],
    };

    // 1. Recebe snapshot
    let follow_ups = app.handle_event(AppEvent::Bluetooth(Box::new(snap)));
    assert!(follow_ups.is_empty());
    assert!(app.bluetooth.is_some());
    assert_eq!(app.bluetooth.as_ref().unwrap().devices.len(), 2);

    // 2. Muda para Tab::Bluetooth e navega
    app.active = Tab::Bluetooth;
    assert_eq!(app.bluetooth_selected, 0);

    app.dispatch(Action::Down, &action_tx);
    assert_eq!(app.bluetooth_selected, 1);

    app.dispatch(Action::Up, &action_tx);
    assert_eq!(app.bluetooth_selected, 0);

    // 3. Enter no dev1 (conectado) -> envia Disconnect
    app.dispatch(Action::Enter, &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::BluetoothDisconnect(DeviceId("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_11".to_string()))
    );

    // 4. Enter no dev2 (desconectado) -> envia Connect
    app.bluetooth_selected = 1;
    app.dispatch(Action::Enter, &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::BluetoothConnect(DeviceId("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_22".to_string()))
    );

    // 5. Atalho de rescan [r]
    app.dispatch(Action::BluetoothRescan, &action_tx);
    assert_eq!(action_rx.try_recv().unwrap(), Action::BluetoothRescan);

    // 6. Atalho de toggle rádio [t]
    app.dispatch(Action::BluetoothToggleRadio, &action_tx);
    assert_eq!(action_rx.try_recv().unwrap(), Action::BluetoothToggleRadio);

    // 7. Atalho de parear [p]
    app.dispatch(Action::BluetoothPair(DeviceId(String::new())), &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::BluetoothPair(DeviceId("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_22".to_string()))
    );
}

#[test]
fn test_bluetooth_headless_ui_render() {
    let mut app = App::new(Config::default());
    app.phase = hal9001::app::Phase::Running;
    app.active = Tab::Bluetooth;

    let dev1 = BluetoothDevice {
        id: DeviceId("/org/bluez/hci0/dev_11_22_33_44_55_66".to_string()),
        address: "11:22:33:44:55:66".to_string(),
        name: "Sony WH-1000XM5".to_string(),
        alias: "Sony WH-1000XM5".to_string(),
        device_type: BluetoothDeviceType::Audio,
        connected: true,
        paired: true,
        trusted: true,
        blocked: false,
        rssi: Some(-48),
        battery_percentage: Some(85),
        icon: Some("audio-headset".to_string()),
    };

    let dev2 = BluetoothDevice {
        id: DeviceId("/org/bluez/hci0/dev_22_33_44_55_66_77".to_string()),
        address: "22:33:44:55:66:77".to_string(),
        name: "DualSense Wireless Controller".to_string(),
        alias: "PS5 Controller".to_string(),
        device_type: BluetoothDeviceType::Gamepad,
        connected: false,
        paired: true,
        trusted: true,
        blocked: false,
        rssi: Some(-62),
        battery_percentage: Some(70),
        icon: Some("input-gaming".to_string()),
    };

    let dev3 = BluetoothDevice {
        id: DeviceId("/org/bluez/hci0/dev_33_44_55_66_77_88".to_string()),
        address: "33:44:55:66:77:88".to_string(),
        name: "Keychron K2 Bluetooth Keyboard".to_string(),
        alias: "Keychron K2".to_string(),
        device_type: BluetoothDeviceType::Keyboard,
        connected: false,
        paired: true,
        trusted: true,
        blocked: false,
        rssi: Some(-55),
        battery_percentage: None,
        icon: Some("input-keyboard".to_string()),
    };

    let dev4 = BluetoothDevice {
        id: DeviceId("/org/bluez/hci0/dev_44_55_66_77_88_99".to_string()),
        address: "44:55:66:77:88:99".to_string(),
        name: "JBL Flip 6".to_string(),
        alias: "JBL Flip 6 (Próximo)".to_string(),
        device_type: BluetoothDeviceType::Audio,
        connected: false,
        paired: false,
        trusted: false,
        blocked: false,
        rssi: Some(-74),
        battery_percentage: None,
        icon: Some("audio-speakers".to_string()),
    };

    let adapter = BluetoothAdapter {
        id: DeviceId("/org/bluez/hci0".to_string()),
        address: "00:1A:7D:DA:71:13".to_string(),
        name: "Intel Wireless Bluetooth".to_string(),
        powered: true,
        discovering: false,
        discoverable: false,
        pairable: true,
    };

    app.bluetooth = Some(Box::new(BluetoothSnapshot {
        bluez_available: true,
        adapter: Some(adapter),
        devices: vec![dev1, dev2, dev3, dev4],
    }));

    let backend = TestBackend::new(100, 24);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

    let buffer = terminal.backend().buffer();
    let mut ansi_out = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            let cell = buffer.cell((x, y)).unwrap();
            ansi_out.push_str(cell.symbol());
        }
        ansi_out.push('\n');
    }
    let _ = std::fs::write("/tmp/hall9001_tab3_bluetooth.ansi", ansi_out);
}
