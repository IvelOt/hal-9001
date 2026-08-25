//! Aba 6 — Telas, Monitores & Configuração de Displays (estilo monitui).
//!
//! 100% Pure Rust & Ratatui — Layout visual espacial com canvas de monitores,
//! seletor de modos de arranjo e inspetor interativo de resoluções suportadas.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::backend::display::{DisplayLayoutMode, DisplayNode, DisplaySnapshot};

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
                "[e] Expandir à Direita   [E] Expandir à Esquerda",
                "[m] Espelhar Telas       [x] Somente Externo",
                "[i] Somente Notebook     [p] Definir Monitor Primário",
            ],
        );
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header & Modo Ativo
            Constraint::Length(9), // Canvas Visual 2D dos Monitores (estilo monitui)
            Constraint::Min(9),    // Painel Inferior: Modos de Layout + Inspetor de Resoluções
            Constraint::Length(3), // Rodapé & Atalhos Globais
        ])
        .split(area);

    draw_header(snap, pal, f, chunks[0]);
    draw_monitor_canvas(snap, app, pal, f, chunks[1]);
    draw_inspector_and_modes(snap, app, pal, f, chunks[2]);
    draw_footer(pal, f, chunks[3]);
}

fn draw_header(snap: &DisplaySnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let layout_badge = if let Some(l) = snap.current_layout {
        l.title()
    } else {
        "Individual / Custom"
    };

    let spans = vec![
        Span::styled(" Conectados: ", Style::default().fg(pal.dim)),
        Span::styled(format!("{} tela(s)", snap.connected_count), Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)),
        Span::styled("   Arranjo Atual: ", Style::default().fg(pal.dim)),
        Span::styled(format!("[ {layout_badge} ]"), Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("   Monitor Primário: ", Style::default().fg(pal.dim)),
        Span::styled(snap.primary_name.as_deref().unwrap_or("Nenhum"), Style::default().fg(pal.fg).add_modifier(Modifier::BOLD)),
        Span::styled("   Servidor: ", Style::default().fg(pal.dim)),
        Span::styled(
            if snap.server_type.is_empty() { "X11 (RandR)" } else { &snap.server_type },
            Style::default().fg(pal.accent),
        ),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            " Gerenciador de Telas & Monitores (Canvas 2D) ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

/// Canvas visual dos monitores no espaço virtual 2D (inspirado no monitui).
fn draw_monitor_canvas(snap: &DisplaySnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let connected = snap.connected_displays();

    if connected.is_empty() {
        let p = Paragraph::new("Nenhuma saída de vídeo ativa conectada.")
            .block(Block::default().borders(Borders::ALL).title(" Canvas de Telas "));
        f.render_widget(p, area);
        return;
    }

    let sel_idx = app.display_selected.min(connected.len().saturating_sub(1));

    // Divide a largura do canvas proporcionalmente entre os monitores conectados
    let mut constraints = Vec::new();
    for _ in 0..connected.len() {
        constraints.push(Constraint::Ratio(1, connected.len() as u32));
    }

    let monitor_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, d) in connected.iter().enumerate() {
        let is_selected = i == sel_idx;
        draw_single_monitor_box(d, is_selected, pal, f, monitor_cols[i]);
    }
}

fn draw_single_monitor_box(
    d: &DisplayNode,
    is_selected: bool,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let border_color = if is_selected {
        pal.accent
    } else {
        pal.dim
    };

    let title_badge = if is_selected {
        format!(" ▶ {} [SELECIONADO] ", d.name)
    } else if d.is_primary {
        format!(" ● {} [PRIMÁRIO] ", d.name)
    } else {
        format!(" {} ", d.name)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if is_selected {
            Style::default().fg(border_color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(border_color)
        })
        .title(Span::styled(
            title_badge,
            if is_selected {
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
            } else if d.is_primary {
                Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(pal.fg)
            },
        ));

    let type_str = if d.is_internal {
        "Tela Interna (Notebook)"
    } else {
        "Monitor Externo"
    };

    let status_span = if d.is_primary {
        Span::styled("[● PRIMÁRIO] ", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else if d.is_active {
        Span::styled("[ATIVO] ", Style::default().fg(pal.accent))
    } else {
        Span::styled("[DESATIVADO] ", Style::default().fg(pal.warn))
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(" Tipo: ", Style::default().fg(pal.dim)),
            Span::styled(type_str, Style::default().fg(pal.fg)),
            Span::raw("   "),
            status_span,
        ]),
        Line::from(vec![
            Span::styled(" Resolução: ", Style::default().fg(pal.dim)),
            Span::styled(
                d.resolution_str(),
                Style::default().fg(if is_selected { pal.ok } else { pal.fg }).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Posição Virtual: ", Style::default().fg(pal.dim)),
            Span::styled(format!("X: {}, Y: {}", d.pos_x, d.pos_y), Style::default().fg(pal.dim)),
            Span::styled("   Rotação: ", Style::default().fg(pal.dim)),
            Span::styled(&d.rotation, Style::default().fg(pal.fg)),
        ]),
        Line::from(vec![
            Span::styled(
                if is_selected {
                    " [h/l] Trocar Foco de Tela   [p] Definir Primário "
                } else {
                    " Use [h/l] para selecionar esta tela "
                },
                Style::default().fg(pal.dim),
            ),
        ]),
    ];

    f.render_widget(Paragraph::new(lines).block(block), area);
}

/// Painel inferior dividido em: Modos de Layout (Esquerda) e Inspetor de Resoluções (Direita).
fn draw_inspector_and_modes(snap: &DisplaySnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let sub_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(45), // Modos Rápidos de Arranjo
            Constraint::Percentage(55), // Lista de Resoluções Suportadas
        ])
        .split(area);

    draw_modes_card(snap, pal, f, sub_chunks[0]);
    draw_resolutions_inspector(snap, app, pal, f, sub_chunks[1]);
}

fn draw_modes_card(snap: &DisplaySnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let current = snap.current_layout;

    let modes = [
        (DisplayLayoutMode::ExtendRight, "[e]", "Expandir à Direita (Padrão)"),
        (DisplayLayoutMode::ExtendLeft,  "[E]", "Expandir à Esquerda"),
        (DisplayLayoutMode::Mirror,      "[m]", "Espelhar Telas (Duplicar)"),
        (DisplayLayoutMode::ExternalOnly,"[x]", "Apenas Monitor Externo"),
        (DisplayLayoutMode::InternalOnly,"[i]", "Apenas Tela do Notebook"),
    ];

    let mut lines = Vec::new();
    for (mode, key, desc) in modes {
        let is_active = current == Some(mode);
        let bullet = if is_active { "● " } else { "○ " };
        let style = if is_active {
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.fg)
        };

        lines.push(Line::from(vec![
            Span::styled(format!(" {bullet}"), style),
            Span::styled(format!("{key} "), Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:<28}", desc), style),
            if is_active {
                Span::styled("[ATIVO]", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
            } else {
                Span::raw("")
            },
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(Span::styled(
            " Modos de Arranjo (Atalhos Rápidos) ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_resolutions_inspector(snap: &DisplaySnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let connected = snap.connected_displays();
    let sel_idx = app.display_selected.min(connected.len().saturating_sub(1));

    let Some(selected_display) = connected.get(sel_idx) else {
        let p = Paragraph::new("Nenhum monitor selecionado.")
            .block(Block::default().borders(Borders::ALL).title(" Resoluções Suportadas "));
        f.render_widget(p, area);
        return;
    };

    let modes = &selected_display.supported_modes;
    let sel_res_idx = app.display_res_selected.min(modes.len().saturating_sub(1));

    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled("Resolução", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Taxa (Hz)", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Status", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(0);

    let rows: Vec<Row> = modes
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let is_sel = i == sel_res_idx;
            let bullet = if is_sel {
                Span::styled("▶ ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD))
            } else if m.is_current {
                Span::styled("● ", Style::default().fg(pal.ok))
            } else {
                Span::raw("  ")
            };

            let res_str = format!("{}x{}", m.width, m.height);
            let rate_str = format!("{:.2} Hz", m.rate);

            let mut badges = Vec::new();
            if m.is_current {
                badges.push("[ATUAL]");
            }
            if m.is_preferred {
                badges.push("[PREFERIDA]");
            }
            let badge_str = badges.join(" ");

            let row_style = if is_sel {
                Style::default().fg(pal.accent).add_modifier(Modifier::REVERSED)
            } else if m.is_current {
                Style::default().fg(pal.ok)
            } else {
                Style::default().fg(pal.fg)
            };

            Row::new(vec![
                bullet,
                Span::styled(res_str, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(rate_str, Style::default()),
                Span::styled(badge_str, Style::default().fg(pal.dim)),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(14),
        Constraint::Length(12),
        Constraint::Min(16),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(pal.accent))
                .title(Span::styled(
                    format!(" Resoluções de {} ({} modos) — [j/k] Selecionar [Enter] Aplicar ", selected_display.name, modes.len()),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )),
        );

    f.render_widget(table, area);
}

fn draw_footer(pal: &Palette, f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" [e] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Expandir Dir  ", Style::default().fg(pal.dim)),
        Span::styled("[E] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Expandir Esq  ", Style::default().fg(pal.dim)),
        Span::styled("[m] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Espelhar  ", Style::default().fg(pal.dim)),
        Span::styled("[x] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Só Ext  ", Style::default().fg(pal.dim)),
        Span::styled("[i] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Só Int  ", Style::default().fg(pal.dim)),
        Span::styled("[p] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Primário  ", Style::default().fg(pal.dim)),
        Span::styled("[h/l] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Tela  ", Style::default().fg(pal.dim)),
        Span::styled("[j/k] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Modo  ", Style::default().fg(pal.dim)),
        Span::styled("[Enter] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Aplicar", Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(" Controles & Atalhos ");

    f.render_widget(Paragraph::new(line).block(block), area);
}
