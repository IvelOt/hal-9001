//! Aba 5 — Mixer de Áudio & Dispositivos (PipeWire / PulseAudio).
//!
//! Visão unificada com 3 divisões simultâneas: Saídas + Aplicativos lado a
//! lado no painel superior, Microfones em largura total no painel inferior.
//! `Tab`/`BackTab` alternam qual painel tem o foco (navegação/rolagem); as
//! teclas `1`..`8` permanecem livres para a tabbar principal.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::backend::audio::{AudioCategory, AudioNode, AudioSnapshot};
use crate::ui::widgets::truncate_str;

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let Some(snap) = &app.audio else {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            "Mixer de Áudio",
            "audio",
            &[
                "[Tab/Shift+Tab] alternar foco entre painéis",
                "[j/k] navegar   [+/- ou h/l] volume",
                "[m] alternar mudo",
                "[Enter] definir como dispositivo padrão",
            ],
        );
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Percentage(55), // Painel Superior (Saídas + Apps)
            Constraint::Percentage(45), // Painel Inferior (Microfones)
            Constraint::Length(3), // Rodapé
        ])
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[1]);

    let focus = app.audio_category.min(2);
    let selected = app.audio_selected;

    draw_header(snap, pal, f, chunks[0]);
    draw_card_panel(
        snap.nodes_for_category(AudioCategory::Sink),
        AudioCategory::Sink,
        focus == 0,
        selected,
        pal,
        f,
        top_cols[0],
    );
    draw_card_panel(
        snap.nodes_for_category(AudioCategory::AppStream),
        AudioCategory::AppStream,
        focus == 1,
        selected,
        pal,
        f,
        top_cols[1],
    );
    draw_source_panel(
        snap.nodes_for_category(AudioCategory::Source),
        focus == 2,
        selected,
        pal,
        f,
        chunks[2],
    );
    draw_footer(pal, f, chunks[3]);
}

fn default_node_name(nodes: &[AudioNode]) -> Option<&str> {
    nodes
        .iter()
        .find(|n| n.is_default)
        .map(|n| n.name.as_str())
}

