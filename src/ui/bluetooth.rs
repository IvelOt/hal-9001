
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Row, Table};
use ratatui::Frame;

use crate::app::App;
use crate::backend::bluetooth::{BluetoothDevice, BluetoothSnapshot};

use super::theme::Palette;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let Some(snap) = &app.bluetooth else {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            "Bluetooth",
            "bluetooth",
            &[
                "[Enter] conectar / desconectar",
                "[p] parear   [f] esquecer / remover",
                "[r] escanear   [t] ligar/desligar rádio",
                "Telemetria de bateria e sinal RSSI",
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
            "Bluetooth",
            "bluetooth",
            &["Serviço BlueZ (org.bluez) ou adaptador D-Bus indisponível"],
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
    draw_footer(snap, pal, f, chunks[2]);
}

fn draw_header(snap: &BluetoothSnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let adapter = snap.adapter.as_ref();
    let is_powered = adapter.map(|a| a.powered).unwrap_or(false);

    let radio_style = if is_powered {
        Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.err).add_modifier(Modifier::BOLD)
    };
    let radio_text = if is_powered {
        "[● LIGADO]"
    } else {
        "[○ DESLIGADO]"
    };

    let name = adapter.map(|a| a.name.as_str()).unwrap_or("Nenhum");
    let mac = adapter.map(|a| a.address.as_str()).unwrap_or("—");

    let scanning_badge = if app.bluetooth_scanning {
        Span::styled(" [BUSCANDO...] ", Style::default().fg(pal.warn).add_modifier(Modifier::BOLD))
    } else {
        Span::raw("")
    };

    let connected_count = snap.devices.iter().filter(|d| d.connected).count();
    let connected_badge = if connected_count > 0 {
        Span::styled(
            format!(" Conectados: {connected_count} "),
            Style::default().fg(pal.ok).add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(" Desconectado ", Style::default().fg(pal.dim))
    };

    let header_line = Line::from(vec![
        Span::styled(" Adaptador: ", Style::default().fg(pal.dim)),
        Span::styled(name, Style::default().fg(pal.fg).add_modifier(Modifier::BOLD)),
        Span::styled(" (", Style::default().fg(pal.dim)),
        Span::styled(mac, Style::default().fg(pal.dim)),
        Span::styled(")  Rádio: ", Style::default().fg(pal.dim)),
        Span::styled(radio_text, radio_style),
        Span::raw("  "),
        connected_badge,
        scanning_badge,
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            " Bluetooth & Dispositivos ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ));

    f.render_widget(Paragraph::new(header_line).block(block), area);
}

fn draw_device_list(snap: &BluetoothSnapshot, app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let header = Row::new(vec![
        Span::styled("  ", Style::default()),
        Span::styled("Tipo", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Dispositivo / Alias", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("MAC / ID", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Sinal", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Bateria", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Status", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
    ])
    .style(Style::default().fg(pal.accent))
    .bottom_margin(1);

    if snap.devices.is_empty() {
        let is_powered = snap.adapter.as_ref().map(|a| a.powered).unwrap_or(false);
        let empty_msg = if !is_powered {
            "O rádio Bluetooth está desligado. Pressione [t] para ligar."
        } else {
            "Nenhum dispositivo encontrado. Pressione [r] para escanear dispositivos próximos."
        };
        let p = Paragraph::new(Line::from(Span::styled(empty_msg, Style::default().fg(pal.dim))))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(pal.dim))
                    .title(" Dispositivos "),
            );
        f.render_widget(p, area);
        return;
    }

    let selected_idx = app.bluetooth_selected.min(snap.devices.len().saturating_sub(1));

    let rows: Vec<Row> = snap
        .devices
        .iter()
        .enumerate()
        .map(|(i, dev)| {
            let is_sel = i == selected_idx;
            format_device_row(dev, is_sel, pal)
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

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(pal.accent))
                .title(Span::styled(
                    format!(" Dispositivos ({}) ", snap.devices.len()),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )),
        );

    f.render_widget(table, area);
}

fn format_device_row<'a>(dev: &BluetoothDevice, is_sel: bool, pal: &Palette) -> Row<'a> {
    let bullet = if dev.connected {
        Span::styled("● ", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else if is_sel {
        Span::styled("▶ ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD))
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
        Span::styled(format!("{pct:>3}%"), Style::default().fg(color).add_modifier(Modifier::BOLD))
    } else {
        Span::styled("—", Style::default().fg(pal.dim))
    };

    let status_badge = if dev.blocked {
        Span::styled("Bloqueado", Style::default().fg(pal.err).add_modifier(Modifier::BOLD))
    } else if dev.connected {
        Span::styled("Conectado", Style::default().fg(pal.ok).add_modifier(Modifier::BOLD))
    } else if dev.paired {
        Span::styled("Pareado", Style::default().fg(pal.accent))
    } else {
        Span::styled("Disponível", Style::default().fg(pal.dim))
    };

    let row_style = if is_sel {
        Style::default().fg(pal.accent).add_modifier(Modifier::REVERSED)
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

fn draw_footer(snap: &BluetoothSnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let connected_devices: Vec<&BluetoothDevice> =
        snap.devices.iter().filter(|d| d.connected).collect();

    let line1 = if !connected_devices.is_empty() {
        let mut spans = vec![Span::styled(" Ativos: ", Style::default().fg(pal.dim))];
        for (i, d) in connected_devices.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" | ", Style::default().fg(pal.dim)));
            }
            spans.push(Span::styled(&d.alias, Style::default().fg(pal.ok).add_modifier(Modifier::BOLD)));
            if let Some(pct) = d.battery_percentage {
                spans.push(Span::styled(format!(" ({pct}%)"), Style::default().fg(pal.accent)));
            }
        }
        Line::from(spans)
    } else {
        Line::from(vec![
            Span::styled(" Status: ", Style::default().fg(pal.dim)),
            Span::styled("Nenhum dispositivo conectado no momento", Style::default().fg(pal.dim)),
        ])
    };

    let line2 = Line::from(vec![
        Span::styled(" [Enter] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Conectar/Desconectar   ", Style::default().fg(pal.dim)),
        Span::styled("[p] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Parear   ", Style::default().fg(pal.dim)),
        Span::styled("[r] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Escanear   ", Style::default().fg(pal.dim)),
        Span::styled("[f] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Esquecer   ", Style::default().fg(pal.dim)),
        Span::styled("[t] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Ligar/Desligar   ", Style::default().fg(pal.dim)),
        Span::styled("[b] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
        Span::styled("Bloquear", Style::default().fg(pal.dim)),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(" Telemetria & Atalhos ");

    f.render_widget(Paragraph::new(vec![line1, line2]).block(block), area);
}
