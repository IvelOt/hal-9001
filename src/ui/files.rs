//! Aba 7 — Gerenciador de Arquivos (Yazi embutido via PTY). Render do Módulo 7.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{App, PtyState};
use crate::events::PtyScreenSnapshot;

use super::terminal::draw_grid;
use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    match &app.files_pty {
        PtyState::Starting => super::draw_pending(
            app,
            pal,
            f,
            area,
            m.tab_files,
            "pty",
            &[m.files_starting],
        ),
        PtyState::Unavailable(_) => draw_unavailable(app, pal, f, area),
        PtyState::Exited => super::draw_pending(
            app,
            pal,
            f,
            area,
            m.tab_files,
            "pty",
            &[m.files_exited],
        ),
        PtyState::Running(screen) => draw_running(app, screen, pal, f, area),
    }
}

/// Cartão informativo exibido quando `yazi` não foi encontrado no `$PATH`:
/// instruções de instalação para Arch Linux (pacman) e via `cargo install`.
fn draw_unavailable(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    super::draw_pending(
        app,
        pal,
        f,
        area,
        m.files_unavailable_title,
        "pty",
        &[m.files_unavailable_install_pacman, m.files_unavailable_install_cargo],
    );
}

fn draw_running(app: &App, screen: &PtyScreenSnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let chunks = Layout::vertical([
        Constraint::Length(1), // header: sessão
        Constraint::Min(3),    // grade VT100 (yazi)
        Constraint::Length(1), // footer: foco/atalhos
    ])
    .split(area);

    let header = Line::from(vec![
        Span::styled(
            " yazi ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}x{}", screen.cols, screen.rows),
            Style::default().fg(pal.dim),
        ),
    ]);
    f.render_widget(Paragraph::new(header), chunks[0]);

    let block = super::content_block(m.tab_files, pal);
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
