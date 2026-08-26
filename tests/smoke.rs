//! Testes de fumaça: exercitam lógica pura sem entrar no alt-screen.

use hal9001::app::{App, Phase, Tab};
use hal9001::config::Config;
use hal9001::ui::theme::Palette;
use hal9001::ui::widgets::{human_bytes, human_uptime, metric_line, truncate_str};
use ratatui::text::Line;
use unicode_width::UnicodeWidthStr;

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
                pending_updates: None,
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
                health: Some(0.91),
                cycle_count: Some(142),
                technology: Some("Li-poly".into()),
            }),
            disk_used: Some(120 * 1024 * 1024 * 1024),
            disk_total: Some(512 * 1024 * 1024 * 1024),
            kbd_backlight: Some(0.7),
            power_profile: Some(hal9001::backend::system::PowerProfile::Balanced),
            detail: hal9001::backend::system::DetailInfo {
                board_vendor: Some("ACME".into()),
                board_name: Some("X570".into()),
                bios_version: Some("F35".into()),
                bios_date: Some("2023-05-01".into()),
                gpu: Some("Intel UHD Graphics".into()),
                cpu_arch: Some("x86_64".into()),
                cpu_cores_physical: Some(8),
                cpu_cores_logical: 16,
                cpu_freq_ghz: Some(3.4),
                cpu_temp_c: Some(52.0),
                swap_used: 512 * 1024 * 1024,
                swap_total: 2 * 1024 * 1024 * 1024,
                desktop: Some("sway".into()),
                session_type: Some("wayland".into()),
                top_processes: Vec::new(),
            },
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

    // Modo detalhado do Overview (tecla `.`) também renderiza sem pânico.
    app.dispatch(hal9001::events::Action::ToggleHelp, &tx); // fecha ajuda
    app.dispatch(hal9001::events::Action::SelectTab(0), &tx);
    app.dispatch(hal9001::events::Action::ToggleDetail, &tx);
    assert!(app.detailed_overview);
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
            kbd_backlight: None,
            power_profile: None,
            detail: hal9001::backend::system::DetailInfo::default(),
        },
    )));

    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

    // Também no modo detalhado (degradado: sem swap/bateria/DMI).
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(hal9001::events::Action::ToggleDetail, &tx);
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}

#[test]
fn human_helpers_format() {
    assert_eq!(human_bytes(512), "512 B");
    assert_eq!(human_bytes(1024), "1.0 KiB");
    assert_eq!(human_uptime(90_061), "1d 1h 1m");
}

#[test]
fn truncate_str_adds_ellipsis_and_respects_width() {
    // Cabe: devolvido sem alteração.
    assert_eq!(truncate_str("curto", 10), "curto");
    assert_eq!(truncate_str("exato", 5), "exato");
    // Não cabe: corta e anexa reticência (largura final == max).
    let t = truncate_str("Intel Corporation Raptor Lake-P", 12);
    assert_eq!(t.chars().count(), 12);
    assert!(t.ends_with('…'));
    assert!(t.starts_with("Intel"));
    // Degradação em larguras minúsculas.
    assert_eq!(truncate_str("qualquer", 1), "…");
    assert_eq!(truncate_str("qualquer", 0), "");
}

/// Coluna (0-based) onde a barra de progresso `[` começa numa `metric_line`,
/// somando a largura de todos os spans anteriores ao span do colchete de abertura
/// da barra (o único span cujo conteúdo é exatamente `"["`).
fn gauge_col(line: &Line) -> usize {
    let mut col = 0usize;
    for span in &line.spans {
        if span.content == "[" {
            return col;
        }
        col += UnicodeWidthStr::width(span.content.as_ref());
    }
    panic!("barra não encontrada na linha");
}

