use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::app::App;
use crate::backend::updates::Distro;

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let distro = match Distro::detect() {
        Distro::Arch => m.updates_distro_arch,
        Distro::Debian => m.updates_distro_debian,
        Distro::Unknown => m.updates_distro_unknown,
    };

    super::draw_pending(
        app,
        pal,
        f,
        area,
        m.updates_pending_title,
        "updates",
        &[
            m.updates_pending_count,
            m.updates_pending_list,
            m.updates_pending_run,
        ],
    );

    let footer = Rect {
        x: area.x + 2,
        y: area.y + area.height.saturating_sub(2),
        width: area.width.saturating_sub(4),
        height: 1,
    };
    let line = Line::from(vec![
        Span::styled(
            format!("{} ", m.updates_label_detected_distro),
            style_dim(pal),
        ),
        Span::styled(distro.to_string(), style_fg(pal)),
    ]);
    f.render_widget(ratatui::widgets::Paragraph::new(line), footer);
}

fn style_dim(pal: &Palette) -> ratatui::style::Style {
    ratatui::style::Style::default().fg(pal.dim)
}
fn style_fg(pal: &Palette) -> ratatui::style::Style {
    ratatui::style::Style::default().fg(pal.fg)
}
