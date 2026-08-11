//! Widget TUI (Ratatui) do **AI Terminal Deck**.
//!
//! Renderiza a tela virtual do PTY (interpretada pelo parser `vt100`) dentro de
//! um painel com borda, mais uma linha de status com o estado do servidor IPC
//! (socket UNIX + consentimentos pendentes do gatekeeper).

use std::path::PathBuf;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Widget};

use crate::ai_agent::ipc_server::Gatekeeper;
use crate::ai_agent::pty_session::PtySession;

/// Estado agregado do AI Terminal Deck para renderização.
pub struct AiDeckState {
    /// Sessão PTY ativa (agente), se houver.
    pub session: Option<Arc<PtySession>>,
    /// Gatekeeper compartilhado com o servidor IPC (contagem de consentimentos).
    pub gatekeeper: Option<Gatekeeper>,
    /// Caminho do socket UNIX do servidor IPC, se iniciado.
    pub ipc_socket: Option<PathBuf>,
    /// `true` se o servidor IPC está aceitando conexões.
    pub ipc_listening: bool,
}

impl Default for AiDeckState {
    fn default() -> Self {
        Self {
            session: None,
            gatekeeper: None,
            ipc_socket: None,
            ipc_listening: false,
        }
    }
}

/// Widget que desenha o AI Terminal Deck num `Rect` dado.
pub struct AiDeckWidget<'a> {
    state: &'a AiDeckState,
}

impl<'a> AiDeckWidget<'a> {
    pub fn new(state: &'a AiDeckState) -> Self {
        Self { state }
    }
}

impl Widget for AiDeckWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let block = Block::bordered().title(" AI TERMINAL DECK ");
        let inner = block.inner(area);
        block.render(area, buf);

        let layout = Layout::vertical([Constraint::Min(1), Constraint::Length(1)])
            .split(inner);
        let (screen_area, status_area) = (layout[0], layout[1]);

        self.render_deck(screen_area, buf);
        self.render_status(status_area, buf);
    }
}

impl AiDeckWidget<'_> {
    /// Desenha a tela virtual do PTY (ou um placeholder quando inativa).
    fn render_deck(&self, area: Rect, buf: &mut Buffer) {
        let Some(session) = &self.state.session else {
            let placeholder = [
                Line::from(" Nenhuma sessão de agente ativa."),
                Line::from(" Inicie `opencode`/`claude`/`bash` para interagir no deck."),
            ];
            render_placeholder(&placeholder, area, buf);
            return;
        };

        if session.has_exited() {
            let message = Line::from(" O agente encerrou. Inicie uma nova sessão para continuar.");
            render_placeholder(&[message], area, buf);
            return;
        }

        session.with_screen(|screen| render_screen(screen, area, buf));
    }

    /// Linha de status com o estado do IPC e consentimentos pendentes.
    fn render_status(&self, area: Rect, buf: &mut Buffer) {
        let socket = self
            .state
            .ipc_socket
            .as_deref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "-".to_string());

        let listening = if self.state.ipc_listening { "ativo" } else { "inativo" };
        let pending = self
            .state
            .gatekeeper
            .as_ref()
            .map(|gk| gk.pending().len())
            .unwrap_or(0);

        let status = format!(
            " [IPC] {listening} · {socket} · consentimentos pendentes: {pending}"
        );
        buf.set_stringn(area.x, area.y, &status, area.width as usize, Style::new().fg(Color::Gray));
    }
}

/// Renderiza linhas de texto simples dentro da área da tela do deck.
fn render_placeholder(lines: &[Line<'_>], area: Rect, buf: &mut Buffer) {
    for (i, line) in lines.iter().enumerate() {
        if i as u16 >= area.height {
            break;
        }
        buf.set_stringn(
            area.x,
            area.y + i as u16,
            line.to_string(),
            area.width as usize,
            Style::new().fg(Color::Gray),
        );
    }
}

/// Copia a tela virtual `vt100` para o buffer do Ratatui, célula a célula.
fn render_screen(screen: &vt100::Screen, area: Rect, buf: &mut Buffer) {
    let (screen_rows, screen_cols) = screen.size();
    let rows = screen_rows.min(area.height);
    let cols = screen_cols.min(area.width);

    for y in 0..rows {
        for x in 0..cols {
            let Some(cell) = screen.cell(y, x) else {
                continue;
            };
            // Células de continuação de caracteres largos são puladas; o símbolo
            // de largura dupla já foi colocado na célula `is_wide`.
            if cell.is_wide_continuation() {
                continue;
            }

            let target = &mut buf[(area.x + x, area.y + y)];
            target.set_symbol(cell.contents());

            let mut style = Style::new();
            if cell.bold() {
                style = style.add_modifier(Modifier::BOLD);
            }
            if cell.dim() {
                style = style.add_modifier(Modifier::DIM);
            }
            if cell.italic() {
                style = style.add_modifier(Modifier::ITALIC);
            }
            if cell.underline() {
                style = style.add_modifier(Modifier::UNDERLINED);
            }
            if cell.inverse() {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if let Some(fg) = vt100_color(cell.fgcolor()) {
                style = style.fg(fg);
            }
            if let Some(bg) = vt100_color(cell.bgcolor()) {
                style = style.bg(bg);
            }
            target.set_style(style);
        }
    }

    // Cursor: invertido sobre a célula atual para destacar a posição do agente.
    if !screen.hide_cursor() && rows > 0 && cols > 0 {
        let (cursor_row, cursor_col) = screen.cursor_position();
        let x = area.x + cursor_col.min(cols - 1);
        let y = area.y + cursor_row.min(rows - 1);
        let cell = &mut buf[(x, y)];
        cell.modifier.insert(Modifier::REVERSED);
    }
}

/// Converte uma cor `vt100` para a cor correspondente do Ratatui.
fn vt100_color(color: vt100::Color) -> Option<Color> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Idx(index) => Some(Color::Indexed(index)),
        vt100::Color::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}
