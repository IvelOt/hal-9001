//! Testes do Módulo 2 — Wi-Fi & Rede (NetworkManager).

use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::broadcast;

use hal9001::app::{App, Tab, WifiPasswordPromptState};
use hal9001::backend::network::{
    band_of, derive_security, AccessPoint, NetTelemetry, NetworkSnapshot, Security, WifiBand,
    WifiDevice,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent, DeviceId};
use hal9001::ui;

#[test]
fn test_security_derivation() {
    // Open
    assert_eq!(derive_security(0, 0, 0), Security::Open);

    // WEP (Privacy flag without WPA/RSN)
    assert_eq!(derive_security(0x1, 0, 0), Security::Wep);

    // WPA1
    assert_eq!(derive_security(0x1, 0x100, 0), Security::Wpa);

    // WPA2-PSK (RSN 0x188 = 392)
    assert_eq!(derive_security(0x1, 0, 392), Security::Wpa2);

    // WPA3-SAE (RSN with 0x400)
    assert_eq!(derive_security(0x1, 0, 0x400), Security::Wpa3);

    // WPA2-Enterprise (0x200)
    assert_eq!(derive_security(0x1, 0, 0x200), Security::Wpa2Enterprise);
}

#[test]
fn test_band_of_frequency() {
    assert_eq!(band_of(2412), WifiBand::Ghz24);
    assert_eq!(band_of(2462), WifiBand::Ghz24);
    assert_eq!(band_of(5180), WifiBand::Ghz5);
    assert_eq!(band_of(5745), WifiBand::Ghz5);
    assert_eq!(band_of(6000), WifiBand::Ghz6);
    assert_eq!(band_of(900), WifiBand::Unknown);
}

#[test]
fn test_network_actions_and_event_handling() {
    let mut app = App::new(Config::default());
    app.active = Tab::Network;

    let (action_tx, mut action_rx) = broadcast::channel::<Action>(16);

    // Simulate Network Snapshot event
    let snap = NetworkSnapshot {
        nm_available: true,
        networking_enabled: true,
        wireless_enabled: true,
        wireless_hw_enabled: true,
        wifi_device: Some(WifiDevice {
            id: DeviceId("/org/freedesktop/NetworkManager/Devices/62".to_string()),
            iface: "wlan0".to_string(),
            hw_address: "AA:BB:CC:DD:EE:FF".to_string(),
            state: 100,
            bitrate_kbps: 866700,
            active_ap: Some(DeviceId("/ap/1".to_string())),
            last_scan_ms: 1000,
        }),
        access_points: vec![
            AccessPoint {
                id: DeviceId("/ap/1".to_string()),
                ssid: "Home_5G".to_string(),
                ssid_raw: b"Home_5G".to_vec(),
                bssid: "11:22:33:44:55:66".to_string(),
                strength: 90,
                frequency: 5745,
                band: WifiBand::Ghz5,
                max_bitrate_kbps: 866700,
                security: Security::Wpa2,
                is_active: true,
                is_saved: true,
                saved_conn_path: Some("/org/freedesktop/NetworkManager/Settings/1".to_string()),
            },
            AccessPoint {
                id: DeviceId("/ap/2".to_string()),
                ssid: "Coffee_Shop".to_string(),
                ssid_raw: b"Coffee_Shop".to_vec(),
                bssid: "77:88:99:AA:BB:CC".to_string(),
                strength: 65,
                frequency: 2412,
                band: WifiBand::Ghz24,
                max_bitrate_kbps: 144000,
                security: Security::Wpa2,
                is_active: false,
                is_saved: false,
                saved_conn_path: None,
            },
        ],
        active: None,
        telemetry: NetTelemetry {
            ipv4: Some("192.168.3.6".to_string()),
            gateway: Some("192.168.3.1".to_string()),
            dns: vec!["192.168.3.1".to_string()],
            rx_rate_kbps: 120.5,
            tx_rate_kbps: 15.2,
            total_rx_bytes: 10000,
            total_tx_bytes: 5000,
        },
    };

    let _ = app.handle_event(AppEvent::Network(Box::new(snap)));
    assert!(app.network.is_some());
    assert_eq!(app.network.as_ref().unwrap().access_points.len(), 2);

    // Test Rescan action dispatch
    app.dispatch(Action::NetworkRescan, &action_tx);
    let recv = action_rx.try_recv();
    assert!(matches!(recv, Ok(Action::NetworkRescan)));

    // Test Toggle Radio action dispatch
    app.dispatch(Action::NetworkToggleRadio, &action_tx);
    let recv = action_rx.try_recv();
    assert!(matches!(recv, Ok(Action::NetworkToggleRadio)));

    // Select second AP (unsaved WPA2) and press Enter -> opens wifi prompt
    app.network_selected = 1;
    app.dispatch(Action::Enter, &action_tx);
    assert!(app.wifi_prompt.is_some());
    assert_eq!(app.wifi_prompt.as_ref().unwrap().ssid, "Coffee_Shop");

    // Type password in modal
    app.dispatch(Action::NetworkModalChar('s'), &action_tx);
    app.dispatch(Action::NetworkModalChar('e'), &action_tx);
    app.dispatch(Action::NetworkModalChar('c'), &action_tx);
    app.dispatch(Action::NetworkModalChar('r'), &action_tx);
    app.dispatch(Action::NetworkModalChar('e'), &action_tx);
    app.dispatch(Action::NetworkModalChar('t'), &action_tx);
    assert_eq!(app.wifi_prompt.as_ref().unwrap().password, "secret");

    // Press Enter to connect
    app.dispatch(Action::Enter, &action_tx);
    assert!(app.wifi_prompt.is_none());
    let recv = action_rx.try_recv();
    assert!(matches!(
        recv,
        Ok(Action::NetworkConnect {
            password: Some(ref p),
            ..
        }) if p == "secret"
    ));
}