fn draw_header(snap: &AudioSnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let output = default_node_name(&snap.sinks).unwrap_or("Nenhuma saída padrão");
    let mic = default_node_name(&snap.sources).unwrap_or("Nenhum microfone padrão");

    // Colunas truncadas para nunca estourar a borda, mesmo em terminais estreitos.
    let avail = area.width.saturating_sub(2) as usize; // desconta bordas
    let seg = avail / 3;

    let line = Line::from(vec![
        Span::styled(" Servidor: ", Style::default().fg(pal.dim)),
        Span::styled(
            truncate_str(&snap.server_name, seg),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        ),
        Span::styled("   Saída: ", Style::default().fg(pal.dim)),
        Span::styled(
            truncate_str(output, seg),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled("   Mic: ", Style::default().fg(pal.dim)),
        Span::styled(
            truncate_str(mic, seg),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            " Mixer de Áudio & Dispositivos ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(line).block(block), area);
}

/// Calcula a janela `[start, end)` de itens visíveis dado o total, a altura
/// visível (em itens) e o índice selecionado — rola para manter a seleção
/// dentro da vista quando o painel está focado; painéis sem foco sempre
/// mostram a partir do topo.
fn scroll_window(len: usize, visible: usize, selected: usize, focused: bool) -> (usize, usize) {
    if visible == 0 || len == 0 {
        return (0, 0);
    }
    if len <= visible || !focused {
        return (0, len.min(visible));
    }
    let sel = selected.min(len - 1);
    let mut start = sel.saturating_sub(visible - 1).max(0);
    if start + visible > len {
        start = len - visible;
    }
    if sel < start {
        start = sel;
    }
    (start, (start + visible).min(len))
}

/// Sufixo discreto de rolagem para o título do painel (`▲N ▼N`).
fn scroll_indicator(start: usize, end: usize, len: usize) -> String {
    let above = start;
    let below = len.saturating_sub(end);
    let mut s = String::new();
    if above > 0 {
        s.push_str(&format!(" ▲{above}"));
    }
    if below > 0 {
        s.push_str(&format!(" ▼{below}"));
    }
    s
}

fn panel_title(icon: &str, label: &str, count: usize, start: usize, end: usize) -> String {
    format!(
        " {icon} {label} ({count}){} ",
        scroll_indicator(start, end, count)
    )
}

fn empty_message(cat: AudioCategory) -> &'static str {
    match cat {
        AudioCategory::Sink => "Nenhum dispositivo de saída de áudio detectado.",
        AudioCategory::AppStream => "Nenhum aplicativo reproduzindo áudio no momento.",
        AudioCategory::Source => "Nenhum microfone ou dispositivo de entrada detectado.",
    }
}

/// Painel de card (2 linhas por item: nome + barra/badge) usado pelas
/// colunas superiores (Saídas e Aplicativos), ~50% de largura cada.
fn draw_card_panel(
    nodes: &[AudioNode],
    cat: AudioCategory,
    focused: bool,
    selected: usize,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let border_color = if focused { pal.accent } else { pal.dim };
    let icon = cat.nerd_glyph();

    let visible_items = (area.height.saturating_sub(2) / 2).max(1) as usize;
    let (start, end) = scroll_window(nodes.len(), visible_items, selected, focused);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            panel_title(icon, cat.title(), nodes.len(), start, end),
            Style::default().fg(border_color).add_modifier(Modifier::BOLD),
        ));

    if nodes.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            empty_message(cat),
            Style::default().fg(pal.dim),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    }

    let inner_w = area.width.saturating_sub(2) as usize;
    let sel_idx = selected.min(nodes.len() - 1);

    let mut lines: Vec<Line> = Vec::new();
    for (i, node) in nodes.iter().enumerate().take(end).skip(start) {
        let is_sel = focused && i == sel_idx;
        lines.push(name_line(node, is_sel, inner_w, pal));
        lines.push(bar_status_line(node, inner_w, pal));
    }

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn name_line<'a>(node: &AudioNode, is_sel: bool, width: usize, pal: &Palette) -> Line<'a> {
    let bullet = if is_sel {
        "▶ "
    } else if node.is_default {
        "● "
    } else {
        "  "
    };
    let name_style = if node.is_muted {
        Style::default().fg(pal.dim).add_modifier(Modifier::CROSSED_OUT)
    } else if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.fg)
    };
    let bullet_style = if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
    } else if node.is_default {
        Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let name_budget = width.saturating_sub(2);
    let line_style = if is_sel {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::styled(bullet, bullet_style),
        Span::styled(truncate_str(&node.name, name_budget), name_style),
    ])
    .style(line_style)
}

fn bar_status_line<'a>(node: &AudioNode, width: usize, pal: &Palette) -> Line<'a> {
    let status = if node.is_muted && node.is_default {
        "[MUDO][PADRÃO]"
    } else if node.is_muted {
        "[MUDO]"
    } else if node.is_default {
        "[PADRÃO]"
    } else {
        "Ativo"
    };
    let status_color = if node.is_muted {
        pal.err
    } else if node.is_default {
        pal.ok
    } else {
        pal.dim
    };

    // Reserva espaço para o badge de status + 1 separador; o restante vira a
    // barra de volume compacta.
    let status_w = status.len().min(width.saturating_sub(4));
    let bar_budget = width.saturating_sub(status_w + 1).max(4);
    let bar_span = format_volume_bar(node.volume, node.is_muted, pal, bar_budget);

    Line::from(vec![
        Span::raw("  "),
        bar_span,
        Span::raw(" "),
        Span::styled(truncate_str(status, status_w), Style::default().fg(status_color)),
    ])
}

