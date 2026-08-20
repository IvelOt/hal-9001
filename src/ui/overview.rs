//! Aba 1 — Overview (estética neofetch): besouro à esquerda, resumo à direita.
//!
//! O conteúdo (coluna do besouro + coluna de informações) é centralizado
//! harmoniosamente na área disponível via layouts flexíveis (`Flex::Center`),
//! evitando que fique colado na margem esquerda em telas largas. Em telas
//! estreitas a art do besouro é reduzida (A→B) ou recolhida, mantendo apenas o
//! painel de informações centralizado. A tecla `.` alterna o modo Detalhado.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::ascii;
use crate::backend::system::{DetailInfo, SystemSnapshot};

use super::theme::Palette;
use super::widgets::{bar_line, bar_line_suffix, human_bytes, human_uptime, kv_line, palette_line};

/// Folga horizontal entre a coluna do besouro e a de informações.
const GAP: u16 = 4;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let block = super::content_block("Overview", pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Reserva a última linha para a statusline do Overview (indicador do modo).
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    draw_center(app, pal, f, rows[0]);
    draw_footer(app, pal, f, rows[1]);
}

/// Centraliza besouro + informações na área, escolhendo a art conforme o
/// espaço livre e recolhendo-a em telas estreitas.
fn draw_center(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // Monta as linhas de informação e mede sua largura.
    let info = build_info(app, pal, area.width);
    let info_w = info
        .iter()
        .map(line_width)
        .max()
        .unwrap_or(0)
        .min(area.width as usize) as u16;

    // Largura livre para a coluna do besouro após reservar o painel + folga.
    let avail = area.width.saturating_sub(info_w + GAP);
    let art = ascii::select(&app.config.overview.ascii, avail);

    let (art_w, art_h) = art
        .map(|a| (ascii::art_width(a) as u16, ascii::art_height(a) as u16))
        .unwrap_or((0, 0));

    let content_h = art_h.max(info.len() as u16).min(area.height).max(1);

    // Faixa vertical centralizada de altura `content_h`.
    let vband = Layout::vertical([Constraint::Length(content_h)])
        .flex(Flex::Center)
        .split(area)[0];

    match art {
        Some(art) if art_w > 0 && art_w + GAP + info_w <= area.width => {
            // Duas colunas centralizadas como um bloco.
            let cols = Layout::horizontal([
                Constraint::Length(art_w),
                Constraint::Length(GAP),
                Constraint::Length(info_w),
            ])
            .flex(Flex::Center)
            .split(vband);

            draw_beetle(app, pal, f, cols[0], art);
            f.render_widget(Paragraph::new(Text::from(info)), cols[2]);
        }
        _ => {
            // Sem espaço para o besouro: só o painel, centralizado.
            let col = Layout::horizontal([Constraint::Length(info_w)])
                .flex(Flex::Center)
                .split(vband)[0];
            f.render_widget(Paragraph::new(Text::from(info)), col);
        }
    }
}

/// Statusline interna do Overview com o indicador do modo de detalhe.
fn draw_footer(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let mode = if app.detailed_overview {
        "Expandido"
    } else {
        "Normal"
    };
    let line = Line::from(vec![
        Span::styled(" [.] ", Style::default().fg(pal.accent)),
        Span::styled("Detalhes: ", Style::default().fg(pal.dim)),
        Span::styled(mode, Style::default().fg(pal.fg)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_beetle(app: &App, pal: &Palette, f: &mut Frame, area: Rect, art: &str) {
    let _ = app;
    let rows: Vec<&str> = art.lines().collect();
    let n = rows.len().max(1);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let t = i as f64 / n as f64;
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(pal.gradient(t)),
            ))
        })
        .collect();
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// Monta as linhas do painel de informações a partir do estado atual.
fn build_info<'a>(app: &App, pal: &Palette, width: u16) -> Vec<Line<'a>> {
    let bar_w = (width as usize / 4).clamp(6, 20);
    let mut lines: Vec<Line> = Vec::new();

    match &app.system {
        Some(s) => {
            info_lines(s, pal, bar_w, width, &mut lines);
            if app.detailed_overview {
                detail_lines(&s.detail, s, pal, bar_w, &mut lines);
            }
        }
        None => lines.push(Line::from(Span::styled(
            "coletando dados do sistema…",
            Style::default().fg(pal.dim),
        ))),
    }

    // A paleta ocupa espaço vertical; no modo detalhado é omitida.
    if !app.detailed_overview {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Paleta:",
            Style::default().fg(pal.dim),
        )));
        lines.push(palette_line());
    }

    lines
}

