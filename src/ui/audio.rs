//! Aba 5 — Mixer de Áudio & Dispositivos (PipeWire / PulseAudio).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::backend::audio::{AudioCategory, AudioNode, AudioSnapshot};

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
                "[1/2/3/Tab] alternar entre Saídas / Apps / Microfones",
                "[+/- ou h/l] ajustar volume (com suporte a Overdrive)",
                "[m] alternar mudo",
                "[Enter] definir como dispositivo padrão",
            ],
        );
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header & Sub-Abas
            Constraint::Min(8),    // Sliders de Volume / Lista
            Constraint::Length(3), // Rodapé & Atalhos
        ])
        .split(area);

    let active_cat = match app.audio_category {
        0 => AudioCategory::Sink,
        1 => AudioCategory::AppStream,
        _ => AudioCategory::Source,
    };

    draw_header(snap, active_cat, pal, f, chunks[0]);
    draw_node_list(snap, active_cat, app, pal, f, chunks[1]);
    draw_footer(pal, f, chunks[2]);
}

fn draw_header(
    snap: &AudioSnapshot,
    active_cat: AudioCategory,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let mut spans = vec![
        Span::styled(" Servidor: ", Style::default().fg(pal.dim)),
        Span::styled(&snap.server_name, Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)),
        Span::styled("   Seções: ", Style::default().fg(pal.dim)),
    ];

    for (i, cat) in AudioCategory::ALL.iter().enumerate() {
        let is_sel = *cat == active_cat;
        let count = snap.nodes_for_category(*cat).len();
        let label = format!(" [{}] {} ({count}) ", i + 1, cat.title());

        if is_sel {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(pal.accent)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ));
        } else {
            spans.push(Span::styled(label, Style::default().fg(pal.dim)));
        }
        spans.push(Span::raw(" "));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            " Mixer de Áudio & Dispositivos ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

fn draw_node_list(
    snap: &AudioSnapshot,
    active_cat: AudioCategory,
    app: &App,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let nodes = snap.nodes_for_category(active_cat);

    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled("Tipo", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Dispositivo / Aplicativo", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Volume / Nível", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Status", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(1);

    if nodes.is_empty() {
        let empty_msg = match active_cat {
            AudioCategory::Sink => "Nenhum dispositivo de saída de áudio detectado.",
            AudioCategory::AppStream => "Nenhum aplicativo reproduzindo áudio no momento.",
            AudioCategory::Source => "Nenhum microfone ou dispositivo de entrada detectado.",
        };
        let p = Paragraph::new(Line::from(Span::styled(empty_msg, Style::default().fg(pal.dim))))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(pal.dim))
                    .title(format!(" {} ", active_cat.title())),
            );
        f.render_widget(p, area);
        return;
    }

    let selected_idx = app.audio_selected.min(nodes.len().saturating_sub(1));

    let rows: Vec<Row> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let is_sel = i == selected_idx;
            format_node_row(node, is_sel, pal)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(8),
        Constraint::Percentage(40),
        Constraint::Percentage(32),
        Constraint::Percentage(17),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(pal.accent))
                .title(Span::styled(
                    format!(" {} ({}) ", active_cat.title(), nodes.len()),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )),
        );

    f.render_widget(table, area);
}

fn format_node_row<'a>(node: &AudioNode, is_sel: bool, pal: &Palette) -> Row<'a> {
    let bullet = if is_sel {
        Span::styled("▶ ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD))
    } else if node.is_default {
        Span::styled("● ", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("  ")
    };

    let type_badge = Span::styled(
        node.category.ascii_badge(),
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
    );

    let name_style = if node.is_muted {
        Style::default().fg(pal.dim).add_modifier(Modifier::CROSSED_OUT)
    } else if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.fg)
    };

    let volume_span = format_volume_slider(node.volume, node.is_muted, pal);

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
        type_badge,
        Span::styled(node.name.clone(), name_style),
        volume_span,
        status_span,
    ])
    .style(row_style)
}

fn format_volume_slider<'a>(volume: f32, is_muted: bool, pal: &Palette) -> Span<'a> {
    let pct = (volume * 100.0).round() as u32;
    let total_bars = 16;
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
        pal.warn // Overdrive
    } else if volume > 0.7 {
        pal.ok
    } else {
        pal.accent
    };

    Span::styled(
        format!("[{bar_str}] {pct:>3}%"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn draw_footer(pal: &Palette, f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" [1/2/3/Tab] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Trocar Seção   ", Style::default().fg(pal.dim)),
        Span::styled("[+/- ou h/l] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Volume (±5%)   ", Style::default().fg(pal.dim)),
        Span::styled("[m] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Mudo   ", Style::default().fg(pal.dim)),
        Span::styled("[Enter] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Definir Padrão   ", Style::default().fg(pal.dim)),
        Span::styled("[r] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Atualizar", Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(" Atalhos do Mixer ");

    f.render_widget(Paragraph::new(line).block(block), area);
}