#[test]
fn test_render_network_tab_without_panic() {
    let mut app = App::new(Config::default());
    app.active = Tab::Network;

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    // 1. Render without snapshot (pending)
    terminal.draw(|f| ui::draw(&app, f)).unwrap();

    // 2. Render with snapshot
    let snap = NetworkSnapshot {
        nm_available: true,
        networking_enabled: true,
        wireless_enabled: true,
        wireless_hw_enabled: true,
        wifi_device: Some(WifiDevice {
            id: DeviceId("/dev/1".to_string()),
            iface: "wlan0".to_string(),
            hw_address: "AA:BB:CC:DD:EE:FF".to_string(),
            state: 100,
            bitrate_kbps: 866700,
            active_ap: None,
            last_scan_ms: 1000,
        }),
        access_points: vec![AccessPoint {
            id: DeviceId("/ap/1".to_string()),
            ssid: "Test_SSID".to_string(),
            ssid_raw: b"Test_SSID".to_vec(),
            bssid: "11:22:33:44:55:66".to_string(),
            strength: 85,
            frequency: 5745,
            band: WifiBand::Ghz5,
            max_bitrate_kbps: 866700,
            security: Security::Wpa2,
            is_active: true,
            is_saved: true,
            saved_conn_path: None,
        }],
        active: None,
        telemetry: NetTelemetry::default(),
    };

    let _ = app.handle_event(AppEvent::Network(Box::new(snap)));
    terminal.draw(|f| ui::draw(&app, f)).unwrap();

    // 3. Render with Wi-Fi Password Prompt modal open
    app.wifi_prompt = Some(WifiPasswordPromptState {
        ap_id: "/ap/1".to_string(),
        ssid: "Test_SSID".to_string(),
        password: "password123".to_string(),
        error: Some("Senha incorreta".to_string()),
    });
    terminal.draw(|f| ui::draw(&app, f)).unwrap();
}