/// Linhas do modo **Padrão**.
fn info_lines(s: &SystemSnapshot, pal: &Palette, bar_w: usize, width: u16, out: &mut Vec<Line>) {
    // Cabeçalho user@host + régua.
    out.push(Line::from(vec![
        Span::styled(s.user.clone(), Style::default().fg(pal.accent)),
        Span::styled("@", Style::default().fg(pal.dim)),
        Span::styled(s.host.clone(), Style::default().fg(pal.accent)),
    ]));
    out.push(Line::from(Span::styled(
        "─".repeat((s.user.len() + s.host.len() + 1).min(width as usize)),
        Style::default().fg(pal.dim),
    )));

    out.push(kv_line("OS", s.os.clone(), pal));
    if let Some(model) = &s.host_model {
        out.push(kv_line("Host", model.clone(), pal));
    }
    out.push(kv_line("Kernel", s.kernel.clone(), pal));
    out.push(kv_line("Uptime", human_uptime(s.uptime_secs), pal));
    out.push(kv_line(
        "Pacotes",
        s.packages
            .as_ref()
            .map(|p| p.summary())
            .unwrap_or_else(|| "N/A".into()),
        pal,
    ));
    out.push(kv_line("Shell", s.shell.clone(), pal));

    // CPU + barra.
    out.push(kv_line("CPU", s.cpu_name.clone(), pal));
    out.push(bar_line("Uso CPU", s.cpu_ratio(), bar_w, pal));

    // RAM + barra.
    out.push(kv_line(
        "RAM",
        format!("{} / {}", human_bytes(s.mem_used), human_bytes(s.mem_total)),
        pal,
    ));
    out.push(bar_line("Mem", s.mem_ratio(), bar_w, pal));

    // Bateria (ou N/A em desktop) — sufixo neofetch: [CHARGING +18W].
    match &s.battery {
        Some(b) => {
            let suffix = battery_suffix(b);
            out.push(bar_line_suffix("Bateria", b.ratio(), bar_w, pal, &suffix));
        }
        None => out.push(kv_line("Bateria", "N/A (Desktop)".into(), pal)),
    }

    // Disco raiz.
    match (s.disk_ratio(), s.disk_used, s.disk_total) {
        (Some(r), Some(u), Some(t)) => {
            out.push(kv_line(
                "Disco /",
                format!("{} / {}", human_bytes(u), human_bytes(t)),
                pal,
            ));
            out.push(bar_line("Disco", r, bar_w, pal));
        }
        _ => out.push(kv_line("Disco /", "N/A".into(), pal)),
    }

    // Brilho.
    match s.brightness {
        Some(r) => out.push(bar_line("Brilho", r, bar_w, pal)),
        None => out.push(kv_line("Brilho", "N/A".into(), pal)),
    }

    // Volume — sem emojis; [MUTED] apenas quando mudo.
    match &s.volume {
        Some(v) if v.muted => {
            out.push(bar_line_suffix("Volume", v.ratio(), bar_w, pal, "[MUTED]"));
        }
        Some(v) => out.push(bar_line("Volume", v.ratio(), bar_w, pal)),
        None => out.push(kv_line("Volume", "N/A".into(), pal)),
    }
}

