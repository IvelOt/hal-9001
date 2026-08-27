use hal9001::app::{App, Tab};
use hal9001::backend::display::DisplayLayoutMode;
use hal9001::backend::system::PowerProfile;
use hal9001::config::Config;
use hal9001::i18n::{Language, Messages, SharedLang};
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
    assert_eq!(
        PowerProfile::PowerSaver.label_in(Language::PtBr),
        "Economia"
    );
    assert_eq!(
        PowerProfile::Balanced.label_in(Language::PtBr),
        "Equilibrado"
    );
    assert_eq!(
        PowerProfile::Performance.label_in(Language::PtBr),
        "Desempenho"
    );

    assert_eq!(
        PowerProfile::PowerSaver.label_in(Language::EnUs),
        "Power Saver"
    );
    assert_eq!(PowerProfile::Balanced.label_in(Language::EnUs), "Balanced");
    assert_eq!(
        PowerProfile::Performance.label_in(Language::EnUs),
        "Performance"
    );

    assert_eq!(PowerProfile::PowerSaver.label_in(Language::EsEs), "Ahorro");
    assert_eq!(
        PowerProfile::Balanced.label_in(Language::EsEs),
        "Equilibrado"
    );
    assert_eq!(
        PowerProfile::Performance.label_in(Language::EsEs),
        "Rendimiento"
    );
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

/// All newly-added message fields across network, bluetooth, audio, display,
/// config modal, overview, backend toasts, and shared UI chrome. Ensures
/// every one of the three languages has a non-empty, real translation.
type MessageField = (&'static str, fn(&Messages) -> &'static str);

fn new_message_fields() -> Vec<MessageField> {
    vec![
        ("pending_module_power", |m| m.pending_module_power),
        ("pending_module_updates", |m| m.pending_module_updates),
        ("backend_not_implemented", |m| m.backend_not_implemented),
        ("err_dir_permission_denied", |m| m.err_dir_permission_denied),
        ("err_dir_not_found", |m| m.err_dir_not_found),
        ("err_brightness_unavailable", |m| {
            m.err_brightness_unavailable
        }),
        ("err_kbd_unavailable", |m| m.err_kbd_unavailable),
        ("err_no_audio_backend", |m| m.err_no_audio_backend),
        ("err_volume_unavailable", |m| m.err_volume_unavailable),
        ("err_no_power_backend", |m| m.err_no_power_backend),
        ("toast_airplane_off", |m| m.toast_airplane_off),
        ("toast_airplane_on", |m| m.toast_airplane_on),
        ("toast_updates_available", |m| m.toast_updates_available),
        ("toast_updates_pending", |m| m.toast_updates_pending),
        ("toast_system_updated", |m| m.toast_system_updated),
        ("toast_checking_updates", |m| m.toast_checking_updates),
        ("storage_err_mount_prefix", |m| m.storage_err_mount_prefix),
        ("storage_err_no_mountable_partition", |m| {
            m.storage_err_no_mountable_partition
        }),
        ("storage_toast_eject_success", |m| {
            m.storage_toast_eject_success
        }),
        ("storage_toast_format_done", |m| m.storage_toast_format_done),
        ("storage_err_needs_fat32", |m| m.storage_err_needs_fat32),
        ("storage_err_cancelled_by_user", |m| {
            m.storage_err_cancelled_by_user
        }),
        ("storage_toast_flash_success", |m| {
            m.storage_toast_flash_success
        }),
        ("storage_err_udisks_unavailable", |m| {
            m.storage_err_udisks_unavailable
        }),
        ("net_err_rescan_failed", |m| m.net_err_rescan_failed),
        ("net_toast_scan_started", |m| m.net_toast_scan_started),
        ("net_err_device_not_found", |m| m.net_err_device_not_found),
        ("audio_server_unavailable", |m| m.audio_server_unavailable),
        ("audio_generic_device", |m| m.audio_generic_device),
        ("audio_cat_sink", |m| m.audio_cat_sink),
        ("audio_cat_appstream", |m| m.audio_cat_appstream),
        ("audio_cat_source", |m| m.audio_cat_source),
        ("display_mode_extend_right", |m| m.display_mode_extend_right),
        ("display_mode_mirror", |m| m.display_mode_mirror),
        ("tag_battery", |m| m.tag_battery),
        ("tag_power", |m| m.tag_power),
        ("tag_process", |m| m.tag_process),
        ("tag_disk", |m| m.tag_disk),
        ("tag_system", |m| m.tag_system),
        ("tag_airplane", |m| m.tag_airplane),
        ("tag_keyboard", |m| m.tag_keyboard),
        ("tag_flasher", |m| m.tag_flasher),
        ("tag_network", |m| m.tag_network),
        ("toast_charger_connected", |m| m.toast_charger_connected),
        ("toast_on_battery", |m| m.toast_on_battery),
        ("toast_battery_critical_label", |m| {
            m.toast_battery_critical_label
        }),
        ("toast_device_connected", |m| m.toast_device_connected),
        ("toast_connected_at", |m| m.toast_connected_at),
        ("toast_kill_signal_sent", |m| m.toast_kill_signal_sent),
        ("help_title", |m| m.help_title),
        ("statusline_hints_narrow", |m| m.statusline_hints_narrow),
        ("network_pending_title", |m| m.network_pending_title),
        ("network_err_nm_unavailable", |m| {
            m.network_err_nm_unavailable
        }),
        ("network_title_available", |m| m.network_title_available),
        ("network_wifi_auth_title", |m| m.network_wifi_auth_title),
        ("bt_pending_title", |m| m.bt_pending_title),
        ("bt_err_bluez_unavailable", |m| m.bt_err_bluez_unavailable),
        ("bt_status_available", |m| m.bt_status_available),
        ("audio_pending_title", |m| m.audio_pending_title),
        ("audio_empty_sink", |m| m.audio_empty_sink),
        ("audio_title_input_devices", |m| m.audio_title_input_devices),
        ("display_pending_title", |m| m.display_pending_title),
        ("display_no_video_output", |m| m.display_no_video_output),
        ("display_title_modes", |m| m.display_title_modes),
        ("cfg_icons_enabled", |m| m.cfg_icons_enabled),
        ("cfg_icons_disabled", |m| m.cfg_icons_disabled),
        ("cfg_splash_enabled", |m| m.cfg_splash_enabled),
        ("cfg_polling_performance", |m| m.cfg_polling_performance),
        ("cfg_polling_eco", |m| m.cfg_polling_eco),
        ("cfg_polling_balanced", |m| m.cfg_polling_balanced),
        ("cfg_theme_mono", |m| m.cfg_theme_mono),
        ("cfg_theme_default", |m| m.cfg_theme_default),
        ("overview_health_label", |m| m.overview_health_label),
        ("overview_cycles_suffix", |m| m.overview_cycles_suffix),
        ("overview_battery_extra_label", |m| {
            m.overview_battery_extra_label
        }),
        ("overview_desktop_na", |m| m.overview_desktop_na),
        ("wifi_auth_title", |m| m.wifi_auth_title),
        ("sudo_auth_title", |m| m.sudo_auth_title),
        ("sudo_label_operation", |m| m.sudo_label_operation),
        ("analyzer_title", |m| m.analyzer_title),
        ("analyzer_hint_nav", |m| m.analyzer_hint_nav),
        ("display_status_disabled", |m| m.display_status_disabled),
        ("display_status_disconnected", |m| {
            m.display_status_disconnected
        }),
        ("storage_err_format_generic_prefix", |m| {
            m.storage_err_format_generic_prefix
        }),
    ]
}

