//! Helpers de widgets reutilizáveis (barras, linhas chave/valor).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::theme::Palette;

/// Formata bytes em unidade humana (KiB/MiB/GiB).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut val = bytes as f64;
    let mut unit = 0;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{val:.1} {}", UNITS[unit])
    }
}

/// Uptime humano a partir de segundos: `1d 2h 3m`.
pub fn human_uptime(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    let mut out = String::new();
    if d > 0 {
        out.push_str(&format!("{d}d "));
    }
    if d > 0 || h > 0 {
        out.push_str(&format!("{h}h "));
    }
    out.push_str(&format!("{m}m"));
    out
}

/// Linha "rótulo: valor" com rótulo destacado.
pub fn kv_line<'a>(label: &'a str, value: String, pal: &Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{label:<9}"),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(value, Style::default().fg(pal.fg)),
    ])
}

/// Linha com barra de progresso textual: `CPU  [██████░░░░]  62%`.
pub fn bar_line<'a>(label: &'a str, ratio: f64, width: usize, pal: &Palette) -> Line<'a> {
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = (ratio * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    let color = pal.gauge_color(ratio);

    Line::from(vec![
        Span::styled(
            format!("{label:<9}"),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("[", Style::default().fg(pal.dim)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(pal.dim)),
        Span::styled("] ", Style::default().fg(pal.dim)),
        Span::styled(
            format!("{:>3.0}%", ratio * 100.0),
            Style::default().fg(pal.fg),
        ),
    ])
}

/// Faixa de 16 blocos representando a paleta de cores do terminal.
pub fn palette_line<'a>() -> Line<'a> {
    use ratatui::style::Color;
    let colors = [
        Color::Black,
        Color::Red,
        Color::Green,
        Color::Yellow,
        Color::Blue,
        Color::Magenta,
        Color::Cyan,
        Color::Gray,
        Color::DarkGray,
        Color::LightRed,
        Color::LightGreen,
        Color::LightYellow,
        Color::LightBlue,
        Color::LightMagenta,
        Color::LightCyan,
        Color::White,
    ];
    let spans: Vec<Span> = colors
        .into_iter()
        .map(|c| Span::styled("██", Style::default().fg(c)))
        .collect();
    Line::from(spans)
}
