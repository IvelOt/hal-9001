use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::backend::bluetooth::{BluetoothDevice, BluetoothSnapshot};

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let Some(snap) = &app.bluetooth else {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            m.bt_pending_title,
            "bluetooth",
            &[
                m.bt_pending_connect,
                m.bt_pending_pair_forget,
                m.bt_pending_scan_radio,
                m.bt_pending_telemetry,
            ],
        );
        return;
    };

    if !snap.bluez_available {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            m.bt_pending_title,
            "bluetooth",
            &[m.bt_err_bluez_unavailable],
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
    draw_device_list(snap, app, pal, f, chunks[1]);
    draw_footer(snap, m, pal, f, chunks[2]);
}

fn draw_header(snap: &BluetoothSnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let adapter = snap.adapter.as_ref();
    let is_powered = adapter.map(|a| a.powered).unwrap_or(false);

    let radio_style = if is_powered {
        Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.err).add_modifier(Modifier::BOLD)
    };
    let radio_text = if is_powered {
        m.network_radio_on
    } else {
        m.network_radio_off
    };

    let name = adapter.map(|a| a.name.as_str()).unwrap_or(m.bt_none);
    let mac = adapter.map(|a| a.address.as_str()).unwrap_or("—");

    let scanning_badge = if app.bluetooth_scanning {
        Span::styled(
            m.network_scanning_badge,
            Style::default().fg(pal.warn).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };

    let connected_count = snap.devices.iter().filter(|d| d.connected).count();
    let connected_badge = if connected_count > 0 {
        Span::styled(
            format!(" {} {connected_count} ", m.bt_connected_count_prefix),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {} ", m.bt_disconnected),
            Style::default().fg(pal.dim),
        )
    };

    let header_line = Line::from(vec![
        Span::styled(
            format!(" {} ", m.bt_label_adapter),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            name,
            Style::default().fg(pal.fg).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" (", Style::default().fg(pal.dim)),
        Span::styled(mac, Style::default().fg(pal.dim)),
        Span::styled(
            format!(")  {} ", m.network_label_radio),
            Style::default().fg(pal.dim),
        ),
        Span::styled(radio_text, radio_style),
        Span::raw("  "),
        connected_badge,
        scanning_badge,
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            m.bt_title_header,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(header_line).block(block), area);
}

fn draw_device_list(snap: &BluetoothSnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled(
            m.bt_col_type,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.bt_col_device,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.bt_col_mac,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.bt_col_signal,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.bt_col_battery,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            m.bt_col_status,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(1);

    if snap.devices.is_empty() {
        let is_powered = snap.adapter.as_ref().map(|a| a.powered).unwrap_or(false);
        let empty_msg = if !is_powered {
            m.bt_empty_radio_off
        } else {
            m.bt_empty_no_devices
        };
        let p = Paragraph::new(Line::from(Span::styled(
            empty_msg,
            Style::default().fg(pal.dim),
        )))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(pal.dim))
                .title(m.bt_title_devices),
        );
        f.render_widget(p, area);
        return;
    }

    let selected_idx = app
        .bluetooth_selected
        .min(snap.devices.len().saturating_sub(1));

    let rows: Vec<Row> = snap
        .devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let is_sel = i == selected_idx;
            format_device_row(dev, is_sel, pal, m)
        })
        .collect();

    let widths = [
        Constraint::Length(3),
        Constraint::Length(8),
        Constraint::Percentage(32),
        Constraint::Percentage(20),
        Constraint::Percentage(15),
        Constraint::Length(10),
        Constraint::Percentage(15),
    ];

    let table = Table::new(rows, widths).header(header).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(pal.accent))
            .title(Span::styled(
                format!(" {} ({}) ", m.bt_title_devices.trim(), snap.devices.len()),
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_widget(table, area);
}

fn format_device_row<'a>(
    dev: &BluetoothDevice,
    is_sel: bool,
    pal: &Palette,
    m: &'static crate::i18n::Messages,
) -> Row<'a> {
    let bullet = if dev.connected {
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

    let type_badge = Span::styled(
        dev.device_type.ascii_label(),
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
    );

    let name_style = if dev.connected {
        Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
    } else if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.fg)
    };

    let signal_span = format_rssi_span(dev.rssi, pal);

    let battery_span = if let Some(pct) = dev.battery_percentage {
        let color = if pct >= 50 {
            pal.ok
        } else if pct >= 20 {
            pal.warn
        } else {
            pal.err
        };
        Span::styled(
            format!("{pct:>3}%"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled("—", Style::default().fg(pal.dim))
    };

    let status_badge = if dev.blocked {
        Span::styled(
            m.bt_status_blocked,
            Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
        )
    } else if dev.connected {
        Span::styled(
            m.bt_status_connected,
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        )
    } else if dev.paired {
        Span::styled(m.bt_status_paired, Style::default().fg(pal.accent))
    } else {
        Span::styled(m.bt_status_available, Style::default().fg(pal.dim))
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
        type_badge,
        Span::styled(dev.alias.clone(), name_style),
        Span::styled(dev.address.clone(), Style::default().fg(pal.dim)),
        signal_span,
        battery_span,
        status_badge,
    ])
    .style(row_style)
}

fn format_rssi_span<'a>(rssi: Option<i16>, pal: &Palette) -> Span<'a> {
    let Some(r) = rssi else {
        return Span::styled("—", Style::default().fg(pal.dim));
    };

    let (bars, color) = if r >= -55 {
        ("▇█    ", pal.ok)
    } else if r >= -70 {
        ("▅▆    ", pal.ok)
    } else if r >= -85 {
        ("▃▄    ", pal.warn)
    } else {
        ("▂     ", pal.err)
    };

    Span::styled(
        format!("[{bars}] {r:>3} dBm"),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

fn draw_footer(
    snap: &BluetoothSnapshot,
    m: &'static crate::i18n::Messages,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
) {
    let connected_devices: Vec<&BluetoothDevice> =
        snap.devices.iter().filter(|d| d.connected).collect();

    let line1 = if !connected_devices.is_empty() {
        let mut spans = vec![Span::styled(
            format!(" {} ", m.bt_footer_active_prefix),
            Style::default().fg(pal.dim),
        )];
        for (i, d) in connected_devices.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" | ", Style::default().fg(pal.dim)));
            }
            spans.push(Span::styled(
                &d.alias,
                Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
            ));
            if let Some(pct) = d.battery_percentage {
                spans.push(Span::styled(
                    format!(" ({pct}%)"),
                    Style::default().fg(pal.accent),
                ));
            }
        }
        Line::from(spans)
    } else {
        Line::from(vec![
            Span::styled(
                format!(" {}: ", m.bt_col_status),
                Style::default().fg(pal.dim),
            ),
            Span::styled(m.bt_footer_no_devices, Style::default().fg(pal.dim)),
        ])
    };

    let line2 = Line::from(vec![
        Span::styled(
            " [Enter] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.bt_hint_connect_toggle),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[p] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.bt_hint_pair),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[r] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.bt_hint_scan),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[f] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.bt_hint_forget),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[t] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}   ", m.bt_hint_toggle_radio),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            "[b] ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(m.bt_hint_block, Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(m.bt_footer_title);

    f.render_widget(Paragraph::new(vec![line1, line2]).block(block), area);
}
