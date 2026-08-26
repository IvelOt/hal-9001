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

    let dots = ".".repeat(((elapsed / 300) % 4) as usize);

    let bar_w = 24usize;
    let filled = (progress * bar_w as f64).round() as usize;
    let bar = format!("[{}{}]", "█".repeat(filled), "░".repeat(bar_w - filled));

    let m = app.lang.messages();
    let user = std::env::var("USER").unwrap_or_else(|_| "operador".into());

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

    let size = ascii::select("auto", area.width.saturating_sub(4));
    let phase = ((elapsed / 250) % 4) as u8;

    let mut lines: Vec<Line> = Vec::new();
    if let Some(size) = size {
        lines.extend(ascii::logo_lines_phase(size, phase));
        lines.push(Line::from(""));
    }
    lines.push(headline);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        bar,
        Style::default().fg(pal.accent),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        m.splash_title,
        Style::default().fg(pal.dim),
    )));

    let content_h = (lines.len() as u16).min(area.height).max(1);
    let band = Layout::vertical([Constraint::Length(content_h)])
        .flex(Flex::Center)
        .split(area)[0];

    let para = Paragraph::new(Text::from(lines)).alignment(Alignment::Center);
    f.render_widget(para, band);
}
