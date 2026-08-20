//! Aba 6 — Atualizações do Sistema. Render do Módulo 6.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::Frame;

use crate::app::App;
use crate::backend::updates::Distro;

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let distro = match Distro::detect() {
        Distro::Arch => "Arch (checkupdates / yay / paru)",
        Distro::Debian => "Debian/Ubuntu (apt)",
        Distro::Unknown => "desconhecida",
    };

    super::draw_pending(
        app,
        pal,
        f,
        area,
        "Atualizações do Sistema",
        "updates",
        &[
            "Contagem de pacotes pendentes",
            "Lista pacote: versão atual → nova",
            "[U] rodar atualização em PTY (visível)",
        ],
    );

    // Rodapé informativo com a distro detectada (linha extra sobre o painel).
    let footer = Rect {
        x: area.x + 2,
        y: area.y + area.height.saturating_sub(2),
        width: area.width.saturating_sub(4),
        height: 1,
    };
    let line = Line::from(vec![
        Span::styled("Distro detectada: ", style_dim(pal)),
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
