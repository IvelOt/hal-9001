//! Aba 1 — Overview (estética neofetch): besouro à esquerda, resumo à direita.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::ascii;

use super::theme::Palette;
use super::widgets::{bar_line, human_bytes, human_uptime, kv_line, palette_line};

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
    let lines: Vec<Line> = art
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), Style::default().fg(pal.accent))))
        .collect();
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

fn draw_info(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let bar_w = (area.width as usize).saturating_sub(18).clamp(6, 24);

    let mut lines: Vec<Line> = Vec::new();

    match &app.system {
        Some(s) => {
            lines.push(Line::from(vec![
                Span::styled(
                    s.user.clone(),
                    Style::default().fg(pal.accent),
                ),
                Span::styled("@", Style::default().fg(pal.dim)),
                Span::styled(s.host.clone(), Style::default().fg(pal.accent)),
            ]));
            lines.push(Line::from(Span::styled(
                "─".repeat((s.user.len() + s.host.len() + 1).min(area.width as usize)),
                Style::default().fg(pal.dim),
            )));
            lines.push(kv_line("OS", s.os.clone(), pal));
            lines.push(kv_line("Kernel", s.kernel.clone(), pal));
            lines.push(kv_line("Uptime", human_uptime(s.uptime_secs), pal));
            lines.push(kv_line("Shell", s.shell.clone(), pal));
            lines.push(kv_line("CPU", s.cpu_name.clone(), pal));
            lines.push(bar_line("Uso CPU", s.cpu_ratio(), bar_w, pal));
            lines.push(kv_line(
                "RAM",
                format!(
                    "{} / {}",
                    human_bytes(s.mem_used),
                    human_bytes(s.mem_total)
                ),
                pal,
            ));
            lines.push(bar_line("Mem", s.mem_ratio(), bar_w, pal));
        }
        None => {
            lines.push(Line::from(Span::styled(
                "coletando dados do sistema…",
                Style::default().fg(pal.dim),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Paleta:",
        Style::default().fg(pal.dim),
    )));
    lines.push(palette_line());

    f.render_widget(Paragraph::new(Text::from(lines)), area);
}
