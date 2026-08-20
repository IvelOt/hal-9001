//! Aba 4 — Discos & Armazenamento (UDisks2). Render do Módulo 4.

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
        "Discos & Armazenamento",
        "storage",
        &[
            "Árvore dispositivo → partições",
            "[m] montar   [u] desmontar   [e] ejetar",
            "Uso por montagem (barra)",
            "Destaque para USB removível",
        ],
    );
}
