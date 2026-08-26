
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
        "Energia & Bateria",
        "power",
        &[
            "Saúde (capacidade atual/design), ciclos",
            "Consumo em W e tempo restante",
            "[←/→] perfil: performance/balanced/power-saver",
            "Sparkline de consumo recente",
        ],
    );
}
