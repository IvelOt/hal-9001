//! Aba 1 — Overview (estética neofetch): besouro à esquerda, resumo à direita.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::ascii;
use crate::backend::system::SystemSnapshot;

use super::theme::Palette;
use super::widgets::{bar_line, bar_line_suffix, human_bytes, human_uptime, kv_line, palette_line};

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let block = super::content_block("Overview", pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let cols = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(inner);

    draw_beetle(app, pal, f, cols[0]);
    draw_info(app, pal, f, cols[1]);
}

fn draw_beetle(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let art = ascii::select(&app.config.overview.ascii, area.width);
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

fn draw_info(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let bar_w = (area.width as usize).saturating_sub(18).clamp(6, 24);

    let mut lines: Vec<Line> = Vec::new();

    match &app.system {
        Some(s) => info_lines(s, pal, bar_w, area.width, &mut lines),
        None => lines.push(Line::from(Span::styled(
            "coletando dados do sistema…",
            Style::default().fg(pal.dim),
        ))),
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Paleta:",
        Style::default().fg(pal.dim),
    )));
    lines.push(palette_line());

    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// Monta as linhas do painel a partir de um snapshot — pura, sem `Frame`, para
/// facilitar leitura e futura testabilidade.
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

    // Bateria (ou N/A em desktop).
    match &s.battery {
        Some(b) => {
            let mut suffix = format!("{} {}", b.status.icon(), b.status.label());
            if let Some(w) = b.power_watts {
                suffix.push_str(&format!(" · {w:.1} W"));
            }
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

    // Volume (+ mudo).
    match &s.volume {
        Some(v) => {
            let suffix = if v.muted { "🔇 mudo" } else { "🔊" };
            out.push(bar_line_suffix("Volume", v.ratio(), bar_w, pal, suffix));
        }
        None => out.push(kv_line("Volume", "N/A".into(), pal)),
    }
}
