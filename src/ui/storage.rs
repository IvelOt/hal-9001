//! Aba 4 — Discos & Armazenamento (UDisks2). Render do Módulo 4.
//!
//! Redesenho: lista simples com um item por drive físico/removível (sem a
//! árvore drive→partição de antes) + painel de detalhes com status de
//! montagem, medidor de capacidade, sistema de arquivos e status do
//! multi-boot leve embarcado.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{
    App, FlasherModalState, FlasherStage, FormatField, FormatModalState, FsChoice,
    MultibootIsoManagerStage, MultibootIsoManagerState, StorageModal, SudoPromptState,
};
use crate::backend::multiboot;
use crate::backend::storage::{primary_partition, BusType, DriveInfo, PartitionInfo};

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
                app.lang.messages().storage_hint_multiboot_prepare,
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

    draw_list(app, pal, f, cols[0]);
    draw_details(app, pal, f, cols[1]);
}

fn icon(app: &App, nerd: &str, ascii: &str) -> String {
    if app.config.ui.icons {
        format!("{nerd} ")
    } else {
        format!("{ascii} ")
    }
}

/// Ícone do drive — Nerd Font `\u{f287}` (USB) / `\u{f0a0}` (Disco/SSD/HDD)
/// quando `icons = true`, fallback ASCII limpo caso contrário (Zero Emojis
/// Policy: nenhum emoji é usado em toda a base de código).
fn drive_icon(app: &App, drive: &DriveInfo) -> String {
    if drive.bus == BusType::Usb || drive.removable {
        icon(app, "\u{f287}", "[USB]")
    } else if drive.rotational {
        icon(app, "\u{f0a0}", "[HDD]")
    } else {
        icon(app, "\u{f0a0}", "[SSD]")
    }
}

/// Tag "disco de sistema" — Nerd Font de cadeado `\u{f023}` + palavra
/// traduzida quando `icons = true`, ou o token ASCII `[SISTEMA]`/`[SYSTEM]`
/// quando `icons = false`.
fn system_tag(app: &App) -> String {
    let m = app.lang.messages();
    if app.config.ui.icons {
        format!("\u{f023} {}", m.storage_tag_system)
    } else {
        m.storage_tag_system_ascii.to_string()
    }
}

/// Tag "pendrive Ventoy" (detecção somente-leitura por rótulo — ver
/// `backend::storage::detect_ventoy`) — Nerd Font de disco de boot
/// `\u{f17c}` + palavra traduzida quando `icons = true`, ou o token ASCII
/// `[VENTOY]` caso contrário (Zero Emojis Policy).
fn ventoy_tag(app: &App) -> String {
    let m = app.lang.messages();
    if app.config.ui.icons {
        format!("\u{f17c} {}", m.storage_tag_ventoy)
    } else {
        m.storage_tag_ventoy_ascii.to_string()
    }
}

/// Ícone de multi-boot ativo — Nerd Font de disco de boot `\u{f17c}` quando
/// `icons = true`, ou `[MB]` caso contrário.
fn multiboot_tag(app: &App, n_isos: usize) -> String {
    let m = app.lang.messages();
    let base = if app.config.ui.icons {
        format!("\u{f17c} {}", m.storage_multiboot_active)
    } else {
        format!("[MB] {}", m.storage_multiboot_active)
    };
    format!("{base} ({n_isos})")
}

