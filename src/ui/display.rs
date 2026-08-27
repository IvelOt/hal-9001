use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::backend::display::{DisplayLayoutMode, DisplayNode, DisplaySnapshot};

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let Some(snap) = &app.displays else {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            m.display_pending_title,
            "display",
            &[
                m.display_pending_extend,
                m.display_pending_mirror_ext,
                m.display_pending_notebook_primary,
            ],
        );
        return;
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(9),
            Constraint::Min(9),
            Constraint::Length(3),
        ])
        .split(area);

    draw_header(snap, m, pal, f, chunks[0]);
    draw_monitor_canvas(snap, app, m, pal, f, chunks[1]);
    draw_inspector_and_modes(snap, app, m, pal, f, chunks[2]);
    draw_footer(m, pal, f, chunks[3]);
}

fn draw_header(
    snap: &DisplaySnapshot,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let layout_badge = if let Some(l) = snap.current_layout {
        l.title_in(m)
    } else {
        m.display_mode_custom
    };

    let spans = vec![
        Span::styled(
            format!(" {} ", m.display_label_connected),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("{} {}", snap.connected_count, m.display_screens_suffix),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   {} ", m.display_label_layout),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("[ {layout_badge} ]"),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   {} ", m.display_label_primary),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            snap.primary_name.as_deref().unwrap_or(m.display_none),
            Style::default().fg(pal.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("   {} ", m.display_label_server),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            if snap.server_type.is_empty() {
                m.display_x11_fallback
            } else {
                &snap.server_type
            },
            Style::default().fg(pal.accent),
        ),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            m.display_title_header,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

fn draw_monitor_canvas(
    snap: &DisplaySnapshot,
    app: &App,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let connected = snap.connected_displays();

    if connected.is_empty() {
        let p = Paragraph::new(m.display_no_video_output).block(
            Block::default()
                .borders(Borders::ALL)
                .title(m.display_title_canvas),
        );
        f.render_widget(p, area);
        return;
    }

    let sel_idx = app.display_selected.min(connected.len().saturating_sub(1));

    let mut constraints = Vec::new();
    for _ in 0..connected.len() {
        constraints.push(Constraint::Ratio(1, connected.len() as u32));
    }

    let monitor_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, d) in connected.iter().enumerate() {
        let is_selected = i == sel_idx;
        draw_single_monitor_box(d, is_selected, m, pal, f, monitor_cols[i]);
    }
}

fn draw_single_monitor_box(
    d: &DisplayNode,
    is_selected: bool,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let border_color = if is_selected { pal.accent } else { pal.dim };

    let title_badge = if is_selected {
        format!(" ▶ {} {} ", d.name, m.display_selected_badge)
    } else if d.is_primary {
        format!(" ● {} {} ", d.name, m.display_primary_badge)
    } else {
        format!(" {} ", d.name)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if is_selected {
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(border_color)
        })
        .title(Span::styled(
            title_badge,
            if is_selected {
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
            } else if d.is_primary {
                Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(pal.fg)
            },
        ));

    let type_str = if d.is_internal {
        m.display_type_internal
    } else {
        m.display_type_external
    };

    let status_span = if d.is_primary {
        Span::styled(
            format!(
                "[● {}] ",
                m.display_primary_badge
                    .trim_matches(|c| c == '[' || c == ']')
            ),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        )
    } else if d.is_active {
        Span::styled(
            format!("{} ", m.display_active_badge),
            Style::default().fg(pal.accent),
        )
    } else {
        Span::styled(
            format!("{} ", m.display_disabled_badge),
            Style::default().fg(pal.warn),
        )
    };

    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {} ", m.display_label_type),
                Style::default().fg(pal.dim),
            ),
            Span::styled(type_str, Style::default().fg(pal.fg)),
            Span::raw("   "),
            status_span,
        ]),
        Line::from(vec![
            Span::styled(
                format!(" {} ", m.display_label_resolution),
                Style::default().fg(pal.dim),
            ),
            Span::styled(
                d.resolution_str_in(m),
                Style::default()
                    .fg(if is_selected { pal.ok } else { pal.fg })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(
                format!(" {} ", m.display_label_virtual_pos),
                Style::default().fg(pal.dim),
            ),
            Span::styled(
                format!("X: {}, Y: {}", d.pos_x, d.pos_y),
                Style::default().fg(pal.dim),
            ),
            Span::styled(
                format!("   {} ", m.display_label_rotation),
                Style::default().fg(pal.dim),
            ),
            Span::styled(&d.rotation, Style::default().fg(pal.fg)),
        ]),
        Line::from(vec![Span::styled(
            if is_selected {
                m.display_hint_switch_focus
            } else {
                m.display_hint_select_this
            },
            Style::default().fg(pal.dim),
        )]),
    ];

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_inspector_and_modes(
    snap: &DisplaySnapshot,
    app: &App,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let sub_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);

    draw_modes_card(snap, m, pal, f, sub_chunks[0]);
    draw_resolutions_inspector(snap, app, m, pal, f, sub_chunks[1]);
}

