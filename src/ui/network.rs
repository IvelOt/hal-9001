//! Aba 2 — Wi-Fi / Rede (NetworkManager). Render do Módulo 2.

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
        "Wi-Fi / Rede",
        "network",
        &[
            "[Enter] conectar (modal de senha)",
            "[d] desconectar   [f] esquecer",
            "[r] rescan   [t] ligar/desligar rádio",
            "IP local, gateway e taxa ↓/↑",
        ],
    );
}