#[test]
fn new_message_fields_are_non_empty_in_all_languages() {
    for lang in [Language::PtBr, Language::EnUs, Language::EsEs] {
        let m = lang.messages();
        for (name, get) in new_message_fields() {
            assert!(!get(m).trim().is_empty(), "{name} is empty for {lang:?}");
        }
    }
}

#[test]
fn new_message_fields_are_actually_translated_not_copy_pasted() {
    let pt = Language::PtBr.messages();
    let en = Language::EnUs.messages();
    let es = Language::EsEs.messages();

    let mut identical_to_pt_in_en = 0usize;
    let mut identical_to_pt_in_es = 0usize;
    let fields = new_message_fields();
    let total = fields.len();

    for (_, get) in &fields {
        if get(pt) == get(en) {
            identical_to_pt_in_en += 1;
        }
        if get(pt) == get(es) {
            identical_to_pt_in_es += 1;
        }
    }

    // A handful of short technical strings ("SISTEMA"-alike, bracket tags,
    // etc.) legitimately coincide across languages, but the overwhelming
    // majority must differ, otherwise a language dropped through to the
    // Portuguese default (the exact bug this suite guards against).
    assert!(
        identical_to_pt_in_en < total / 3,
        "too many EN-US fields identical to pt-BR: {identical_to_pt_in_en}/{total}"
    );
    assert!(
        identical_to_pt_in_es < total / 3,
        "too many es-ES fields identical to pt-BR: {identical_to_pt_in_es}/{total}"
    );
}

#[test]
fn display_layout_mode_titles_in_all_languages() {
    assert_eq!(
        DisplayLayoutMode::ExtendRight.title_in(Language::PtBr.messages()),
        "Expandir à Direita"
    );
    assert_eq!(
        DisplayLayoutMode::ExtendRight.title_in(Language::EnUs.messages()),
        "Extend Right"
    );
    assert_eq!(
        DisplayLayoutMode::ExtendRight.title_in(Language::EsEs.messages()),
        "Extender a la Derecha"
    );
}

#[test]
fn shared_lang_reflects_updates_across_clones() {
    let shared = SharedLang::new(Language::PtBr);
    let clone = shared.clone();

    assert_eq!(shared.get(), Language::PtBr);
    assert_eq!(clone.get(), Language::PtBr);

    shared.set(Language::EnUs);

    // The clone observes the update: it's the same underlying atomic, which
    // is exactly what lets independent backend tasks pick up a language
    // change made in the config modal.
    assert_eq!(clone.get(), Language::EnUs);
    assert_eq!(shared.messages().tab_network, "Network");
}

#[test]
fn render_config_modal_and_help_in_all_languages_without_panic() {
    for lang in [Language::PtBr, Language::EnUs, Language::EsEs] {
        let mut cfg = Config::default();
        cfg.ui.language = lang.code().to_string();
        let mut app = App::new(cfg);
        app.show_config = true;
        app.show_help = true;

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
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
