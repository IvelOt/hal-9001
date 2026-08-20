//! Aba 7 — Gerenciador de Arquivos (Yazi embutido via PTY). Render do Módulo 7.

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
        "Arquivos (Yazi)",
        "pty",
        &[
            "Lança `yazi` embutido via PTY",
            "Suspensão/retorno de raw mode sem artefatos",
            "[Enter] focar   [Esc] devolver foco ao chrome",
            "Fallback: instruções se `yazi` ausente",
        ],
    );
}
