//! Aba 6 — Telas, Monitores & Configuração de Displays (X11 / xrandr).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::backend::display::{DisplayNode, DisplaySnapshot};

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let Some(snap) = &app.displays else {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            "Telas & Monitores",
            "display",
            &[
                "[1] Expandir à Direita   [2] Expandir à Esquerda",
                "[3] Espelhar Telas       [4] Somente Externo",
                "[5] Somente Notebook     [p] Definir Monitor Primário",
            ],
        );
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header & Modo Atual
            Constraint::Length(7), // Diagrama ASCII 2D de Layout
            Constraint::Min(8),    // Lista de Monitores & Resoluções
            Constraint::Length(3), // Rodapé & Atalhos
        ])
        .split(area);

    draw_header(snap, pal, f, chunks[0]);
    draw_layout_diagram(snap, app, pal, f, chunks[1]);
    draw_displays_table(snap, app, pal, f, chunks[2]);
    draw_footer(pal, f, chunks[3]);
}

fn draw_header(snap: &DisplaySnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let layout_badge = if let Some(l) = snap.current_layout {
        l.title()
    } else {
        "Individual / Personalizado"
    };

    let spans = vec![
        Span::styled(" Conectados: ", Style::default().fg(pal.dim)),
        Span::styled(format!("{} monitor(es)", snap.connected_count), Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)),
        Span::styled("   Modo Ativo: ", Style::default().fg(pal.dim)),
        Span::styled(format!("[ {layout_badge} ]"), Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("   Primário: ", Style::default().fg(pal.dim)),
        Span::styled(snap.primary_name.as_deref().unwrap_or("Nenhum"), Style::default().fg(pal.fg)),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            " Gerenciador de Telas & Monitores (X11) ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

fn draw_layout_diagram(snap: &DisplaySnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let connected = snap.connected_displays();

    if connected.is_empty() {
        let p = Paragraph::new("Nenhuma saída de vídeo ativa.")
            .block(Block::default().borders(Borders::ALL).title(" Layout Espacial das Telas "));
        f.render_widget(p, area);
        return;
    }

    let mut lines = Vec::new();

    // Renderiza caixas ASCII para até 2 monitores lado a lado
    if connected.len() >= 2 {
        let d1 = connected[0];
        let d2 = connected[1];

        let sel_idx = app.display_selected.min(connected.len().saturating_sub(1));
        let d1_sel = sel_idx == 0;
        let d2_sel = sel_idx == 1;

        let b1 = if d1_sel { "▶" } else if d1.is_primary { "●" } else { " " };
        let b2 = if d2_sel { "▶" } else if d2.is_primary { "●" } else { " " };

        let d1_label = format!("{b1} {} ({})", d1.name, d1.resolution_str());
        let d2_label = format!("{b2} {} ({})", d2.name, d2.resolution_str());

        lines.push(Line::from(vec![
            Span::styled("+------------------------------------+   +------------------------------------+", Style::default().fg(pal.accent)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("| {d1_label:<34} |   | {d2_label:<34} |"), Style::default().fg(pal.fg).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("| Pos: ({}, {})  Rot: {:<12} |   | Pos: ({}, {})  Rot: {:<12} |", d1.pos_x, d1.pos_y, d1.rotation, d2.pos_x, d2.pos_y, d2.rotation), Style::default().fg(pal.dim)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("+------------------------------------+   +------------------------------------+", Style::default().fg(pal.accent)),
        ]));
    } else if let Some(d1) = connected.first() {
        let b1 = if d1.is_primary { "●" } else { " " };
        let d1_label = format!("{b1} {} ({})", d1.name, d1.resolution_str());

        lines.push(Line::from(vec![
            Span::styled("+------------------------------------+", Style::default().fg(pal.accent)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("| {d1_label:<34} |"), Style::default().fg(pal.fg).add_modifier(Modifier::BOLD)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(format!("| Pos: ({}, {})  Rot: {:<12} |", d1.pos_x, d1.pos_y, d1.rotation), Style::default().fg(pal.dim)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("+------------------------------------+", Style::default().fg(pal.accent)),
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(Span::styled(" Arranjo Espacial de Telas (Canvas 2D) ", Style::default().fg(pal.accent)));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_displays_table(snap: &DisplaySnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled("Saída", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Tipo", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Resolução Atual", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Status / Modos Suportados", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(1);

    let sel_idx = app.display_selected.min(snap.displays.len().saturating_sub(1));

    let rows: Vec<Row> = snap.displays
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let is_sel = i == sel_idx;
            format_display_row(d, is_sel, pal)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(28),
        Constraint::Min(20),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(pal.accent))
                .title(Span::styled(
                    format!(" Saídas de Vídeo Detectadas ({}) ", snap.displays.len()),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )),
        );

    f.render_widget(table, area);
}

fn format_display_row<'a>(d: &DisplayNode, is_sel: bool, pal: &Palette) -> Row<'a> {
    let bullet = if is_sel {
        Span::styled("▶ ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD))
    } else if d.is_primary {
        Span::styled("● ", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("  ")
    };

    let type_badge = if d.is_internal {
        Span::styled("[NOTEBOOK]", Style::default().fg(pal.accent))
    } else {
        Span::styled("[EXTERNO ]", Style::default().fg(pal.ok))
    };

    let res_span = if d.is_active {
        Span::styled(d.resolution_str(), Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else if d.is_connected {
        Span::styled("Conectado (Desativado)", Style::default().fg(pal.warn))
    } else {
        Span::styled("Desconectado", Style::default().fg(pal.dim))
    };

    let status_str = if d.is_primary {
        format!("[● PRIMÁRIO] ({} resoluções)", d.supported_modes.len())
    } else if d.is_connected {
        format!("[ATIVO] ({} resoluções)", d.supported_modes.len())
    } else {
        "Sem sinal de vídeo".to_string()
    };

    let row_style = if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    Row::new(vec![
        bullet,
        Span::styled(d.name.clone(), Style::default().fg(pal.fg).add_modifier(Modifier::BOLD)),
        type_badge,
        res_span,
        Span::styled(status_str, Style::default().fg(pal.dim)),
    ])
    .style(row_style)
}

fn draw_footer(pal: &Palette, f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" [1] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Expandir (Dir)  ", Style::default().fg(pal.dim)),
        Span::styled("[2] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Expandir (Esq)  ", Style::default().fg(pal.dim)),
        Span::styled("[3] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Espelhar  ", Style::default().fg(pal.dim)),
        Span::styled("[4] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Só Externo  ", Style::default().fg(pal.dim)),
        Span::styled("[5] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Só Interno  ", Style::default().fg(pal.dim)),
        Span::styled("[p] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Primário  ", Style::default().fg(pal.dim)),
        Span::styled("[r] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Scan", Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(" Ações Rápidas de Telas ");

    f.render_widget(Paragraph::new(line).block(block), area);
}
