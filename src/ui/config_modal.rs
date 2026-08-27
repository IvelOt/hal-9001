use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use super::theme::Palette;
use crate::app::App;
use crate::i18n::Language;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame) {
    let area = centered(68, 62, f.area());
    f.render_widget(Clear, area);

    let m = app.lang.messages();

    let lang_display = match app.config.ui.language.to_lowercase().as_str() {
        "pt-br" | "pt" => m.cfg_lang_pt_br,
        "en-us" | "en" => m.cfg_lang_en_us,
        "es-es" | "es" => m.cfg_lang_es_es,
        _ => match app.lang {
            Language::PtBr => m.cfg_lang_auto_pt,
            Language::EnUs => m.cfg_lang_auto_en,
            Language::EsEs => m.cfg_lang_auto_es,
        },
    };

    let theme_display = match app.config.theme.name.to_lowercase().as_str() {
        "catppuccin" | "mocha" => "Catppuccin (Mocha)",
        "tokyo-night" | "tokyonight" => "Tokyo Night",
        "nord" => "Nord (Arctic)",
        "gruvbox" => "Gruvbox (Dark)",
        "cyberpunk" => "Cyberpunk (Neon)",
        "dracula" => "Dracula",
        "mono" => m.cfg_theme_mono,
        _ => m.cfg_theme_default,
    };

    let logo_display = match app.config.overview.ascii.as_str() {
        "main" | "a" => m.cfg_logo_main,
        "medium" | "c" => m.cfg_logo_medium,
        "compact" | "b" => m.cfg_logo_compact,
        "none" => m.cfg_logo_none,
        _ => m.cfg_logo_auto,
    };

    let icons_display = if app.config.ui.icons {
        m.cfg_icons_enabled
    } else {
        m.cfg_icons_disabled
    };

    let fps_display = match app.config.ui.frame_ms {
        16 => "60 FPS (~16ms)",
        66 => "15 FPS (~66ms)",
        _ => "30 FPS (~33ms)",
    };

    let splash_display = if app.config.splash.enabled {
        m.cfg_splash_enabled
    } else {
        m.cfg_splash_disabled
    };

    let polling_display = match app.config.polling.system_ms {
        750 => m.cfg_polling_performance,
        3000 => m.cfg_polling_eco,
        _ => m.cfg_polling_balanced,
    };

    let labels = [
        m.cfg_lbl_language,
        m.cfg_lbl_theme,
        m.cfg_lbl_ascii,
        m.cfg_lbl_icons,
        m.cfg_lbl_fps,
        m.cfg_lbl_splash,
        m.cfg_lbl_polling,
    ];

    let values = [
        lang_display,
        theme_display,
        logo_display,
        icons_display,
        fps_display,
        splash_display,
        polling_display,
    ];

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (idx, (label, val)) in labels.iter().zip(values.iter()).enumerate() {
        let is_selected = app.config_cursor == idx;
        let prefix = if is_selected { " ▶ " } else { "   " };
        let style_label = if is_selected {
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.fg)
        };
        let style_val = if is_selected {
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.dim)
        };

        lines.push(Line::from(vec![
            Span::styled(
                prefix,
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{label:<22} "), style_label),
            Span::styled(format!("◄ {val} ►"), style_val),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(vec![
        Span::styled(m.cfg_palette_sample, Style::default().fg(pal.dim)),
        Span::styled(
            " [● ACCENT] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " [● OK] ",
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " [● WARN] ",
            Style::default().fg(pal.warn).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            " [● ERR] ",
            Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    let footer_text = m.cfg_modal_footer;

    lines.push(Line::from(Span::styled(
        footer_text,
        Style::default().fg(pal.dim),
    )));

    let modal_title = m.cfg_modal_title;

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            modal_title,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .alignment(Alignment::Center),
        area,
    );
}

fn centered(pw: u16, ph: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - ph) / 2),
        Constraint::Percentage(ph),
        Constraint::Percentage((100 - ph) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pw) / 2),
        Constraint::Percentage(pw),
        Constraint::Percentage((100 - pw) / 2),
    ])
    .split(v[1])[1]
}