fn draw_modes_card(
    snap: &DisplaySnapshot,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let current = snap.current_layout;

    let modes = [
        (
            DisplayLayoutMode::ExtendRight,
            "[e]",
            format!(
                "{}{}",
                m.display_mode_extend_right, m.display_default_suffix
            ),
        ),
        (
            DisplayLayoutMode::ExtendLeft,
            "[E]",
            m.display_mode_extend_left.to_string(),
        ),
        (
            DisplayLayoutMode::Mirror,
            "[m]",
            format!("{}{}", m.display_mode_mirror, m.display_duplicate_suffix),
        ),
        (
            DisplayLayoutMode::ExternalOnly,
            "[x]",
            m.display_mode_external_only.to_string(),
        ),
        (
            DisplayLayoutMode::InternalOnly,
            "[i]",
            m.display_mode_internal_only.to_string(),
        ),
    ];

    let mut lines = Vec::new();
    for (mode, key, desc) in modes {
        let is_active = current == Some(mode);
        let bullet = if is_active { "● " } else { "○ " };
        let style = if is_active {
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.fg)
        };

        lines.push(Line::from(vec![
            Span::styled(format!(" {bullet}"), style),
            Span::styled(
                format!("{key} "),
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("{:<28}", desc), style),
            if is_active {
                Span::styled(
                    m.display_active_tag,
                    Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw("")
            },
        ]));
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(Span::styled(
            m.display_title_modes,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(lines).block(block), area);
}

fn draw_resolutions_inspector(
    snap: &DisplaySnapshot,
    app: &App,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let connected = snap.connected_displays();
    let sel_idx = app.display_selected.min(connected.len().saturating_sub(1));

    let Some(selected_display) = connected.get(sel_idx) else {
        let p = Paragraph::new(m.display_title_no_monitor).block(
            Block::default()
                .borders(Borders::ALL)
                .title(m.display_title_resolutions),
        );
        f.render_widget(p, area);
        return;
    };

    let modes = &selected_display.supported_modes;
    let sel_res_idx = app.display_res_selected.min(modes.len().saturating_sub(1));

    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            m.display_col_resolution,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.display_col_rate,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.display_col_status,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(0);

    let rows: Vec<Row> = modes
        .iter()
        .enumerate()
        .map(|(i, mode)| {
            let is_sel = i == sel_res_idx;
            let bullet = if is_sel {
                Span::styled(
                    "▶ ",
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )
            } else if mode.is_current {
                Span::styled("● ", Style::default().fg(pal.ok))
            } else {
                Span::raw("  ")
            };

            let res_str = format!("{}x{}", mode.width, mode.height);
            let rate_str = format!("{:.2} Hz", mode.rate);

            let mut badges = Vec::new();
            if mode.is_current {
                badges.push(m.display_tag_current);
            }
            if mode.is_preferred {
                badges.push(m.display_tag_preferred);
            }
            let badge_str = badges.join(" ");

            let row_style = if is_sel {
                Style::default()
                    .fg(pal.accent)
                    .add_modifier(Modifier::REVERSED)
            } else if mode.is_current {
                Style::default().fg(pal.ok)
            } else {
                Style::default().fg(pal.fg)
            };

            Row::new(vec![
                bullet,
                Span::styled(res_str, Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(rate_str, Style::default()),
                Span::styled(badge_str, Style::default().fg(pal.dim)),
            ])
            .style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(14),
        Constraint::Length(12),
        Constraint::Min(16),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(pal.accent))
            .title(Span::styled(
                m.display_title_resolutions_of
                    .replace("{name}", &selected_display.name)
                    .replace("{count}", &modes.len().to_string()),
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, area);
}

fn draw_footer(m: &'static crate::i18n::Messages, pal: &Palette, f: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " [e] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_footer_extend_right),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[E] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_footer_extend_left),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[m] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_footer_mirror),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[x] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_footer_ext_only),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[i] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_footer_int_only),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[p] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_label_primary.trim_end_matches(':')),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[h/l] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_footer_screen),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[j/k] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}  ", m.display_footer_mode),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[Enter] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(m.display_footer_apply, Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(m.display_footer_title);

    f.render_widget(Paragraph::new(line).block(block), area);
}
