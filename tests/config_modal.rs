//! Testes unitários e de integração do Modal Interativo de Configurações.

use hal9001::app::App;
use hal9001::config::Config;
use hal9001::events::Action;
use hal9001::i18n::Language;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::broadcast;

#[test]
fn toggle_config_modal() {
    let (tx, _rx) = broadcast::channel(16);
    let mut app = App::new(Config::default());
    assert!(!app.show_config);

    app.dispatch(Action::ToggleConfig, &tx);
    assert!(app.show_config);

    app.dispatch(Action::ToggleConfig, &tx);
    assert!(!app.show_config);
}

#[test]
fn navigate_and_cycle_config_fields() {
    let (tx, _rx) = broadcast::channel(16);
    let mut app = App::new(Config::default());
    app.dispatch(Action::ToggleConfig, &tx);
    assert!(app.show_config);
    assert_eq!(app.config_cursor, 0);

    // Row 0: Language. Cycle forward.
    let initial_lang = app.config.ui.language.clone();
    app.dispatch(Action::Right, &tx);
    assert_ne!(app.config.ui.language, initial_lang);

    // Navigate down to Theme
    app.dispatch(Action::Down, &tx);
    assert_eq!(app.config_cursor, 1);

    // Cycle Theme
    assert_eq!(app.config.theme.name, "hal");
    app.dispatch(Action::Right, &tx);
    assert_eq!(app.config.theme.name, "mono");

    // Navigate down to ASCII Logo
    app.dispatch(Action::Down, &tx);
    assert_eq!(app.config_cursor, 2);
    app.dispatch(Action::Right, &tx);
    assert_eq!(app.config.overview.ascii, "main");

    // Navigate down to Icons
    app.dispatch(Action::Down, &tx);
    assert_eq!(app.config_cursor, 3);
    assert!(app.config.ui.icons);
    app.dispatch(Action::Enter, &tx);
    assert!(!app.config.ui.icons);

    // Navigate down to FPS
    app.dispatch(Action::Down, &tx);
    assert_eq!(app.config_cursor, 4);
    assert_eq!(app.config.ui.frame_ms, 33);
    app.dispatch(Action::Right, &tx);
    assert_eq!(app.config.ui.frame_ms, 16);

    // Navigate down to Splash
    app.dispatch(Action::Down, &tx);
    assert_eq!(app.config_cursor, 5);
    assert!(app.config.splash.enabled);
    app.dispatch(Action::Left, &tx);
    assert!(!app.config.splash.enabled);

    // Wrap around to Row 0
    app.dispatch(Action::Down, &tx);
    assert_eq!(app.config_cursor, 0);
}

#[test]
fn realtime_language_switch_in_modal() {
    let (tx, _rx) = broadcast::channel(16);
    let mut cfg = Config::default();
    cfg.ui.language = "en-US".to_string();
    let mut app = App::new(cfg);
    assert_eq!(app.lang, Language::EnUs);

    app.dispatch(Action::ToggleConfig, &tx);
    assert_eq!(app.config_cursor, 0);

    // Cycle language forward to "es-ES"
    app.dispatch(Action::Right, &tx);
    assert_eq!(app.config.ui.language, "es-ES");
    assert_eq!(app.lang, Language::EsEs);
}

#[test]
fn render_config_modal_without_panic() {
    let mut app = App::new(Config::default());
    app.show_config = true;

    for (w, h) in [(80u16, 24u16), (120, 40), (60, 20)] {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
}
