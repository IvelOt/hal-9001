//! Splash animada: `LOADING...` → `Bem-vindo, <user>!` sobre o besouro.

use ratatui::layout::{Alignment, Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::App;
use crate::ascii;

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let elapsed = app.elapsed_ms();
    let total = app.config.splash.min_ms.max(1) as u128;
    let progress = (elapsed as f64 / total as f64).clamp(0.0, 1.0);

    // Reticências animadas.
    let dots = ".".repeat(((elapsed / 300) % 4) as usize);

    // Barra de carregamento.
    let bar_w = 24usize;
    let filled = (progress * bar_w as f64).round() as usize;
    let bar = format!("[{}{}]", "█".repeat(filled), "░".repeat(bar_w - filled));

    let m = app.lang.messages();
    let user = std::env::var("USER").unwrap_or_else(|_| "operador".into());

    // Nas primeiras ~2/3 do tempo: LOADING; depois: boas-vindas.
    let headline = if progress < 0.66 {
        Line::from(Span::styled(
            format!("{}{dots}", m.splash_loading),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))
    } else {
        Line::from(Span::styled(
            format!("{}, {user}!", m.splash_welcome),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        ))
    };

    // Logo das engrenagens com o olho do HAL, colorida, responsiva e com o pulso
    // de respiração do olho (fase derivada do tempo decorrido). Em telas micro,
    // `select` devolve `None` e a logo é recolhida, mantendo o texto centralizado.
    let size = ascii::select("auto", area.width.saturating_sub(4));
    let phase = ((elapsed / 250) % 4) as u8;

    let mut lines: Vec<Line> = Vec::new();
    if let Some(size) = size {
        lines.extend(ascii::logo_lines_phase(size, phase));
        lines.push(Line::from(""));
    }
    lines.push(headline);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(bar, Style::default().fg(pal.accent))));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        m.splash_title,
        Style::default().fg(pal.dim),
    )));

    // Centraliza o bloco vertical e horizontalmente (`Flex::Center` + alinhamento
    // central do parágrafo), recortando à área quando o terminal é curto.
    let content_h = (lines.len() as u16).min(area.height).max(1);
    let band = Layout::vertical([Constraint::Length(content_h)])
        .flex(Flex::Center)
        .split(area)[0];

    let para = Paragraph::new(Text::from(lines)).alignment(Alignment::Center);
    f.render_widget(para, band);
}
