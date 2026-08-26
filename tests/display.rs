use hal9001::app::{App, Tab};
use hal9001::backend::display::{
    parse_xrandr_query, DisplayLayoutMode, DisplayMode, DisplayNode, DisplaySnapshot,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::broadcast;

const XRANDR_SAMPLE_MULTI_MONITOR: &str = r#"
Screen 0: minimum 320 x 200, current 3286 x 1080, maximum 16384 x 16384
eDP-1 connected primary 1366x768+0+0 (normal left inverted right x axis y axis) 344mm x 193mm
   1366x768      60.00*+
   1280x720      60.00    59.99    59.86
   1024x768      60.04    60.00
HDMI-1 connected 1920x1080+1366+0 (normal left inverted right x axis y axis) 598mm x 336mm
   1920x1080     60.00*+  59.94    50.00
   1680x1050     59.88
   1280x720      60.00    59.94    50.00
DP-1 disconnected (normal left inverted right x axis y axis)
"#;

#[test]
fn test_parse_xrandr_query_multi_monitor() {
    let snap = parse_xrandr_query(XRANDR_SAMPLE_MULTI_MONITOR).unwrap();

    assert_eq!(snap.displays.len(), 3);
    assert_eq!(snap.connected_count, 2);
    assert_eq!(snap.primary_name, Some("eDP-1".to_string()));

    let internal = snap.internal_display().unwrap();
    assert_eq!(internal.name, "eDP-1");
    assert!(internal.is_connected);
    assert!(internal.is_primary);
    assert!(internal.is_active);
    assert_eq!(internal.pos_x, 0);
    assert_eq!(internal.pos_y, 0);
    assert_eq!(internal.supported_modes.len(), 3);
    assert_eq!(internal.current_mode.as_ref().unwrap().width, 1366);
    assert_eq!(internal.current_mode.as_ref().unwrap().height, 768);

    let external = snap.external_display().unwrap();
    assert_eq!(external.name, "HDMI-1");
    assert!(external.is_connected);
    assert!(!external.is_primary);
    assert!(external.is_active);
    assert_eq!(external.pos_x, 1366);
    assert_eq!(external.pos_y, 0);
    assert_eq!(external.supported_modes.len(), 3);
    assert_eq!(external.current_mode.as_ref().unwrap().width, 1920);
    assert_eq!(external.current_mode.as_ref().unwrap().height, 1080);

    let dp = &snap.displays[2];
    assert_eq!(dp.name, "DP-1");
    assert!(!dp.is_connected);
    assert!(!dp.is_active);

    assert_eq!(snap.current_layout, Some(DisplayLayoutMode::ExtendRight));
}

#[test]
fn test_display_app_navigation_and_actions() {
    let mut app = App::new(Config::default());
    app.phase = hal9001::app::Phase::Running;
    app.active = Tab::Displays;

    let (action_tx, mut action_rx) = broadcast::channel(16);

    let snap = parse_xrandr_query(XRANDR_SAMPLE_MULTI_MONITOR).unwrap();
    let follow_ups = app.handle_event(AppEvent::Display(Box::new(snap)));
    assert!(follow_ups.is_empty());
    assert!(app.displays.is_some());

    app.dispatch(Action::Right, &action_tx);
    assert_eq!(app.display_selected, 1);

    app.dispatch(
        Action::DisplaySetLayout(DisplayLayoutMode::Mirror),
        &action_tx,
    );
    assert_eq!(
        action_rx.try_recv().unwrap(),
        Action::DisplaySetLayout(DisplayLayoutMode::Mirror)
    );
}

#[test]
fn test_display_headless_ui_render() {
    let mut app = App::new(Config::default());
    app.phase = hal9001::app::Phase::Running;
    app.active = Tab::Displays;

    let d1 = DisplayNode {
        name: "eDP-1".to_string(),
        is_connected: true,
        is_primary: true,
        is_active: true,
        current_mode: Some(DisplayMode {
            width: 1366,
            height: 768,
            rate: 60.0,
            is_current: true,
            is_preferred: true,
        }),
        supported_modes: vec![],
        pos_x: 0,
        pos_y: 0,
        rotation: "normal".to_string(),
        is_internal: true,
    };

    let d2 = DisplayNode {
        name: "HDMI-1".to_string(),
        is_connected: true,
        is_primary: false,
        is_active: true,
        current_mode: Some(DisplayMode {
            width: 1920,
            height: 1080,
            rate: 60.0,
            is_current: true,
            is_preferred: true,
        }),
        supported_modes: vec![],
        pos_x: 1366,
        pos_y: 0,
        rotation: "normal".to_string(),
        is_internal: false,
    };

    let snap = DisplaySnapshot {
        displays: vec![d1, d2],
        primary_name: Some("eDP-1".to_string()),
        connected_count: 2,
        server_type: String::new(),
        current_layout: Some(DisplayLayoutMode::ExtendRight),
    };

    app.displays = Some(Box::new(snap));

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
    let _ = std::fs::write("/tmp/hall9001_tab6_display.ansi", ansi_out);
}

const WLR_RANDR_SAMPLE: &str = r#"
eDP-1 "Unknown (0x0000)"
  Physical size: 310x170 mm
  Enabled: yes
  Modes:
    1920x1080 px, 60.000000 Hz (preferred, current)
    1280x720 px, 60.000000 Hz
  Position: 0,0
  Transform: normal
  Scale: 1.000000
HDMI-A-1 "Unknown (0x0000)"
  Physical size: 600x340 mm
  Enabled: yes
  Modes:
    1920x1080 px, 60.000000 Hz (preferred, current)
  Position: 1920,0
  Transform: normal
  Scale: 1.000000
"#;

#[test]
fn test_parse_wlr_randr_impl() {
    let snap = hal9001::backend::display::parse_wlr_randr(WLR_RANDR_SAMPLE).unwrap();
    assert_eq!(snap.displays.len(), 2);
    assert_eq!(snap.connected_count, 2);

    let internal = snap.internal_display().unwrap();
    assert_eq!(internal.name, "eDP-1");
    assert!(internal.is_active);
    assert_eq!(internal.pos_x, 0);
    assert_eq!(internal.current_mode.as_ref().unwrap().width, 1920);

    let external = snap.external_display().unwrap();
    assert_eq!(external.name, "HDMI-A-1");
    assert!(external.is_active);
    assert_eq!(external.pos_x, 1920);

    assert_eq!(
        snap.current_layout,
        Some(hal9001::backend::display::DisplayLayoutMode::ExtendRight)
    );
}

const HYPRCTL_SAMPLE: &str = r#"
Monitor eDP-1 (ID 0):
	1920x1080@60.00000 at 0x0
	description: Unknown
	make: Unknown
	model: Unknown
	serial: Unknown
	active workspace: 1 (1)
	special workspace: 0 ()
	reserved: 0 0 0 0
	scale: 1.00
	transform: 0
	focused: yes
	dpmsStatus: 1
	vrr: 0
	solitary:
	availableModes: 1920x1080@60.00000 1280x720@60.00000

Monitor HDMI-A-1 (ID 1):
	1920x1080@60.00000 at 1920x0
	description: Unknown
	make: Unknown
	model: Unknown
	serial: Unknown
	active workspace: 2 (2)
	special workspace: 0 ()
	reserved: 0 0 0 0
	scale: 1.00
	transform: 0
	focused: no
	dpmsStatus: 1
	vrr: 0
	solitary:
	availableModes: 1920x1080@60.00000
"#;

#[test]
fn test_parse_hyprctl_impl() {
    let snap = hal9001::backend::display::parse_hyprctl_monitors(HYPRCTL_SAMPLE).unwrap();
    assert_eq!(snap.displays.len(), 2);

    let internal = snap.internal_display().unwrap();
    assert_eq!(internal.name, "eDP-1");
    assert!(internal.is_active);
    assert_eq!(internal.pos_x, 0);
    assert_eq!(internal.current_mode.as_ref().unwrap().width, 1920);

    let external = snap.external_display().unwrap();
    assert_eq!(external.name, "HDMI-A-1");
    assert!(external.is_active);
    assert_eq!(external.pos_x, 1920);

    assert_eq!(
        snap.current_layout,
        Some(hal9001::backend::display::DisplayLayoutMode::ExtendRight)
    );
}
