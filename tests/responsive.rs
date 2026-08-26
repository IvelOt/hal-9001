
use hal9001::app::App;
use hal9001::backend::system::{
    Battery, BatteryStatus, DetailInfo, Packages, PowerProfile, SystemSnapshot, Volume,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

const RESOLUTIONS: [(u16, u16); 4] = [(60, 15), (80, 24), (120, 35), (200, 50)];

fn sample_snapshot() -> SystemSnapshot {
    SystemSnapshot {
        host: "hall".into(),
        user: "operator".into(),
        shell: "zsh".into(),
        os: "Arch Linux".into(),
        kernel: "6.18.43-1-lts".into(),
        uptime_secs: 3 * 86_400 + 4 * 3_600 + 12 * 60,
        cpu_name: "AMD Ryzen 7 5800X 8-Core Processor".into(),
        cpu_usage: 37.0,
        mem_used: 9 * 1024 * 1024 * 1024,
        mem_total: 32 * 1024 * 1024 * 1024,
        host_model: Some("ThinkPad X1 Carbon Gen 11".into()),
        packages: Some(Packages {
            total: 1560,
            by_manager: vec![("pacman", 1500), ("flatpak", 60)],
            pending_updates: Some(12),
        }),
        brightness: Some(0.6),
        volume: Some(Volume {
            level: 0.42,
            muted: false,
        }),
        battery: Some(Battery {
            percent: 76.0,
            status: BatteryStatus::Discharging,
            power_watts: Some(14.0),
            health: Some(0.88),
            cycle_count: Some(212),
            technology: Some("Li-poly".into()),
        }),
        disk_used: Some(220 * 1024 * 1024 * 1024),
        disk_total: Some(512 * 1024 * 1024 * 1024),
        kbd_backlight: Some(0.5),
        power_profile: Some(PowerProfile::Performance),
        detail: DetailInfo {
            board_vendor: Some("LENOVO".into()),
            board_name: Some("21HM".into()),
            bios_version: Some("N3AET50W".into()),
            bios_date: Some("06/12/2023".into()),
            gpu: Some("Intel Corporation Raptor Lake-P [Iris Xe Graphics]".into()),
            cpu_arch: Some("x86_64".into()),
            cpu_cores_physical: Some(8),
            cpu_cores_logical: 16,
            cpu_freq_ghz: Some(3.80),
            cpu_temp_c: Some(48.0),
            swap_used: 1024 * 1024 * 1024,
            swap_total: 8 * 1024 * 1024 * 1024,
            desktop: Some("sway".into()),
            session_type: Some("wayland".into()),
            top_processes: vec![
                hal9001::backend::system::ProcessInfo {
                    pid: 1420,
                    name: "firefox".into(),
                    cpu_usage: 18.5,
                    mem_bytes: 2 * 1024 * 1024 * 1024,
                },
                hal9001::backend::system::ProcessInfo {
                    pid: 8932,
                    name: "rust-analyzer".into(),
                    cpu_usage: 12.0,
                    mem_bytes: 1400 * 1024 * 1024,
                },
                hal9001::backend::system::ProcessInfo {
                    pid: 4511,
                    name: "discord".into(),
                    cpu_usage: 4.2,
                    mem_bytes: 850 * 1024 * 1024,
                },
                hal9001::backend::system::ProcessInfo {
                    pid: 1205,
                    name: "cargo".into(),
                    cpu_usage: 3.1,
                    mem_bytes: 420 * 1024 * 1024,
                },
                hal9001::backend::system::ProcessInfo {
                    pid: 980,
                    name: "kitty".into(),
                    cpu_usage: 1.0,
                    mem_bytes: 180 * 1024 * 1024,
                },
            ],
        },
    }
}

fn render_overview(w: u16, h: u16, detailed: bool) -> Buffer {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::System(Box::new(sample_snapshot())));

    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::SelectTab(0), &tx);
    if detailed {
        app.dispatch(Action::ToggleDetail, &tx);
    }

    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    terminal.backend().buffer().clone()
}