fn draw_list(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let block = super::content_block(m.storage_col_tree, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let Some(snapshot) = &app.storage else {
        return;
    };
    if snapshot.drives.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                m.storage_empty,
                Style::default().fg(pal.dim),
            ))),
            inner,
        );
        return;
    }

    let selected_idx = app.storage_drive_index();
    let mut lines: Vec<Line> = Vec::with_capacity(snapshot.drives.len());
    for (idx, drive) in snapshot.drives.iter().enumerate() {
        lines.push(drive_line(app, pal, drive, Some(idx) == selected_idx));
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

/// Uma linha da lista: ` <rótulo amigável> (<tamanho>)  [tags]`. O rótulo
/// prefere o `IdLabel` da partição primária (ex.: `MEUPENDRIVE`) ao nome
/// genérico `<vendor> <model>` — ver [`DriveInfo::friendly_label`].
fn drive_line<'a>(app: &App, pal: &Palette, drive: &DriveInfo, selected: bool) -> Line<'a> {
    let m = app.lang.messages();
    let style = row_style(pal, selected);
    let mut spans = vec![
        Span::styled(drive_icon(app, drive), style),
        Span::styled(drive.friendly_label(), style),
        Span::styled(format!(" ({})", human_bytes(drive.size)), style.fg(pal.dim)),
    ];
    if drive.is_system {
        spans.push(Span::styled(
            format!("  {}", system_tag(app)),
            Style::default().fg(pal.err),
        ));
    } else if drive.bus == BusType::Usb || drive.removable {
        spans.push(Span::styled(
            format!("  [{}]", m.storage_tag_usb),
            Style::default().fg(pal.accent),
        ));
    }
    if let Some(p) = primary_partition(drive) {
        if let Some(mp) = p.mount_points.first() {
            if multiboot::is_multiboot_installed(mp) {
                let n = multiboot::count_isos(mp);
                spans.push(Span::styled(
                    format!("  {}", multiboot_tag(app, n)),
                    Style::default().fg(pal.ok),
                ));
            }
        }
    }
    if drive.is_ventoy {
        spans.push(Span::styled(
            format!("  {}", ventoy_tag(app)),
            Style::default().fg(pal.ok),
        ));
    }
    Line::from(spans)
}

/// Medidor de capacidade "<livre> livres de <total> (<pct>%)" com barra —
/// baseado no espaço usado/total da partição primária (quando montada e com
/// uso conhecido via `sysinfo`).
fn capacity_bar<'a>(pal: &Palette, m: &crate::i18n::Messages, p: &PartitionInfo) -> Vec<Line<'a>> {
    let mut out = Vec::new();
    let Some(used) = p.used else {
        return out;
    };
    let free = p.size.saturating_sub(used);
    let ratio = p.usage_ratio().unwrap_or(0.0);
    let bar_w = 24usize;
    let filled = (ratio * bar_w as f64).round() as usize;
    let empty = bar_w.saturating_sub(filled);
    out.push(Line::from(vec![
        Span::styled("█".repeat(filled), Style::default().fg(pal.gauge_color(ratio))),
        Span::styled("░".repeat(empty), Style::default().fg(pal.dim)),
        Span::styled(format!(" {:>3.0}%", ratio * 100.0), Style::default().fg(pal.dim)),
    ]));
    out.push(Line::from(Span::styled(
        format!(
            "{} {} {} ({:.0}%)",
            human_bytes(free),
            m.storage_free_of,
            human_bytes(p.size),
            ratio * 100.0
        ),
        Style::default().fg(pal.dim),
    )));
    out
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
            lines.push(Line::from(Span::styled(
                drive.friendly_label(),
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
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
                    system_tag(app),
                    Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
                )));
            }

            lines.push(Line::from(""));
            match partition {
                None => {
                    lines.push(Line::from(Span::styled(
                        m.storage_label_not_mounted,
                        Style::default().fg(pal.dim),
                    )));
                }
                Some(p) => {
                    lines.push(kv(m.storage_label_fs, p.fs.label(), pal));
                    if p.is_mounted() {
                        lines.push(kv(
                            m.storage_label_mounted_at,
                            p.mount_points.join(", "),
                            pal,
                        ));
                        for line in capacity_bar(pal, m, p) {
                            lines.push(line);
                        }
                    } else {
                        lines.push(Line::from(Span::styled(
                            m.storage_label_not_mounted,
                            Style::default().fg(pal.dim),
                        )));
                    }

                    lines.push(Line::from(""));
                    let mb_status = match p.mount_points.first() {
                        Some(mp) if multiboot::is_multiboot_installed(mp) => {
                            format!(
                                "{} ({} ISOs)",
                                m.storage_multiboot_active,
                                multiboot::count_isos(mp)
                            )
                        }
                        Some(_) => m.storage_multiboot_not_installed.to_string(),
                        None => m.storage_multiboot_unknown.to_string(),
                    };
                    lines.push(kv(m.storage_label_multiboot, mb_status, pal));
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
            format!("{}  ", m.storage_hint_format),
            Style::default().fg(pal.dim),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled(
            format!("{}  ", m.storage_hint_multiboot_prepare),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("{}  ", m.storage_hint_iso_manager),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("{}  ", m.storage_hint_eject),
            Style::default().fg(pal.dim),
        ),
        Span::styled(
            format!("{}  ", m.storage_hint_refresh),
            Style::default().fg(pal.dim),
        ),
    ]));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub(crate) fn kv<'a>(label: &'a str, value: impl Into<String>, pal: &Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(
            format!("{label}: "),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(value.into(), Style::default().fg(pal.fg)),
    ])
}

