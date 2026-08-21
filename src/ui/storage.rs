//! Aba 4 — Discos & Armazenamento (UDisks2). Render do Módulo 4.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::backend::storage::{BusType, DriveInfo, PartitionInfo, StorageRow};

use super::theme::Palette;
use super::widgets::human_bytes;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let Some(snapshot) = &app.storage else {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            app.lang.messages().tab_storage,
            "storage",
            &[
                app.lang.messages().storage_hint_mount,
                app.lang.messages().storage_hint_eject,
                app.lang.messages().storage_hint_format,
                app.lang.messages().storage_hint_iso,
            ],
        );
        return;
    };

    if !snapshot.udisks_available {
        super::draw_pending(
            app,
            pal,
            f,
            area,
            app.lang.messages().tab_storage,
            "storage",
            &[app.lang.messages().storage_empty],
        );
        return;
    }

    let cols =
        Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).split(area);

    draw_tree(app, pal, f, cols[0]);
    draw_details(app, pal, f, cols[1]);
}

fn icon(app: &App, nerd: &str, ascii: &str) -> String {
    if app.config.ui.icons {
        format!("{nerd} ")
    } else {
        format!("{ascii} ")
    }
}

fn drive_icon(app: &App, drive: &DriveInfo) -> String {
    if drive.bus == BusType::Usb || drive.removable {
        icon(app, "󰇄", "[USB]")
    } else if drive.rotational {
        icon(app, "", "[HDD]")
    } else {
        icon(app, "󰋊", "[SSD]")
    }
}

fn partition_icon(app: &App) -> String {
    icon(app, "", "-")
}

fn draw_tree(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let block = super::content_block(m.storage_col_tree, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(snapshot) = &app.storage else {
        return;
    };
    let rows = snapshot.rows();
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                m.storage_empty,
                Style::default().fg(pal.dim),
            ))),
            inner,
        );
        return;
    }

    let selected = app.storage_row();
    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for row in &rows {
        let is_selected = Some(*row) == selected;
        let line = match *row {
            StorageRow::Drive(di) => snapshot
                .drive(di)
                .map(|d| drive_line(app, pal, d, is_selected))
                .unwrap_or_default(),
            StorageRow::Partition(di, pi) => snapshot
                .drive(di)
                .zip(snapshot.partition(di, pi))
                .map(|(d, p)| partition_line(app, pal, d, p, is_selected))
                .unwrap_or_default(),
        };
        lines.push(line);
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn row_style(pal: &Palette, selected: bool) -> Style {
    if selected {
        Style::default()
            .fg(pal.bg)
            .bg(pal.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(pal.fg)
    }
}

fn drive_line<'a>(app: &App, pal: &Palette, drive: &DriveInfo, selected: bool) -> Line<'a> {
    let m = app.lang.messages();
    let style = row_style(pal, selected);
    let name = if drive.model.is_empty() {
        drive.dev_node.clone()
    } else {
        format!("{} {}", drive.vendor, drive.model)
            .trim()
            .to_string()
    };
    let mut spans = vec![
        Span::styled(drive_icon(app, drive), style),
        Span::styled(name, style),
        Span::styled(format!(" {}", human_bytes(drive.size)), style.fg(pal.dim)),
    ];
    if drive.is_system {
        spans.push(Span::styled(
            format!("  {}", m.storage_tag_system),
            Style::default().fg(pal.err),
        ));
    } else if drive.bus == BusType::Usb || drive.removable {
        spans.push(Span::styled(
            format!("  [{}]", m.storage_tag_usb),
            Style::default().fg(pal.accent),
        ));
    }
    Line::from(spans)
}

fn partition_line<'a>(
    app: &App,
    pal: &Palette,
    _drive: &DriveInfo,
    partition: &PartitionInfo,
    selected: bool,
) -> Line<'a> {
    let m = app.lang.messages();
    let style = row_style(pal, selected);
    let label = if partition.label.is_empty() {
        partition.dev_node.clone()
    } else {
        partition.label.clone()
    };
    let mut spans = vec![
        Span::styled("  ", style),
        Span::styled(partition_icon(app), style.fg(pal.dim)),
        Span::styled(format!("{label} "), style),
        Span::styled(format!("{} ", partition.fs.label()), style.fg(pal.dim)),
    ];
    if let Some(ratio) = partition.usage_ratio() {
        let bar_w = 8usize;
        let filled = (ratio * bar_w as f64).round() as usize;
        let empty = bar_w.saturating_sub(filled);
        spans.push(Span::styled(
            "█".repeat(filled),
            Style::default().fg(pal.gauge_color(ratio)),
        ));
        spans.push(Span::styled(
            "░".repeat(empty),
            Style::default().fg(pal.dim),
        ));
        spans.push(Span::styled(
            format!(" {:>3.0}%", ratio * 100.0),
            style.fg(pal.dim),
        ));
    }
    if partition.is_system {
        spans.push(Span::styled(
            format!(" {}", m.storage_tag_system),
            Style::default().fg(pal.err),
        ));
    }
    Line::from(spans)
}

fn draw_details(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let block = super::content_block(m.storage_col_details, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    match app.storage_selection() {
        None => lines.push(Line::from(Span::styled(
            m.storage_no_selection,
            Style::default().fg(pal.dim),
        ))),
        Some((drive, partition)) => {
            lines.push(kv(
                m.storage_label_model,
                format!("{} {}", drive.vendor, drive.model).trim(),
                pal,
            ));
            lines.push(kv(m.storage_label_node, &drive.dev_node, pal));
            lines.push(kv(m.storage_label_bus, drive.bus.label(), pal));
            lines.push(kv(
                m.storage_label_removable,
                if drive.removable {
                    m.storage_yes
                } else {
                    m.storage_no
                },
                pal,
            ));
            lines.push(kv(m.storage_label_size, human_bytes(drive.size), pal));
            if drive.is_system {
                lines.push(Line::from(Span::styled(
                    m.storage_tag_system,
                    Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
                )));
            }

            if let Some(p) = partition {
                lines.push(Line::from(""));
                let label = if p.label.is_empty() {
                    p.dev_node.clone()
                } else {
                    p.label.clone()
                };
                lines.push(Line::from(Span::styled(
                    label,
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )));
                lines.push(kv(m.storage_label_fs, p.fs.label(), pal));
                lines.push(kv(m.storage_label_size, human_bytes(p.size), pal));
                if p.is_mounted() {
                    lines.push(kv(
                        m.storage_label_mounted_at,
                        p.mount_points.join(", "),
                        pal,
                    ));
                    if let Some(used) = p.used {
                        lines.push(kv(
                            m.storage_label_usage,
                            format!("{} / {}", human_bytes(used), human_bytes(p.size)),
                            pal,
                        ));
                    }
                } else {
                    lines.push(Line::from(Span::styled(
                        m.storage_label_not_mounted,
                        Style::default().fg(pal.dim),
                    )));
                }
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{}  ", m.storage_hint_nav),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("{}  ", m.storage_hint_mount),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("{}  ", m.storage_hint_eject),
            Style::default().fg(pal.dim),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{}  ", m.storage_hint_format),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("{}  ", m.storage_hint_iso),
            Style::default().fg(pal.dim),
        ),
        Span::styled(m.storage_hint_refresh, Style::default().fg(pal.dim)),
    ]));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn kv<'a>(label: &'a str, value: impl Into<String>, pal: &Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{label}: "),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(value.into(), Style::default().fg(pal.fg)),
    ])
}