#[test]
fn metric_bars_align_regardless_of_status_suffix() {
    // Requisito do briefing: o status ([CHARGING +25W], [MUTED]) fica ENTRE o
    // rótulo/valor e a barra, e as barras ficam alinhadas verticalmente entre
    // linhas com valores/status de larguras diferentes.
    let pal = Palette::from_config(&Config::default());
    let bateria = metric_line("Bateria", "", 18, 0.65, 12, &pal, Some("[CHARGING +25W]"));
    let volume = metric_line("Volume", "", 18, 0.80, 12, &pal, Some("[MUTED]"));
    let brilho = metric_line("Brilho", "", 18, 1.0, 12, &pal, None);
    let ram = metric_line("RAM", "6.0 / 15.3 GiB", 18, 0.39, 12, &pal, None);

    let base = gauge_col(&bateria);
    for (name, line) in [
        ("Volume", &volume),
        ("Brilho", &brilho),
        ("RAM", &ram),
    ] {
        assert_eq!(gauge_col(line), base, "barra desalinhada em {name}");
    }

    // O status aparece antes da barra (coluna do status < coluna da barra).
    let text: String = bateria.spans.iter().map(|s| s.content.as_ref()).collect();
    let status_at = text.find("[CHARGING").unwrap();
    let bar_at = text.rfind('[').unwrap();
    assert!(status_at < bar_at, "status deveria preceder a barra");
}

#[test]
fn brightness_and_volume_actions_dispatch_without_panic() {
    use hal9001::events::Action;

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let (tx, mut rx) = tokio::sync::broadcast::channel(16);
    let mut app = App::new(cfg);

    // As ações de controle são repassadas aos backends (broadcast), sem mutar
    // o estado da UI diretamente.
    for action in [
        Action::BrightnessUp,
        Action::BrightnessDown,
        Action::VolumeUp,
        Action::VolumeDown,
        Action::ToggleMute,
        Action::CyclePowerProfile,
    ] {
        app.dispatch(action, &tx);
    }

    // Seis ações devem ter sido difundidas.
    let mut count = 0;
    while rx.try_recv().is_ok() {
        count += 1;
    }
    assert_eq!(count, 6);
}

#[test]
fn all_theme_palettes_build_and_render_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let themes = [
        "hal",
        "catppuccin",
        "tokyo-night",
        "nord",
        "gruvbox",
        "cyberpunk",
        "dracula",
        "mono",
    ];

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).unwrap();

    for theme_name in themes {
        let mut cfg = Config::default();
        cfg.splash.enabled = false;
        cfg.theme.name = theme_name.to_string();

        let pal = Palette::from_config(&cfg);
        assert_ne!(pal.accent, ratatui::style::Color::Reset);

        let app = App::new(cfg);
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
}

#[test]
fn config_modal_navigation_and_theme_cycling() {
    use hal9001::events::Action;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let (tx, _rx) = tokio::sync::broadcast::channel(16);
    let mut app = App::new(cfg);

    // Abre o modal de configurações
    app.dispatch(Action::ToggleConfig, &tx);
    assert!(app.show_config);
    assert_eq!(app.config_cursor, 0);

    // Navega para baixo -> Campo 1 (Tema)
    app.dispatch(Action::Down, &tx);
    assert_eq!(app.config_cursor, 1);

    // Cicla tema para a direita: hal -> catppuccin -> tokyo-night -> etc.
    app.dispatch(Action::Right, &tx);
    assert_eq!(app.config.theme.name, "catppuccin");

    app.dispatch(Action::Right, &tx);
    assert_eq!(app.config.theme.name, "tokyo-night");

    // Cicla para trás
    app.dispatch(Action::Left, &tx);
    assert_eq!(app.config.theme.name, "catppuccin");

    // Navega para o campo 6 (Polling)
    app.dispatch(Action::Up, &tx);
    assert_eq!(app.config_cursor, 0); // de 1 para 0
    app.dispatch(Action::Up, &tx);
    assert_eq!(app.config_cursor, 6); // wrap para 6

    // Cicla perfil de polling para a direita (Fast)
    app.dispatch(Action::Right, &tx);
    assert_eq!(app.config.polling.system_ms, 750);

    // Salva configuração
    app.dispatch(Action::SaveConfig, &tx);
    assert!(app.toast.is_some());

    // Renderiza frame com o modal aberto para screenshot
    let backend = TestBackend::new(100, 26);
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
    let _ = std::fs::write("/tmp/hall9001_config_modal.ansi", ansi_out);

    // Fecha o modal
    app.dispatch(Action::ToggleConfig, &tx);
    assert!(!app.show_config);
}

