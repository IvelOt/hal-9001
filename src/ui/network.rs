use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Row, Table, Wrap};
use ratatui::Frame;

use crate::app::{App, WifiPasswordPromptState};
use crate::backend::network::{AccessPoint, NetworkSnapshot, Security, WifiBand};

use super::storage::modal_block;
use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let Some(snap) = &app.network else {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            m.network_pending_title,
            "network",
            &[
                m.network_pending_connect,
                m.network_pending_disconnect_forget,
                m.network_pending_rescan_radio,
                m.network_pending_telemetry,
            ],
        );
        return;
    };

    if !snap.nm_available {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            m.network_pending_title,
            "network",
            &[m.network_err_nm_unavailable],
        );
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(4),
        ])
        .split(area);

    draw_header(snap, app, pal, f, chunks[0]);
    draw_ap_list(snap, app, pal, f, chunks[1]);
    draw_footer(snap, app.lang.messages(), pal, f, chunks[2]);

    if let Some(prompt) = &app.wifi_prompt {
        draw_wifi_prompt(app.lang.messages(), pal, f, prompt);
    }
}

fn draw_header(snap: &NetworkSnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let radio_style = if snap.wireless_enabled {
        Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.err).add_modifier(Modifier::BOLD)
    };
    let radio_text = if snap.wireless_enabled {
        m.network_radio_on
    } else {
        m.network_radio_off
    };

    let iface = snap
        .wifi_device
        .as_ref()
        .map(|d| d.iface.as_str())
        .unwrap_or(m.network_none);

    let scanning_badge = if app.network_scanning {
        Span::styled(
            m.network_scanning_badge,
            Style::default().fg(pal.warn).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };

    let active_desc = if let Some(act) = &snap.active {
        Span::styled(
            format!(" {} {} ", m.network_connected_prefix, act.ssid),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {} ", m.network_disconnected),
            Style::default().fg(pal.dim),
        )
    };

    let header_line = Line::from(vec![
        Span::styled(
            format!(" {} ", m.network_label_adapter),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            iface,
            Style::default().fg(pal.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} ", m.network_label_radio),
            Style::default().fg(pal.dim),
        ),
        Span::styled(radio_text, radio_style),
        Span::raw("  "),
        active_desc,
        scanning_badge,
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            m.network_title_header,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(header_line).block(block), area);
}

fn draw_ap_list(snap: &NetworkSnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            m.network_col_ssid,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.network_col_signal,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.network_col_band,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.network_col_security,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.network_col_status,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(1);

    if snap.access_points.is_empty() {
        let empty_msg = if !snap.wireless_enabled {
            m.network_empty_radio_off
        } else {
            m.network_empty_no_networks
        };
        let p = Paragraph::new(Line::from(Span::styled(
            empty_msg,
            Style::default().fg(pal.dim),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(pal.dim))
                .title(format!(" {} ", m.network_title_available)),
        );
        f.render_widget(p, area);
        return;
    }

    let selected_idx = app
        .network_selected
        .min(snap.access_points.len().saturating_sub(1));

    let rows: Vec<Row> = snap
        .access_points
        .iter()
        .enumerate()
        .map(|(i, ap)| {
            let is_sel = i == selected_idx;
            format_ap_row(ap, is_sel, pal, m)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Percentage(32),
        Constraint::Percentage(24),
        Constraint::Percentage(14),
        Constraint::Percentage(16),
        Constraint::Percentage(14),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(pal.accent))
            .title(Span::styled(
                format!(
                    " {} ({}) ",
                    m.network_title_available,
                    snap.access_points.len()
                ),
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, area);
}

fn format_ap_row<'a>(
    ap: &AccessPoint,
    is_sel: bool,
    pal: &Palette,
    m: &'static crate::i18n::Messages,
) -> Row<'a> {
    let bullet = if ap.is_active {
        Span::styled(
            "● ",
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        )
    } else if is_sel {
        Span::styled(
            "▶ ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("  ")
    };

    let ssid_style = if ap.is_active {
        Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
    } else if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.fg)
    };

    let signal_bar = format_signal_bar(ap.strength, pal);
    let band_badge = match ap.band {
        WifiBand::Ghz5 => Span::styled("5 GHz", Style::default().fg(pal.accent)),
        WifiBand::Ghz24 => Span::styled("2.4 GHz", Style::default().fg(pal.dim)),
        WifiBand::Ghz6 => Span::styled("6 GHz", Style::default().fg(pal.ok)),
        WifiBand::Unknown => Span::styled("-", Style::default().fg(pal.dim)),
    };

    let sec_style = match ap.security {
        Security::Open => Style::default().fg(pal.warn),
        Security::Wpa2 | Security::Wpa3 => Style::default().fg(pal.accent),
        _ => Style::default().fg(pal.dim),
    };
    let sec_badge = Span::styled(ap.security.label(), sec_style);

    let status_badge = if ap.is_active {
        Span::styled(
            m.network_status_connected,
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        )
    } else if ap.is_saved {
        Span::styled(m.network_status_saved, Style::default().fg(pal.accent))
    } else {
        Span::styled("-", Style::default().fg(pal.dim))
    };

    let row_style = if is_sel {
        Style::default()
            .fg(pal.accent)
            .add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    Row::new(vec![
        bullet,
        Span::styled(ap.ssid.clone(), ssid_style),
        signal_bar,
        band_badge,
        sec_badge,
        status_badge,
    ])
    .style(row_style)
}

fn format_signal_bar<'a>(strength: u8, pal: &Palette) -> Span<'a> {
    let bars = match strength {
        0..=20 => "▂     ",
        21..=40 => "▃▄    ",
        41..=60 => "▅▆    ",
        61..=80 => "▆▇    ",
        _ => "▇█    ",
    };

    let color = if strength >= 70 {
        pal.ok
    } else if strength >= 40 {
        pal.warn
    } else {
        pal.err
    };

    Span::styled(
        format!("[{bars}] {strength:>3}%"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn draw_footer(
    snap: &NetworkSnapshot,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let t = &snap.telemetry;
    let ip = t.ipv4.as_deref().unwrap_or("—");
    let gw = t.gateway.as_deref().unwrap_or("—");

    let rx_str = format_rate(t.rx_rate_kbps);
    let tx_str = format_rate(t.tx_rate_kbps);

    let line1 = Line::from(vec![
        Span::styled(
            format!(" {} ", m.network_label_ip),
            Style::default().fg(pal.dim),
        ),
        Span::styled(ip, Style::default().fg(pal.fg).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("   {} ", m.network_label_gateway),
            Style::default().fg(pal.dim),
        ),
        Span::styled(gw, Style::default().fg(pal.fg).add_modifier(Modifier::BOLD)),
        Span::styled(
            format!("   {} ", m.network_label_rate),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("↓ {rx_str}"),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("↑ {tx_str}"),
            Style::default().fg(pal.warn).add_modifier(Modifier::BOLD),
        ),
    ]);

    let line2 = Line::from(vec![
        Span::styled(
            " [Enter] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.network_hint_connect),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[d] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.network_hint_disconnect),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[f] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.network_hint_forget),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[r] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.network_hint_scan),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[t] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(m.network_hint_toggle_radio, Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(m.network_footer_title);

    f.render_widget(Paragraph::new(vec![line1, line2]).block(block), area);
}

fn format_rate(rate_kbps: f64) -> String {
    if rate_kbps >= 1024.0 {
        format!("{:.1} MB/s", rate_kbps / 1024.0)
    } else {
        format!("{:.1} KB/s", rate_kbps)
    }
}

pub fn draw_wifi_prompt(
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    prompt: &WifiPasswordPromptState,
) {
    let area = super::centered(54, 28, f.area());
    f.render_widget(Clear, area);

    let block = modal_block(m.network_wifi_auth_title, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", m.network_label_network),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            &prompt.ssid,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    let masked = "*".repeat(prompt.password.chars().count());
    lines.push(Line::from(vec![
        Span::styled(
            format!("{} ", m.network_label_password),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{masked}▏"),
            Style::default().fg(pal.bg).bg(pal.accent),
        ),
    ]));
    lines.push(Line::from(""));

    if let Some(err) = &prompt.error {
        lines.push(Line::from(Span::styled(
            err.as_str(),
            Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        m.network_hint_connect_cancel,
        Style::default().fg(pal.dim),
    )));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
