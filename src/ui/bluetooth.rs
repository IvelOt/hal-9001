//! Aba 3 — Bluetooth (bluez). Render do Módulo 3.

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
        "Bluetooth",
        "bluetooth",
        &[
            "[s] scan on/off   [p] parear",
            "[Enter] conectar   [d] desconectar",
            "[x] remover   [t] ligar/desligar adaptador",
            "Bateria de fones/periféricos (Battery1)",
        ],
    );
}
