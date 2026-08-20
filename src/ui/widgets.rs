//! Helpers de widgets reutilizáveis (barras, linhas chave/valor).

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::theme::Palette;

/// Largura reservada ao rótulo das linhas `rótulo valor` do Overview.
const LABEL_W: usize = 9;

/// Trunca `s` para caber em `max` colunas de exibição, anexando `…` quando
/// corta. Respeita a largura Unicode de cada caractere (CJK/emoji contam 2).
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
    // Reserva 1 coluna para a reticência.
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

/// Título de seção estilo Hermes: rótulo em âmbar/amarelo, maiúsculo e negrito,
/// com um marcador `▍` à esquerda para ancorar a coluna de informações.
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

/// Linha "rótulo valor" com rótulo destacado. O valor é truncado com `…` para
/// nunca ultrapassar `width` colunas (rótulo incluso), evitando o vazamento
/// horizontal apontado no briefing.
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

/// Linha densa que combina, numa única linha, o rótulo, um `valor` métrico
/// (ex.: `6.3 / 15.3 GiB`) e a barra de progresso com percentual — reduzindo à
/// metade a altura das seções. O `valor` é alinhado numa coluna de `val_w`
/// colunas (truncado com `…` se preciso) para que as barras fiquem alinhadas.
/// Um `suffix` opcional (ex.: `[CHARGING +25W]`, `[MUTED]`) é anexado ao fim.
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

    let val = truncate_str(value, val_w);
    let pad = val_w.saturating_sub(UnicodeWidthStr::width(val.as_str()));

    let mut spans = vec![
        Span::styled(
            format!("{label:<LABEL_W$}"),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(val, Style::default().fg(pal.fg)),
        // Espaçamento até a barra (padding do valor + 1 folga).
        Span::raw(" ".repeat(pad + 1)),
        Span::styled("[", Style::default().fg(pal.dim)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(pal.dim)),
        Span::styled("] ", Style::default().fg(pal.dim)),
        Span::styled(
            format!("{:>3.0}%", ratio * 100.0),
            Style::default().fg(pal.fg),
        ),
    ];
    if let Some(s) = suffix {
        spans.push(Span::styled(
            format!("  {s}"),
            Style::default().fg(pal.dim),
        ));
    }
    Line::from(spans)
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