/// Sufixo textual de bateria no estilo neofetch clássico, sem emojis:
/// `[CHARGING +18W]`, `[DISCHARGING -14W]`, `[FULL]`.
fn battery_suffix(b: &crate::backend::system::Battery) -> String {
    match (b.power_watts, b.status.power_sign()) {
        (Some(w), sign) if !sign.is_empty() => {
            format!("[{} {}{:.0}W]", b.status.tag(), sign, w)
        }
        _ => format!("[{}]", b.status.tag()),
    }
}

/// Linhas extras do modo **Detalhado** (tecla `.`).
fn detail_lines(
    d: &DetailInfo,
    s: &SystemSnapshot,
    pal: &Palette,
    bar_w: usize,
    out: &mut Vec<Line>,
) {
    out.push(Line::from(""));
    out.push(Line::from(Span::styled(
        "── Detalhes ──",
        Style::default().fg(pal.dim),
    )));

    // Placa-mãe & BIOS.
    if let Some(board) = join_opt(d.board_vendor.as_deref(), d.board_name.as_deref()) {
        out.push(kv_line("Placa", board, pal));
    }
    match (&d.bios_version, &d.bios_date) {
        (Some(v), Some(dt)) => out.push(kv_line("BIOS", format!("{v} ({dt})"), pal)),
        (Some(v), None) => out.push(kv_line("BIOS", v.clone(), pal)),
        _ => {}
    }

    // GPU.
    if let Some(gpu) = &d.gpu {
        out.push(kv_line("GPU", gpu.clone(), pal));
    }

    // CPU detalhada: núcleos, frequência, arquitetura, temperatura.
    let cores = match d.cpu_cores_physical {
        Some(p) => format!("{p}c / {}t", d.cpu_cores_logical),
        None => format!("{}t", d.cpu_cores_logical),
    };
    let mut cpu_det = cores;
    if let Some(f) = d.cpu_freq_ghz {
        cpu_det.push_str(&format!(" @ {f:.2} GHz"));
    }
    if let Some(arch) = &d.cpu_arch {
        cpu_det.push_str(&format!(" · {arch}"));
    }
    out.push(kv_line("Núcleos", cpu_det, pal));
    if let Some(t) = d.cpu_temp_c {
        out.push(kv_line("Temp CPU", format!("{t:.0} °C"), pal));
    }

    // Saúde da bateria.
    if let Some(b) = &s.battery {
        let mut parts: Vec<String> = Vec::new();
        if let Some(h) = b.health {
            parts.push(format!("saúde {:.0}%", h * 100.0));
        }
        if let Some(c) = b.cycle_count {
            parts.push(format!("{c} ciclos"));
        }
        if let Some(tech) = &b.technology {
            parts.push(tech.clone());
        }
        if !parts.is_empty() {
            out.push(kv_line("Bateria+", parts.join(" · "), pal));
        }
    }

    // Memória virtual / swap.
    if d.swap_total > 0 {
        out.push(kv_line(
            "Swap",
            format!(
                "{} / {}",
                human_bytes(d.swap_used),
                human_bytes(d.swap_total)
            ),
            pal,
        ));
        out.push(bar_line("Swap", d.swap_ratio(), bar_w, pal));
    } else {
        out.push(kv_line("Swap", "N/A".into(), pal));
    }

    // Desktop / Window Manager.
    if let Some(de) = &d.desktop {
        let val = match &d.session_type {
            Some(st) => format!("{de} ({st})"),
            None => de.clone(),
        };
        out.push(kv_line("Desktop", val, pal));
    }
}

/// Junta dois campos opcionais com espaço, retornando `None` se ambos vazios.
fn join_opt(a: Option<&str>, b: Option<&str>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, Some(b)) => Some(b.to_string()),
        (None, None) => None,
    }
}

/// Largura de exibição (colunas) de uma `Line`, somando seus spans.
fn line_width(line: &Line) -> usize {
    line.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}