/// Barra de volume compacta `[████░░░░] %`, com o número de blocos ajustado
/// ao orçamento de colunas disponível (nunca estoura o painel).
fn format_volume_bar<'a>(volume: f32, is_muted: bool, pal: &Palette, budget: usize) -> Span<'a> {
    let pct = (volume * 100.0).round() as u32;
    let pct_str = format!("{pct:>3}%");
    // "[" + barras + "] " + pct
    let overhead = 2 + 1 + pct_str.len();
    let total_bars = budget.saturating_sub(overhead).clamp(3, 16);
    let filled_bars = ((volume.min(1.0) * total_bars as f32).round() as usize).min(total_bars);

    let mut bar_str = String::new();
    for _ in 0..filled_bars {
        bar_str.push('█');
    }
    for _ in filled_bars..total_bars {
        bar_str.push('░');
    }

    let color = if is_muted {
        pal.dim
    } else if volume > 1.0 {
        pal.warn
    } else if volume > 0.7 {
        pal.ok
    } else {
        pal.accent
    };

    Span::styled(
        format!("[{bar_str}] {pct_str}"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// Painel de microfones (largura total, ~45% da altura útil): tabela em
/// linha única por item.
fn draw_source_panel(
    nodes: &[AudioNode],
    focused: bool,
    selected: usize,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let border_color = if focused { pal.accent } else { pal.dim };
    let icon = AudioCategory::Source.nerd_glyph();

    let visible_items = area.height.saturating_sub(3).max(1) as usize; // desconta bordas + header da tabela
    let (start, end) = scroll_window(nodes.len(), visible_items, selected, focused);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(
            panel_title(icon, "Dispositivos de Entrada / Microfones", nodes.len(), start, end),
            Style::default().fg(border_color).add_modifier(Modifier::BOLD),
        ));

    if nodes.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            empty_message(AudioCategory::Source),
            Style::default().fg(pal.dim),
        )))
        .block(block);
        f.render_widget(p, area);
        return;
    }

    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled("Dispositivo", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Nível", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Status", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(1);

    let sel_idx = selected.min(nodes.len() - 1);
    let inner_w = area.width.saturating_sub(2) as usize;
    let name_w = (inner_w * 40 / 100).max(6);

    let rows: Vec<Row> = nodes
        .iter()
        .enumerate()
        .take(end)
        .skip(start)
        .map(|(i, node)| format_source_row(node, focused && i == sel_idx, name_w, pal))
        .collect();

    let widths = [
        Constraint::Length(2),
        Constraint::Percentage(40),
        Constraint::Percentage(35),
        Constraint::Percentage(25),
    ];

    let table = Table::new(rows, widths).header(header).block(block);
    f.render_widget(table, area);
}

fn format_source_row<'a>(node: &AudioNode, is_sel: bool, name_w: usize, pal: &Palette) -> Row<'a> {
    let bullet = if is_sel {
        Span::styled("▶ ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD))
    } else if node.is_default {
        Span::styled("● ", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("  ")
    };

    let name_style = if node.is_muted {
        Style::default().fg(pal.dim).add_modifier(Modifier::CROSSED_OUT)
    } else if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.fg)
    };

    let volume_span = format_volume_bar(node.volume, node.is_muted, pal, 20);

    let status_span = if node.is_muted && node.is_default {
        Span::styled("[MUDO] [PADRÃO]", Style::default().fg(pal.err).add_modifier(Modifier::BOLD))
    } else if node.is_muted {
        Span::styled("[MUDO]", Style::default().fg(pal.err).add_modifier(Modifier::BOLD))
    } else if node.is_default {
        Span::styled("[PADRÃO]", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("Ativo", Style::default().fg(pal.dim))
    };

    let row_style = if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    Row::new(vec![
        bullet,
        Span::styled(truncate_str(&node.name, name_w), name_style),
        volume_span,
        status_span,
    ])
    .style(row_style)
}

fn draw_footer(pal: &Palette, f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" [Tab/⇧Tab] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Foco   ", Style::default().fg(pal.dim)),
        Span::styled("[j/k] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Navegar   ", Style::default().fg(pal.dim)),
        Span::styled("[+/- h/l] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Volume (±5%)   ", Style::default().fg(pal.dim)),
        Span::styled("[m] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Mudo   ", Style::default().fg(pal.dim)),
        Span::styled("[Enter] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Padrão/Mudo   ", Style::default().fg(pal.dim)),
        Span::styled("[r] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Atualizar", Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(" Atalhos do Mixer ");

    f.render_widget(Paragraph::new(line).block(block), area);
}