// ---------------------------------------------------------------------------
// Modais interativos: Formatação (Épico G), ISO Flasher (Épico H) e
// gerenciador de ISOs multi-boot.
// ---------------------------------------------------------------------------

/// Ponto de entrada dos modais de storage — despachado por `ui::draw` quando
/// `App::storage_modal_open()` é `true` e a aba ativa é Storage.
pub fn draw_modal(app: &App, pal: &Palette, f: &mut Frame) {
    match &app.storage_modal {
        StorageModal::Format(s) => draw_format_modal(app, pal, f, s),
        StorageModal::Flasher(s) => draw_flasher_modal(app, pal, f, s),
        StorageModal::FilePicker(s) => super::file_picker::draw(app, pal, f, s),
        StorageModal::MultibootIsoManager(s) => draw_multiboot_iso_manager_modal(app, pal, f, s),
        StorageModal::None => {}
    }
}

/// Modal nativo (senha mascarada com `•`) de autenticação sudo — desenhado
/// por cima de qualquer outra tela/modal (ver `ui::draw`). Substitui o
/// antigo prompt real de `pkexec`/`sudo` herdado do terminal.
pub fn draw_sudo_prompt(_app: &App, pal: &Palette, f: &mut Frame, s: &SudoPromptState) {
    let area = super::centered(56, 30, f.area());
    f.render_widget(Clear, area);
    let block = modal_block("Autenticacao sudo", pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(kv("Operacao", &s.label, pal));
    lines.push(Line::from(""));

    let masked: String = "*".repeat(s.password.chars().count());
    lines.push(Line::from(vec![
        Span::styled(
            "Senha sudo: ",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{masked}▏"),
            Style::default().fg(pal.bg).bg(pal.accent),
        ),
    ]));
    lines.push(Line::from(""));

    if let Some(err) = &s.error {
        lines.push(Line::from(Span::styled(
            err.as_str(),
            Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "[Enter] Confirmar   [Esc] Cancelar",
        Style::default().fg(pal.dim),
    )));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub(crate) fn modal_block<'a>(title: &'a str, pal: &Palette) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))
}

