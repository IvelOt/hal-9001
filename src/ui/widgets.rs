
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::Palette;

const LABEL_W: usize = 10;

pub fn truncate_str(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(s) <= max {
        return s.to_string();
    }
    if max == 1 {
        return "…".to_string();
    }

    let budget = max - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

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

pub fn section_title<'a>(text: &str, pal: &Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled("▍ ", Style::default().fg(pal.accent)),
        Span::styled(
            text.to_uppercase(),
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

pub fn kv_line<'a>(label: &'a str, value: String, width: usize, pal: &Palette) -> Line<'a> {
    let avail = width.saturating_sub(LABEL_W).max(1);
    Line::from(vec![
        Span::styled(
            format!("{label:<LABEL_W$}"),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(truncate_str(&value, avail), Style::default().fg(pal.fg)),
    ])
}

#[allow(clippy::too_many_arguments)]
pub fn metric_line<'a>(
    label: &'a str,
    value: &str,
    val_w: usize,
    ratio: f64,
    bar_w: usize,
    pal: &Palette,
    suffix: Option<&str>,
) -> Line<'a> {
    let ratio = ratio.clamp(0.0, 1.0);
    let filled = (ratio * bar_w as f64).round() as usize;
    let empty = bar_w.saturating_sub(filled);
    let color = pal.gauge_color(ratio);

    let mut spans: Vec<Span> = vec![Span::styled(
        format!("{label:<LABEL_W$}"),
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
    )];

    let mut mid_w = 0usize;
    if !value.is_empty() {
        let val = truncate_str(value, val_w);
        mid_w += UnicodeWidthStr::width(val.as_str());
        spans.push(Span::styled(val, Style::default().fg(pal.fg)));
    }
    if let Some(s) = suffix {
        let sep = usize::from(mid_w > 0);
        let budget = val_w.saturating_sub(mid_w + sep);
        let tag = truncate_str(s, budget);
        if !tag.is_empty() {
            if sep == 1 {
                spans.push(Span::raw(" "));
                mid_w += 1;
            }
            mid_w += UnicodeWidthStr::width(tag.as_str());
            spans.push(Span::styled(tag, Style::default().fg(pal.dim)));
        }
    }

    let pad = val_w.saturating_sub(mid_w);
    spans.extend([
        Span::raw(" ".repeat(pad + 1)),
        Span::styled("[", Style::default().fg(pal.dim)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(pal.dim)),
        Span::styled("] ", Style::default().fg(pal.dim)),
        Span::styled(
            format!("{:>3.0}%", ratio * 100.0),
            Style::default().fg(pal.fg),
        ),
    ]);
    Line::from(spans)
}

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