fn min_content_col(buf: &Buffer, rows: std::ops::Range<u16>) -> Option<u16> {
    let area = *buf.area();
    let mut min_col: Option<u16> = None;

    for y in rows {
        if y >= area.height {
            break;
        }
        for x in 1..area.width.saturating_sub(1) {
            if buf[(x, y)].symbol().trim().is_empty() {
                continue;
            }
            min_col = Some(min_col.map_or(x, |m| m.min(x)));
            break;
        }
    }
    min_col
}

#[test]
fn renders_every_resolution_without_panic() {
    for (w, h) in RESOLUTIONS {
        for detailed in [false, true] {

            let buf = render_overview(w, h, detailed);
            assert_eq!(buf.area().width, w);
            assert_eq!(buf.area().height, h);
        }
    }
}

#[test]
fn wide_terminals_center_content() {

    for (w, h) in [(120u16, 35u16), (200, 50)] {
        let buf = render_overview(w, h, false);

        let min_col = min_content_col(&buf, 5..20).expect("deve haver conteúdo");
        assert!(
            min_col > 3,
            "conteúdo colado à esquerda em {w} col (min_col={min_col})"
        );
    }
}

#[test]
fn micro_terminal_collapses_logo_but_still_renders() {

    let buf = render_overview(60, 15, false);
    let joined = buffer_text(&buf);
    assert!(
        joined.contains("operator") || joined.contains("hall"),
        "painel de informações ausente no micro terminal"
    );
}

#[test]
fn portrait_mobile_terminal_renders_centered_logo_and_vertical_stack() {

    for (w, h) in [(45, 80), (40, 70), (50, 60)] {
        let buf = render_overview(w, h, false);
        let text = buffer_text(&buf);

        assert!(
            logo_gears(&buf) > 0,
            "logo ASCII compacta deveria estar presente no topo em {w}x{h}"
        );

        assert!(text.contains("HAL-9001"), "cabeçalho HAL-9001 ausente em {w}x{h}");
        assert!(text.contains("operator"), "usuário ausente em {w}x{h}");
        assert!(text.contains("RAM"), "métrica RAM ausente em {w}x{h}");
    }

    let small_buf = render_overview(25, 20, false);
    assert_eq!(logo_gears(&small_buf), 0);

    let buf = render_overview(48, 50, false);
    let mut ansi_out = String::new();
    let area = *buf.area();
    for y in 0..area.height {
        for x in 0..area.width {
            ansi_out.push_str(buf[(x, y)].symbol());
        }
        ansi_out.push('\n');
    }
    let _ = std::fs::write("/tmp/hall9001_mobile_portrait.ansi", ansi_out);
}

fn logo_gears(buf: &Buffer) -> usize {
    let area = *buf.area();
    let mut gears = 0usize;
    for y in 0..area.height {
        for x in 0..area.width {
            if buf[(x, y)].symbol() == "#" {
                gears += 1;
            }
        }
    }
    gears
}

#[test]
fn wide_terminal_renders_gear_logo_and_sections() {

    let buf = render_overview(120, 40, false);
    assert!(logo_gears(&buf) > 0, "dentes de engrenagem ausentes");
    assert!(buffer_text(&buf).contains('O'), "olho do HAL ausente");

    let text = buffer_text(&buf);
    for title in [
        "AVAILABLE COMPUTE",
        "SYSTEM & PLATFORM",
        "PERIPHERALS & POWER",
        "COLOR PALETTE",
    ] {
        assert!(text.contains(title), "seção ausente: {title}");
    }
}

#[test]
fn logo_does_not_shrink_in_detailed_mode() {

    for (w, h) in [(120u16, 40u16), (200, 50)] {
        let normal = logo_gears(&render_overview(w, h, false));
        let detailed = logo_gears(&render_overview(w, h, true));
        assert!(normal > 0, "logo ausente em {w}x{h}");
        assert_eq!(
            normal, detailed,
            "logo mudou de tamanho ao expandir em {w}x{h} (normal={normal}, detalhe={detailed})"
        );
    }
}

