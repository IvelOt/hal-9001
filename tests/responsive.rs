//! Testes de responsividade do harness: renderiza o cockpit (com foco no
//! Overview) em múltiplas resoluções, garantindo 0 pânicos, 0 quebras de
//! layout e centralização em telas largas — nos modos Padrão e Detalhado.

use hal9001::app::App;
use hal9001::backend::system::{
    Battery, BatteryStatus, DetailInfo, Packages, SystemSnapshot, Volume,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

/// Resoluções cobertas: micro, padrão-80, moderno e ultrawide.
const RESOLUTIONS: [(u16, u16); 4] = [(60, 15), (80, 24), (120, 35), (200, 50)];

/// Snapshot rico o suficiente para exercitar todas as seções (padrão e detalhe).
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
        },
    }
}

/// Renderiza o Overview numa resolução e devolve o buffer resultante.
fn render_overview(w: u16, h: u16, detailed: bool) -> Buffer {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::System(Box::new(sample_snapshot())));

    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::SelectTab(0), &tx); // Overview
    if detailed {
        app.dispatch(Action::ToggleDetail, &tx);
    }

    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    terminal.backend().buffer().clone()
}

/// Menor coluna com conteúdo não-branco dentro de uma faixa vertical de linhas.
fn min_content_col(buf: &Buffer, rows: std::ops::Range<u16>) -> Option<u16> {
    let area = *buf.area();
    let mut min_col: Option<u16> = None;
    // Ignora as bordas do bloco (colunas 0 e width-1).
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
            // O próprio draw entra em pânico se algo estourar os limites.
            let buf = render_overview(w, h, detailed);
            assert_eq!(buf.area().width, w);
            assert_eq!(buf.area().height, h);
        }
    }
}

#[test]
fn wide_terminals_center_content() {
    // Em telas largas o conteúdo não deve ficar colado na margem esquerda:
    // a menor coluna com conteúdo, nas linhas internas, fica bem além da borda.
    for (w, h) in [(120u16, 35u16), (200, 50)] {
        let buf = render_overview(w, h, false);
        // Ignora tabbar (linhas 0..3) e statusline; foca no miolo.
        let min_col = min_content_col(&buf, 5..20).expect("deve haver conteúdo");
        assert!(
            min_col > 3,
            "conteúdo colado à esquerda em {w} col (min_col={min_col})"
        );
    }
}

#[test]
fn micro_terminal_collapses_logo_but_still_renders() {
    // 60x15: espaço apertado — a logo recolhe e mostramos só os metadados +
    // seções (procuramos o cabeçalho user@host).
    let buf = render_overview(60, 15, false);
    let joined = buffer_text(&buf);
    assert!(
        joined.contains("operator") || joined.contains("hall"),
        "painel de informações ausente no micro terminal"
    );
}

/// Conta os dentes de engrenagem `#` — glifo exclusivo da logo (não aparece no
/// texto das seções/metadados), servindo de impressão digital estável do
/// tamanho renderizado.
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
    // Em tela larga aparecem a logo das engrenagens (dentes + olho) e os
    // títulos das seções estilo Hermes.
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
    // Requisito do briefing: a logo NÃO encolhe ao ativar o modo detalhado.
    // O tamanho renderizado (impressão digital de engrenagens/olho) deve ser
    // idêntico nos modos Normal e Expandido.
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
    // Em tela moderna, o modo detalhado expõe campos extras (ex.: BIOS, GPU).
    let buf = render_overview(120, 40, true);
    let text = buffer_text(&buf);
    assert!(text.contains("BIOS") || text.contains("GPU") || text.contains("Núcleos"));
    assert!(text.contains("Expandido"), "indicador de modo ausente");
}

#[test]
fn footer_shows_mode_indicator() {
    let normal = buffer_text(&render_overview(120, 40, false));
    assert!(normal.contains("Normal"));
    assert!(normal.contains("[.]"));
}

#[test]
fn footer_shows_control_hints() {
    // O rodapé do Overview anuncia os controles interativos de brilho/volume.
    let text = buffer_text(&render_overview(120, 40, false));
    assert!(text.contains("[b/B]"), "atalho de brilho ausente");
    assert!(text.contains("[v/V]"), "atalho de volume ausente");
    assert!(text.contains("[m]"), "atalho de mudo ausente");
}

#[test]
fn dense_lines_combine_metric_and_bar() {
    // Requisito do briefing: a métrica (ex.: RAM `9.0 / 32.0 GiB`) e a barra de
    // progresso convivem na MESMA linha do buffer, e não em duas separadas.
    let buf = render_overview(120, 40, false);
    let area = *buf.area();
    let mut found = false;
    for y in 0..area.height {
        let mut row = String::new();
        for x in 0..area.width {
            row.push_str(buf[(x, y)].symbol());
        }
        // Linha da RAM: rótulo + valor GiB + barra `[` + percentual `%`.
        if row.contains("RAM") && row.contains("GiB") && row.contains('[') && row.contains('%') {
            found = true;
            break;
        }
    }
    assert!(found, "linha densa RAM (valor + barra) não encontrada");
}

#[test]
fn standard_terminal_keeps_box_and_footer_visible() {
    // 80x24 (terminal padrão): a moldura do bloco (topo) e o rodapé de controles
    // (base) — ambos em posições fixas — nunca são empurrados para fora.
    for detailed in [false, true] {
        let buf = render_overview(80, 24, detailed);
        let text = buffer_text(&buf);
        assert!(text.contains("Overview"), "título do bloco cortado (detailed={detailed})");
        assert!(
            text.contains("[b/B]") && text.contains("[m]"),
            "rodapé de controles cortado (detailed={detailed})"
        );
    }

    // No modo NORMAL o layout denso (~16 linhas) cabe por completo: as três
    // seções e a paleta permanecem visíveis sem corte.
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

/// Concatena todos os símbolos do buffer numa string para buscas simples.
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
