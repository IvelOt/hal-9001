
use hal9001::app::{App, Tab};
use hal9001::backend::audio::{
    parse_pactl_output, parse_wpctl_status, AudioCategory, AudioNode, AudioSnapshot,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::broadcast;

const WPCTL_STATUS_SAMPLE: &str = r#"
PipeWire 'pipewire-0' [1.6.8, ivelot@IvelPC, cookie:1213496054]
 └─ Clients:
        32. WirePlumber                         [1.6.8, ivelot@IvelPC, pid:1261]
        66. Firefox                             [1.6.8, ivelot@IvelPC, pid:1549641]
        80. Spotify                             [1.6.8, ivelot@IvelPC, pid:1550000]

Audio
 ├─ Devices:
 │      48. Áudio interno                      [alsa]
 │
 ├─ Sinks:
 │  *   57. Áudio interno Estéreo analógico  [vol: 0.80 MUTED]
 │      59. Fone de Ouvido Bluetooth          [vol: 1.00]
 │
 ├─ Sources:
 │  *   58. Microfone Interno Estéreo         [vol: 0.38]
 │
 ├─ Filters:
 │
 └─ Streams:
        66. Firefox                             [vol: 0.95]
        80. Spotify                             [vol: 0.60]
"#;

#[test]
fn test_parse_wpctl_status_sinks_sources_and_streams() {
    let snap = parse_wpctl_status(WPCTL_STATUS_SAMPLE).unwrap();

    assert_eq!(snap.server_name, "PipeWire (WirePlumber)");
    assert_eq!(snap.sinks.len(), 2);
    assert_eq!(snap.sources.len(), 1);
    assert_eq!(snap.apps.len(), 2);

    assert_eq!(snap.default_sink_id, Some(57));
    let sink1 = &snap.sinks[0];
    assert_eq!(sink1.id, 57);
    assert!(sink1.is_default);
    assert!(sink1.is_muted);
    assert_eq!((sink1.volume * 100.0).round() as u32, 80);

    let app1 = &snap.apps[0];
    assert_eq!(app1.id, 66);
    assert_eq!(app1.name, "Firefox");
    assert_eq!((app1.volume * 100.0).round() as u32, 95);
    assert!(!app1.is_muted);

    let app2 = &snap.apps[1];
    assert_eq!(app2.id, 80);
    assert_eq!(app2.name, "Spotify");
    assert_eq!((app2.volume * 100.0).round() as u32, 60);

    assert_eq!(snap.default_source_id, Some(58));
    let src1 = &snap.sources[0];
    assert_eq!(src1.id, 58);
    assert_eq!((src1.volume * 100.0).round() as u32, 38);
}

const WPCTL_STATUS_WITH_PORT_LINKS: &str = r#"
PipeWire 'pipewire-0' [1.6.8, ivelot@IvelPC, cookie:349836155]
Audio
 ├─ Sinks:
 │  *   67. 联想thinkplus-LP75                [vol: 0.50]
 ├─ Sources:
 │  *   58. Áudio interno Estéreo analógico  [vol: 0.38 MUTED]
 └─ Streams:
        90. Firefox
             79. output_FL       > 联想thinkplus-LP75:playback_FL	[active]
             81. output_FR       > 联想thinkplus-LP75:playback_FR	[active]
"#;

#[test]
fn test_parse_wpctl_filters_channel_links() {
    let snap = parse_wpctl_status(WPCTL_STATUS_WITH_PORT_LINKS).unwrap();
    assert_eq!(snap.apps.len(), 1);
    assert_eq!(snap.apps[0].id, 90);
    assert_eq!(snap.apps[0].name, "Firefox");
}

#[test]
fn test_parse_pactl_output_fallback() {
    let sinks_raw = r#"
Sink #1
    Name: alsa_output.pci-0000_00_1f.3.analog-stereo
    Description: Built-in Audio Analog Stereo
    Mute: no
    Volume: front-left: 65536 / 100% / 0.00 dB,   front-right: 65536 / 100% / 0.00 dB
"#;
    let apps_raw = r#"
Sink Input #42
    application.name = "Spotify"
    Mute: yes
    Volume: front-left: 32768 / 50% / -18.06 dB
"#;
    let sources_raw = r#"
Source #2
    Name: alsa_input.pci-0000_00_1f.3.analog-stereo
    Description: Built-in Microphone
    Mute: no
    Volume: front-left: 45875 / 70% / -9.28 dB
"#;

    let snap = parse_pactl_output(sinks_raw, apps_raw, sources_raw);
    assert_eq!(snap.server_name, "PulseAudio");
    assert_eq!(snap.sinks.len(), 1);
    assert_eq!(snap.apps.len(), 1);
    assert_eq!(snap.sources.len(), 1);

    assert_eq!(snap.sinks[0].volume_percent(), 100);
    assert_eq!(snap.apps[0].name, "Spotify");
    assert!(snap.apps[0].is_muted);
    assert_eq!(snap.apps[0].volume_percent(), 50);
}

#[test]
fn test_audio_mixer_navigation_and_actions() {
    let mut app = App::new(Config::default());
    app.phase = hal9001::app::Phase::Running;
    app.active = Tab::Audio;

    let (action_tx, mut action_rx) = broadcast::channel(16);

    let snap = parse_wpctl_status(WPCTL_STATUS_SAMPLE).unwrap();
    let follow_ups = app.handle_event(AppEvent::Audio(Box::new(snap)));
    assert!(follow_ups.is_empty());
    assert!(app.audio.is_some());

    assert_eq!(app.audio_category, 0);
    assert_eq!(app.audio_selected, 0);

    app.dispatch(Action::Down, &action_tx);
    assert_eq!(app.audio_selected, 1);

    app.dispatch(Action::Enter, &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::AudioSetDefault(59)
    );

    app.dispatch(Action::VolumeUp, &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::AudioVolumeUp(59, 0.05)
    );

    app.dispatch(Action::VolumeDown, &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::AudioVolumeDown(59, 0.05)
    );

    app.dispatch(Action::ToggleMute, &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::AudioToggleMute(59)
    );

    app.dispatch(Action::AudioSelectCategory(1), &action_tx);
    assert_eq!(app.audio_category, 1);
    assert_eq!(app.audio_selected, 0);

    app.dispatch(Action::Enter, &action_tx);
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::AudioToggleMute(66)
    );
}

#[test]
fn test_audio_headless_ui_render() {
    let mut app = App::new(Config::default());
    app.phase = hal9001::app::Phase::Running;
    app.active = Tab::Audio;

    let node1 = AudioNode {
        id: 57,
        name: "Áudio Interno (Alto-falantes)".to_string(),
        description: "Built-in Analog Stereo".to_string(),
        category: AudioCategory::Sink,
        volume: 0.85,
        is_muted: false,
        is_default: true,
        icon_name: Some("audio-speakers".to_string()),
    };

    let node2 = AudioNode {
        id: 59,
        name: "Sony WH-1000XM5 (Bluetooth A2DP)".to_string(),
        description: "Bluetooth Headset".to_string(),
        category: AudioCategory::Sink,
        volume: 1.20,
        is_muted: false,
        is_default: false,
        icon_name: Some("audio-headset".to_string()),
    };

    let snap = AudioSnapshot {
        server_name: "PipeWire 1.6.8".to_string(),
        sinks: vec![node1, node2],
        apps: Vec::new(),
        sources: Vec::new(),
        default_sink_id: Some(57),
        default_source_id: None,
    };

    app.audio = Some(Box::new(snap));

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
    let _ = std::fs::write("/tmp/hall9001_tab5_audio.ansi", ansi_out);
}
