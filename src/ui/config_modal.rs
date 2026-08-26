
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::i18n::Language;
use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame) {
    let area = centered(68, 62, f.area());
    f.render_widget(Clear, area);

    let lang_display = match app.config.ui.language.to_lowercase().as_str() {
        "pt-br" | "pt" => "Português (Brasil)",
        "en-us" | "en" => "English (US)",
        "es-es" | "es" => "Español",
        _ => match app.lang {
            Language::PtBr => "Auto (Português)",
            Language::EnUs => "Auto (English)",
            Language::EsEs => "Auto (Español)",
        },
    };

    let theme_display = match app.config.theme.name.to_lowercase().as_str() {
        "catppuccin" | "mocha" => "Catppuccin (Mocha)",
        "tokyo-night" | "tokyonight" => "Tokyo Night",
        "nord" => "Nord (Arctic)",
        "gruvbox" => "Gruvbox (Dark)",
        "cyberpunk" => "Cyberpunk (Neon)",
        "dracula" => "Dracula",
        "mono" => "Monocromático",
        _ => "HAL-9001 (Âmbar Sci-Fi)",
    };

    let logo_display = match app.config.overview.ascii.as_str() {
        "main" | "a" => "main (Grande)",
        "medium" | "c" => "medium (Média)",
        "compact" | "b" => "compact (Compacta)",
        "none" => "none (Sem logo)",
        _ => "auto (Responsiva)",
    };

    let icons_display = if app.config.ui.icons {
        if app.lang == Language::EnUs {
            "Enabled (Nerd Fonts)"
        } else {
            "Ativado (Nerd Fonts)"
        }
    } else if app.lang == Language::EnUs {
        "Disabled (ASCII)"
    } else {
        "Desativado (ASCII)"
    };

    let fps_display = match app.config.ui.frame_ms {
        16 => "60 FPS (~16ms)",
        66 => "15 FPS (~66ms)",
        _ => "30 FPS (~33ms)",
    };

    let splash_display = if app.config.splash.enabled {
        if app.lang == Language::EnUs {
            "Enabled"
        } else {
            "Ativada"
        }
    } else if app.lang == Language::EnUs {
        "Disabled"
    } else {
        "Desativada"
    };

    let polling_display = match app.config.polling.system_ms {
        750 => if app.lang == Language::EnUs { "Performance (0.7s)" } else { "Desempenho (0.7s)" },
        3000 => if app.lang == Language::EnUs { "Eco / Battery (3.0s)" } else { "Econômico (3.0s)" },
        _ => if app.lang == Language::EnUs { "Balanced (1.5s)" } else { "Equilibrado (1.5s)" },
    };

    let labels = match app.lang {
        Language::EnUs => [
            "Language",
            "Theme Palette",
            "ASCII Logo",
            "Nerd Font Icons",
            "Frame Rate",
            "Splash Screen",
            "Hardware Polling",
        ],
        Language::EsEs => [
            "Idioma",
            "Paleta de Tema",
            "Logo ASCII",
            "Iconos Nerd Font",
            "Tasa de Cuadros",
            "Pantalla de Inicio",
            "Sondeo de Hardware",
        ],
        Language::PtBr => [
            "Idioma",
            "Paleta de Tema",
            "Logo ASCII",
            "Ícones Nerd Font",
            "Taxa de Quadros",
            "Splash Screen",
            "Telemetria / Polling",
        ],
    };

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
            Span::styled(prefix, Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{label:<22} "), style_label),
            Span::styled(format!("◄ {val} ►"), style_val),
        ]));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(vec![
        Span::styled(" Amostra da Paleta: ", Style::default().fg(pal.dim)),
        Span::styled(" [● ACCENT] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled(" [● OK] ", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)),
        Span::styled(" [● WARN] ", Style::default().fg(pal.warn).add_modifier(Modifier::BOLD)),
        Span::styled(" [● ERR] ", Style::default().fg(pal.err).add_modifier(Modifier::BOLD)),
    ]));
    lines.push(Line::from(""));

    let footer_text = match app.lang {
        Language::EnUs => "[↑/↓] Navigate  [←/→/Enter] Change  [s] Save to disk  [Esc/c] Close",
        Language::EsEs => "[↑/↓] Navegar  [←/→/Enter] Cambiar  [s] Guardar  [Esc/c] Cerrar",
        Language::PtBr => "[↑/↓] Navegar  [←/→/Enter] Alterar  [s] Salvar no disco  [Esc/c] Fechar",
    };

    lines.push(Line::from(Span::styled(
        footer_text,
        Style::default().fg(pal.dim),
    )));

    let modal_title = match app.lang {
        Language::EnUs => " Settings & Theme Preferences ",
        Language::EsEs => " Configuración y Preferencias de Tema ",
        Language::PtBr => " Configurações & Preferências de Tema ",
    };

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
