//! Testes de fumaça: exercitam lógica pura sem entrar no alt-screen.

use hal9001::app::{App, Phase, Tab};
use hal9001::config::Config;
use hal9001::ui::widgets::{human_bytes, human_uptime};

#[test]
fn tabs_round_trip_by_index() {
    for (i, tab) in Tab::ALL.iter().enumerate() {
        assert_eq!(tab.index(), i);
        assert_eq!(Tab::from_index(i), *tab);
    }
}

#[test]
fn next_prev_tab_wraps() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let mut app = App::new(cfg);

    assert_eq!(app.active, Tab::Overview);
    app.dispatch(hal9001::events::Action::PrevTab, &tx);
    assert_eq!(app.active, Tab::Terminal); // wrap-around
    app.dispatch(hal9001::events::Action::NextTab, &tx);
    assert_eq!(app.active, Tab::Overview);
}

#[test]
fn splash_promotes_to_running_on_key() {
    let cfg = Config::default();
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let mut app = App::new(cfg);
    assert_eq!(app.phase, Phase::Splash);
    // Qualquer ação durante a splash revela o dashboard.
    app.dispatch(hal9001::events::Action::Down, &tx);
    assert_eq!(app.phase, Phase::Running);
}

#[test]
fn quit_sets_flag() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let mut app = App::new(cfg);
    assert!(!app.should_quit);
    app.dispatch(hal9001::events::Action::Quit, &tx);
    assert!(app.should_quit);
}

#[test]
fn render_all_tabs_and_splash_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut cfg = Config::default();
    cfg.splash.enabled = true;
    let mut app = App::new(cfg);

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

    // Splash frame.
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

    // Feed a system snapshot so the Overview has real data to lay out.
    app.handle_event(hal9001::events::AppEvent::System(Box::new(
        hal9001::backend::system::SystemSnapshot {
            host: "testhost".into(),
            user: "tester".into(),
            shell: "zsh".into(),
            os: "Linux Test".into(),
            kernel: "6.0".into(),
            uptime_secs: 3600,
            cpu_name: "Test CPU".into(),
            cpu_usage: 42.0,
            mem_used: 4 * 1024 * 1024 * 1024,
            mem_total: 16 * 1024 * 1024 * 1024,
            host_model: Some("ACME Laptop 9000".into()),
            packages: Some(hal9001::backend::system::Packages {
                total: 1234,
                by_manager: vec![("pacman", 1200), ("flatpak", 34)],
            }),
            brightness: Some(0.7),
            volume: Some(hal9001::backend::system::Volume {
                level: 0.65,
                muted: false,
            }),
            battery: Some(hal9001::backend::system::Battery {
                percent: 82.0,
                status: hal9001::backend::system::BatteryStatus::Charging,
                power_watts: Some(12.5),
            }),
            disk_used: Some(120 * 1024 * 1024 * 1024),
            disk_total: Some(512 * 1024 * 1024 * 1024),
        },
    )));

    // Promote to running and render every tab + help overlay.
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(hal9001::events::Action::Redraw, &tx); // leaves splash
    for i in 0..Tab::ALL.len() {
        app.dispatch(hal9001::events::Action::SelectTab(i), &tx);
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
    app.dispatch(hal9001::events::Action::ToggleHelp, &tx);
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}

#[test]
fn render_overview_desktop_degraded_without_panic() {
    // Máquina "desktop": sem bateria, brilho, volume, disco ou pacotes.
    // A UI deve renderizar "N/A" sem entrar em pânico.
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);

    app.handle_event(hal9001::events::AppEvent::System(Box::new(
        hal9001::backend::system::SystemSnapshot {
            host: "desktop".into(),
            user: "op".into(),
            shell: "bash".into(),
            os: "Linux".into(),
            kernel: "6.0".into(),
            uptime_secs: 120,
            cpu_name: "Desktop CPU".into(),
            cpu_usage: 5.0,
            mem_used: 1024,
            mem_total: 8192,
            host_model: None,
            packages: None,
            brightness: None,
            volume: None,
            battery: None,
            disk_used: None,
            disk_total: None,
        },
    )));

    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}

#[test]
fn human_helpers_format() {
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(1024), "1.0 KiB");
    assert_eq!(human_uptime(90_061), "1d 1h 1m");
}