#[test]
fn detailed_mode_shows_extra_fields_when_space_allows() {

    let buf = render_overview(110, 42, true);
    let text = buffer_text(&buf);
    assert!(text.contains("BIOS") || text.contains("GPU") || text.contains("Núcleos") || text.contains("TOP PROCESSES"));
    assert!(text.contains("Expandido") || text.contains("Detalhes"), "indicador de modo ausente");

    let mut ansi_out = String::new();
    let area = *buf.area();
    for y in 0..area.height {
        for x in 0..area.width {
            ansi_out.push_str(buf[(x, y)].symbol());
        }
        ansi_out.push('\n');
    }
    let _ = std::fs::write("/tmp/hall9001_tab1_overview_detailed.ansi", ansi_out);
}

#[test]
fn window_header_uses_clean_project_name() {

    let text = buffer_text(&render_overview(120, 40, false));
    assert!(
        text.contains("HAL-9001"),
        "cabeçalho sem o nome do projeto"
    );
    assert!(
        !text.to_lowercase().contains("cockpit"),
        "termo 'cockpit' ainda presente na UI"
    );
}

#[test]
fn battery_status_renders_before_bar_on_its_line() {

    let buf = render_overview(120, 40, false);
    let area = *buf.area();
    let mut checked = false;
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        if row.contains("Bateria") && row.contains("DISCHARGING") {
            let status_at = row.find("DISCHARGING").unwrap();

            let bar_at = row.find('█').or_else(|| row.find('░')).unwrap();
            assert!(status_at < bar_at, "status deveria preceder a barra");
            checked = true;
            break;
        }
    }
    assert!(checked, "linha da bateria com status não encontrada");
}

#[test]
fn footer_shows_mode_indicator() {
    let normal = buffer_text(&render_overview(120, 40, false));
    assert!(normal.contains("Normal"));
    assert!(normal.contains("[.]"));
}

#[test]
fn footer_shows_control_hints() {

    let text = buffer_text(&render_overview(120, 40, false));
    assert!(text.contains("[b/B]"), "atalho de brilho ausente");
    assert!(text.contains("[v/V]"), "atalho de volume ausente");
    assert!(text.contains("[m]"), "atalho de mudo ausente");
}

#[test]
fn footer_shows_power_profile_hint() {

    let text = buffer_text(&render_overview(120, 40, false));
    assert!(text.contains("[p]"), "atalho de perfil ausente");
    assert!(text.contains("Perfil"), "rótulo de perfil ausente no rodapé");
}

#[test]
fn power_profile_row_renders_active_tag() {

    let text = buffer_text(&render_overview(120, 40, false));
    assert!(
        text.contains("[PERFORMANCE]"),
        "tag do perfil de energia ausente"
    );
}

#[test]
fn dense_lines_combine_metric_and_bar() {

    let buf = render_overview(120, 40, false);
    let area = *buf.area();
    let mut found = false;
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buf[(x, y)].symbol());
        }

        if row.contains("RAM") && row.contains("GiB") && row.contains('[') && row.contains('%') {
            found = true;
            break;
        }
    }
    assert!(found, "linha densa RAM (valor + barra) não encontrada");
}

#[test]
fn standard_terminal_keeps_box_and_footer_visible() {

    for detailed in [false, true] {
        let buf = render_overview(80, 24, detailed);
        let text = buffer_text(&buf);
        assert!(text.contains("Overview"), "título do bloco cortado (detailed={detailed})");
        assert!(
            text.contains("[b/B]") && text.contains("[m]"),
            "rodapé de controles cortado (detailed={detailed})"
        );
    }

    let text = buffer_text(&render_overview(80, 24, false));
    for title in [
        "AVAILABLE COMPUTE",
        "SYSTEM & PLATFORM",
        "PERIPHERALS & POWER",
        "COLOR PALETTE",
    ] {
        assert!(text.contains(title), "seção '{title}' cortada no modo normal");
    }
}

fn buffer_text(buf: &Buffer) -> String {
    let area = *buf.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
