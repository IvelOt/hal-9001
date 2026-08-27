use ratatui::layout::Rect;
use ratatui::Frame;

use crate::app::App;

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    super::draw_pending(
        app,
        pal,
        f,
        area,
        m.power_pending_title,
        "power",
        &[
            m.power_pending_health,
            m.power_pending_consumption,
            m.power_pending_profile,
            m.power_pending_sparkline,
        ],
    );
}
