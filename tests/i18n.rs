
use hal9001::app::{App, Tab};
use hal9001::backend::system::PowerProfile;
use hal9001::config::Config;
use hal9001::i18n::Language;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

#[test]
fn language_parsing_variants() {
    assert_eq!(Language::parse("pt-BR"), Some(Language::PtBr));
    assert_eq!(Language::parse("pt_BR.UTF-8"), Some(Language::PtBr));
    assert_eq!(Language::parse("pt"), Some(Language::PtBr));
    assert_eq!(Language::parse("PT"), Some(Language::PtBr));

    assert_eq!(Language::parse("en-US"), Some(Language::EnUs));
    assert_eq!(Language::parse("en_US.UTF-8"), Some(Language::EnUs));
    assert_eq!(Language::parse("en-GB"), Some(Language::EnUs));
    assert_eq!(Language::parse("en"), Some(Language::EnUs));

    assert_eq!(Language::parse("es-ES"), Some(Language::EsEs));
    assert_eq!(Language::parse("es_ES.UTF-8"), Some(Language::EsEs));
    assert_eq!(Language::parse("es"), Some(Language::EsEs));

    assert_eq!(Language::parse("invalid-lang"), None);
}

#[test]
fn tab_titles_in_all_languages() {

    assert_eq!(Tab::Overview.title_in(Language::PtBr), "Overview");
    assert_eq!(Tab::Network.title_in(Language::PtBr), "Rede");
    assert_eq!(Tab::Audio.title_in(Language::PtBr), "Áudio");
    assert_eq!(Tab::Displays.title_in(Language::PtBr), "Telas");

    assert_eq!(Tab::Overview.title_in(Language::EnUs), "Overview");
    assert_eq!(Tab::Network.title_in(Language::EnUs), "Network");
    assert_eq!(Tab::Audio.title_in(Language::EnUs), "Audio");
    assert_eq!(Tab::Displays.title_in(Language::EnUs), "Displays");

    assert_eq!(Tab::Overview.title_in(Language::EsEs), "Visión General");
    assert_eq!(Tab::Network.title_in(Language::EsEs), "Red");
    assert_eq!(Tab::Audio.title_in(Language::EsEs), "Audio");
    assert_eq!(Tab::Displays.title_in(Language::EsEs), "Pantallas");
}

#[test]
fn power_profile_labels_in_all_languages() {
    assert_eq!(PowerProfile::PowerSaver.label_in(Language::PtBr), "Economia");
    assert_eq!(PowerProfile::Balanced.label_in(Language::PtBr), "Equilibrado");
    assert_eq!(PowerProfile::Performance.label_in(Language::PtBr), "Desempenho");

    assert_eq!(PowerProfile::PowerSaver.label_in(Language::EnUs), "Power Saver");
    assert_eq!(PowerProfile::Balanced.label_in(Language::EnUs), "Balanced");
    assert_eq!(PowerProfile::Performance.label_in(Language::EnUs), "Performance");

    assert_eq!(PowerProfile::PowerSaver.label_in(Language::EsEs), "Ahorro");
    assert_eq!(PowerProfile::Balanced.label_in(Language::EsEs), "Equilibrado");
    assert_eq!(PowerProfile::Performance.label_in(Language::EsEs), "Rendimiento");
}

#[test]
fn all_messages_are_non_empty() {
    for lang in [Language::PtBr, Language::EnUs, Language::EsEs] {
        let m = lang.messages();
        assert!(!m.app_title_suffix.is_empty());
        assert!(!m.splash_title.is_empty());
        assert!(!m.splash_loading.is_empty());
        assert!(!m.splash_welcome.is_empty());
        assert!(!m.sec_compute.is_empty());
        assert!(!m.sec_system.is_empty());
        assert!(!m.sec_peripherals.is_empty());
        assert!(!m.sec_palette.is_empty());
        assert!(!m.label_ram.is_empty());
        assert!(!m.label_battery.is_empty());
        assert!(!m.label_brightness.is_empty());
        assert!(!m.label_volume.is_empty());
        assert!(!m.label_power_profile.is_empty());
    }
}

#[test]
fn config_language_resolution() {
    let mut cfg = Config::default();
    cfg.ui.language = "en-US".to_string();
    assert_eq!(cfg.ui.resolved_language(), Language::EnUs);

    cfg.ui.language = "es-ES".to_string();
    assert_eq!(cfg.ui.resolved_language(), Language::EsEs);

    cfg.ui.language = "pt-BR".to_string();
    assert_eq!(cfg.ui.resolved_language(), Language::PtBr);
}

#[test]
fn render_ui_in_all_languages_without_panic() {
    for lang in [Language::PtBr, Language::EnUs, Language::EsEs] {
        let mut cfg = Config::default();
        cfg.ui.language = lang.code().to_string();
        let app = App::new(cfg);

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
}
