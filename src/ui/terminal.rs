//! Aba 8 — Terminal Deck (PTY + parser VT100). Render do Módulo 8.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, PtyState};
use crate::events::PtyScreenSnapshot;

use super::theme::Palette;
use super::widgets::pty_cell_style;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    match &app.terminal_pty {
        PtyState::Starting => super::draw_pending(
            app,
            pal,
            f,
            area,
            m.tab_terminal,
            "pty",
            &[m.terminal_starting],
        ),
        PtyState::Unavailable(reason) => super::draw_pending(
            app,
            pal,
            f,
            area,
            m.terminal_unavailable_title,
            "pty",
            &[reason.as_str()],
        ),
        PtyState::Exited => super::draw_pending(
            app,
            pal,
            f,
            area,
            m.tab_terminal,
            "pty",
            &[m.terminal_exited],
        ),
        PtyState::Running(screen) => draw_running(app, screen, pal, f, area),
    }
}

fn draw_running(app: &App, screen: &PtyScreenSnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let chunks = Layout::vertical([
        Constraint::Length(1), // header: sessão
        Constraint::Min(3),    // grade VT100
        Constraint::Length(1), // footer: foco/atalhos
    ])
    .split(area);

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "shell".to_string());
    let header = Line::from(vec![
        Span::styled(
            format!(" {shell} "),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}x{}", screen.cols, screen.rows),
            Style::default().fg(pal.dim),
        ),
    ]);
    f.render_widget(Paragraph::new(header), chunks[0]);

    let block = super::content_block(m.tab_terminal, pal);
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);
    draw_grid(screen, app.pty_focused, pal, f, inner);

    let focused = app.pty_focused;
    let status = if focused {
        m.pty_status_focused
    } else {
        m.pty_status_navigation
    };
    let hint = if focused {
        m.pty_hint_unfocus
    } else {
        m.pty_hint_focus
    };
    let footer = Line::from(vec![
        Span::styled(
            format!(" {status} "),
            Style::default()
                .fg(if focused { pal.ok } else { pal.dim })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(hint, Style::default().fg(pal.dim)),
    ]);
    f.render_widget(Paragraph::new(footer), chunks[2]);
}

/// Renderiza a grade de células `screen` dentro de `area`, invertendo a
/// célula do cursor quando o PTY está em foco e o cursor é visível.
pub(crate) fn draw_grid(
    screen: &PtyScreenSnapshot,
    focused: bool,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let (cur_row, cur_col) = screen.cursor;
    let rows_to_draw = (area.height as usize).min(screen.cells.len());
    let lines: Vec<Line> = screen
        .cells
        .iter()
        .take(rows_to_draw)
        .enumerate()
        .map(|(ri, row)| {
            let cols_to_draw = (area.width as usize).min(row.len());
            let spans: Vec<Span> = row
                .iter()
                .take(cols_to_draw)
                .enumerate()
                .map(|(ci, cell)| {
                    let mut style = pty_cell_style(cell, pal);
                    if focused
                        && screen.cursor_visible
                        && ri as u16 == cur_row
                        && ci as u16 == cur_col
                    {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    Span::styled(cell.ch.to_string(), style)
                })
                .collect();
            Line::from(spans)
        })
        .collect();
    f.render_widget(Paragraph::new(lines), area);
}