fn draw_format_modal(app: &App, pal: &Palette, f: &mut Frame, s: &FormatModalState) {
    let m = app.lang.messages();
    let area = super::centered(56, 40, f.area());
    f.render_widget(Clear, area);
    let block = modal_block(m.storage_format_title, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(kv(m.storage_format_target, &s.target_label, pal));
    lines.push(Line::from(""));

    let fs_style = |focused: bool| {
        if focused {
            Style::default()
                .fg(pal.bg)
                .bg(pal.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.fg)
        }
    };
    let mut fs_spans = vec![Span::styled(
        format!("{}: ", m.storage_format_fs_label),
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
    )];
    for (idx, choice) in FsChoice::ALL.iter().enumerate() {
        let selected = idx == s.fs_idx;
        let focused = selected && s.field == FormatField::Fs;
        let text = if selected {
            format!("[{}] ", choice.label())
        } else {
            format!(" {} ", choice.label())
        };
        fs_spans.push(Span::styled(text, fs_style(focused)));
    }
    lines.push(Line::from(fs_spans));
    lines.push(Line::from(""));

    let label_focused = s.field == FormatField::Label;
    let label_display = if label_focused {
        format!("{}▏", s.label)
    } else {
        s.label.clone()
    };
    lines.push(Line::from(vec![
        Span::styled(
            format!("{}: ", m.storage_format_label_label),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            label_display,
            if label_focused {
                Style::default().fg(pal.bg).bg(pal.accent)
            } else {
                Style::default().fg(pal.fg)
            },
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        m.storage_format_warning,
        Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let confirm_focused = s.field == FormatField::Confirm;
    lines.push(Line::from(Span::styled(
        m.storage_format_confirm,
        if confirm_focused {
            Style::default()
                .fg(pal.bg)
                .bg(pal.err)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.dim)
        },
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        m.storage_format_hint,
        Style::default().fg(pal.dim),
    )));

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_flasher_modal(app: &App, pal: &Palette, f: &mut Frame, s: &FlasherModalState) {
    let m = app.lang.messages();
    let area = super::centered(70, 60, f.area());
    f.render_widget(Clear, area);
    let block = modal_block(m.storage_flash_title, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = vec![
        kv(m.storage_flash_target_label, &s.target_label, pal),
        kv(m.storage_label_node, &s.target_dev_node, pal),
        kv(m.storage_label_size, human_bytes(s.target_size), pal),
        Line::from(""),
    ];

    match &s.stage {
        FlasherStage::SelectIso { input, error } => {
            lines.push(Line::from(Span::styled(
                m.storage_flash_path_prompt,
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                format!("{input}▏"),
                Style::default().fg(pal.bg).bg(pal.accent),
            )));
            if let Some(err) = error {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    err.as_str(),
                    Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
                )));
            }
        }
        FlasherStage::Checksumming { pct } => {
            lines.push(Line::from(Span::styled(
                m.storage_flash_checksumming,
                Style::default().fg(pal.accent),
            )));
            lines.push(progress_line(*pct, pal));
        }
        FlasherStage::Ready { sha256 } => {
            lines.push(kv(
                m.storage_flash_iso_label,
                s.iso_path.display().to_string(),
                pal,
            ));
            lines.push(kv(m.storage_flash_size_label, human_bytes(s.iso_size), pal));
            if let Some(sha) = sha256 {
                lines.push(kv(m.storage_flash_sha_label, sha, pal));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                m.storage_flash_checksum_hint,
                Style::default().fg(pal.dim),
            )));
        }
        FlasherStage::Confirm1 => {
            lines.push(kv(
                m.storage_flash_iso_label,
                s.iso_path.display().to_string(),
                pal,
            ));
            lines.push(kv(m.storage_flash_size_label, human_bytes(s.iso_size), pal));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                m.storage_flash_confirm1_title,
                Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(m.storage_flash_hint_continue, Style::default().fg(pal.dim)),
                Span::raw("  "),
                Span::styled(m.storage_flash_hint_cancel, Style::default().fg(pal.dim)),
            ]));
        }
        FlasherStage::Confirm2 { typed } => {
            lines.push(Line::from(Span::styled(
                m.storage_flash_confirm2_title,
                Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("{}: {}", m.storage_flash_confirm2_prompt, s.target_dev_node),
                Style::default().fg(pal.accent),
            )));
            lines.push(Line::from(Span::styled(
                format!("{typed}▏"),
                Style::default().fg(pal.bg).bg(pal.accent),
            )));
        }
        FlasherStage::Flashing {
            bytes_written,
            total_bytes,
            speed_mbps,
            eta_secs,
        } => {
            let pct = if *total_bytes > 0 {
                (*bytes_written as f32 / *total_bytes as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            lines.push(Line::from(Span::styled(
                m.storage_flash_flashing,
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )));
            lines.push(progress_line(pct, pal));
            lines.push(Line::from(format!(
                "{} / {}   {:.1} MB/s   ETA {}s",
                human_bytes(*bytes_written),
                human_bytes(*total_bytes),
                speed_mbps,
                eta_secs
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                m.storage_flash_hint_cancel,
                Style::default().fg(pal.dim),
            )));
        }
        FlasherStage::Done { ok, message } => {
            let (color, text) = if *ok {
                (pal.ok, m.storage_flash_success)
            } else {
                (pal.err, m.storage_flash_failed)
            };
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(Span::styled(
                message.as_str(),
                Style::default().fg(pal.dim),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                m.storage_flash_hint_continue,
                Style::default().fg(pal.dim),
            )));
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub(crate) fn progress_line<'a>(pct: f32, pal: &Palette) -> Line<'a> {
    let bar_w = 30usize;
    let filled = (pct.clamp(0.0, 1.0) * bar_w as f32).round() as usize;
    let empty = bar_w.saturating_sub(filled);
    Line::from(vec![
        Span::styled(
            "█".repeat(filled),
            Style::default().fg(pal.gauge_color(pct as f64)),
        ),
        Span::styled("░".repeat(empty), Style::default().fg(pal.dim)),
        Span::styled(
            format!(" {:>3.0}%", pct * 100.0),
            Style::default().fg(pal.dim),
        ),
    ])
}

// ---------------------------------------------------------------------------
// Gerenciador de ISOs multi-boot (`<mount>/ISOs/`).
// ---------------------------------------------------------------------------

fn draw_multiboot_iso_manager_modal(
    app: &App,
    pal: &Palette,
    f: &mut Frame,
    s: &MultibootIsoManagerState,
) {
    let m = app.lang.messages();
    let area = super::centered(70, 60, f.area());
    f.render_widget(Clear, area);
    let block = modal_block(m.multiboot_iso_mgr_title, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = vec![
        kv(m.storage_multiboot_target_label, &s.target_label, pal),
        Line::from(""),
    ];

    match &s.stage {
        MultibootIsoManagerStage::Loading => {
            lines.push(Line::from(Span::styled(
                m.storage_flash_checksumming, // reaproveita "Calculando..." como placeholder de "Carregando..."
                Style::default().fg(pal.dim),
            )));
        }
        MultibootIsoManagerStage::Listing {
            entries,
            selected,
            free_bytes,
        } => {
            if let Some(free) = free_bytes {
                lines.push(kv(m.multiboot_iso_mgr_free_space, human_bytes(*free), pal));
                lines.push(Line::from(""));
            }
            if entries.is_empty() {
                lines.push(Line::from(Span::styled(
                    m.multiboot_iso_mgr_empty,
                    Style::default().fg(pal.dim),
                )));
            } else {
                for (idx, entry) in entries.iter().enumerate() {
                    let style = row_style(pal, idx == *selected);
                    lines.push(Line::from(vec![
                        Span::styled(format!("{} ", entry.name), style),
                        Span::styled(human_bytes(entry.size), style.fg(pal.dim)),
                    ]));
                }
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}  ", m.multiboot_iso_mgr_hint_add),
                    Style::default().fg(pal.dim),
                ),
                Span::styled(m.multiboot_iso_mgr_hint_remove, Style::default().fg(pal.dim)),
            ]));
        }
        MultibootIsoManagerStage::ConfirmRemove { file_name } => {
            lines.push(Line::from(Span::styled(
                format!("{}: {file_name}", m.multiboot_iso_mgr_confirm_remove),
                Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(m.storage_flash_hint_continue, Style::default().fg(pal.dim)),
                Span::raw("  "),
                Span::styled(m.storage_flash_hint_cancel, Style::default().fg(pal.dim)),
            ]));
        }
        MultibootIsoManagerStage::Copying {
            bytes_written,
            total_bytes,
            file_name,
        } => {
            let pct = if *total_bytes > 0 {
                (*bytes_written as f32 / *total_bytes as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            lines.push(kv(m.storage_flash_iso_label, file_name, pal));
            lines.push(Line::from(Span::styled(
                m.multiboot_iso_mgr_copying,
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )));
            lines.push(progress_line(pct, pal));
            lines.push(Line::from(format!(
                "{} / {}",
                human_bytes(*bytes_written),
                human_bytes(*total_bytes)
            )));
        }
        MultibootIsoManagerStage::Removing { file_name } => {
            lines.push(kv(m.storage_flash_iso_label, file_name, pal));
            lines.push(Line::from(Span::styled(
                m.multiboot_iso_mgr_removing,
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )));
        }
        MultibootIsoManagerStage::Error { message } => {
            lines.push(Line::from(Span::styled(
                message.as_str(),
                Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                m.storage_flash_hint_continue,
                Style::default().fg(pal.dim),
            )));
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
