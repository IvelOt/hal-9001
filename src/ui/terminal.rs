//! Aba 8 — Terminal Deck (PTY + parser VT100). Render do Módulo 8.

use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    super::draw_pending(
        app,
        pal,
        f,
        area,
        "Terminal Deck",
        "pty",
        &[
            "Shell embutido via portable-pty",
            "Parser VT100 renderiza a grade",
            "[Enter] focar PTY   [Ctrl-a] leader de escape",
            "Base do AI Terminal Deck (múltiplas sessões)",
        ],
    );
}
