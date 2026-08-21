//! Seletor de arquivos estilo Yazi, usado pelo ISO Flasher e pelo gerenciador
//! de ISOs do Ventoy para escolher um `.iso`/`.img`/`.vhd` sem digitar o
//! caminho manualmente. Este módulo é intencionalmente puro (sem async/D-Bus):
//! apenas listagem de diretório (`std::fs`) e renderização.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, FilePickerState};

use super::storage::modal_block;
use super::theme::Palette;
use super::widgets::human_bytes;

/// Uma entrada da listagem de diretório do seletor (arquivo ou subdiretório).
#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<SystemTime>,
}

/// Extensões de imagem que o seletor reconhece como "escolhíveis"
/// (`.iso`/`.img`/`.vhd`, sem diferenciar maiúsculas/minúsculas).
pub fn is_pickable_image(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".iso") || lower.ends_with(".img") || lower.ends_with(".vhd")
}

/// Ordena entradas com diretórios primeiro, depois arquivos, alfabeticamente
/// (sem diferenciar caixa) dentro de cada grupo — função pura, testável sem
/// tocar o filesystem real.
pub fn sort_entries(entries: &mut [FileEntry]) {
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()),
    });
}

/// Lista o conteúdo de `path`, já ordenado (ver [`sort_entries`]). Entradas
/// ilegíveis (permissão negada, symlink quebrado, etc.) são silenciosamente
/// ignoradas em vez de abortar a listagem inteira.
pub fn list_dir(path: &Path) -> Result<Vec<FileEntry>, String> {
    let rd = std::fs::read_dir(path).map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_string();
        entries.push(FileEntry {
            name,
            path: entry.path(),
            is_dir: meta.is_dir(),
            size: meta.len(),
            modified: meta.modified().ok(),
        });
    }
    sort_entries(&mut entries);
    Ok(entries)
}

/// Formata um `SystemTime` como `YYYY-MM-DD HH:MM` (UTC), sem depender de
/// nenhuma crate de data/hora além do que já está no `Cargo.lock`.
fn format_mtime(t: SystemTime) -> String {
    let Ok(dur) = t.duration_since(SystemTime::UNIX_EPOCH) else {
        return String::new();
    };
    let secs = dur.as_secs();
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (h, m) = (rem / 3600, (rem % 3600) / 60);

    // Conversão civil de dias-desde-época para (ano, mês, dia) — algoritmo de
    // Howard Hinnant (`days_from_civil` invertido), sem dependências extras.
    let z = days as i64 + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m2 = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m2 <= 2 { y + 1 } else { y };

    format!("{y:04}-{m2:02}-{d:02} {h:02}:{m:02}")
}

/// Modal centralizado (~80% x 70%) com cabeçalho de caminho, lista rolável de
/// entradas, rodapé de detalhes do item selecionado e dicas de teclado.
pub fn draw(app: &App, pal: &Palette, f: &mut Frame, s: &FilePickerState) {
    let m = app.lang.messages();
    let area = super::centered(80, 70, f.area());
    f.render_widget(Clear, area);
    let block = modal_block(m.filepicker_title, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = ratatui::layout::Layout::vertical([
        ratatui::layout::Constraint::Length(1), // caminho atual
        ratatui::layout::Constraint::Length(1), // espaço
        ratatui::layout::Constraint::Min(3),    // lista
        ratatui::layout::Constraint::Length(1), // espaço
        ratatui::layout::Constraint::Length(1), // detalhes do item selecionado
        ratatui::layout::Constraint::Length(1), // dica de navegação
        ratatui::layout::Constraint::Length(1), // dica de saltos
    ])
    .split(inner);

    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            s.cwd.display().to_string(),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))),
        rows[0],
    );

    draw_list(app, pal, f, rows[2], s);

    let detail_line = match s.entries.get(s.selected) {
        Some(entry) if !entry.is_dir => {
            let size = human_bytes(entry.size);
            let modified = entry
                .modified
                .map(format_mtime)
                .unwrap_or_else(|| "-".to_string());
            Line::from(vec![
                Span::styled(
                    format!("{}: ", m.filepicker_label_size),
                    Style::default().fg(pal.dim),
                ),
                Span::styled(size, Style::default().fg(pal.fg)),
                Span::raw("   "),
                Span::styled(
                    format!("{}: ", m.filepicker_label_modified),
                    Style::default().fg(pal.dim),
                ),
                Span::styled(modified, Style::default().fg(pal.fg)),
            ])
        }
        Some(_) => Line::from(""),
        None => Line::from(Span::styled(
            m.filepicker_empty_dir,
            Style::default().fg(pal.dim),
        )),
    };
    f.render_widget(Paragraph::new(detail_line), rows[4]);

    if let Some(err) = &s.error {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                err.as_str(),
                Style::default().fg(pal.err).add_modifier(Modifier::BOLD),
            ))),
            rows[5],
        );
    } else {
        let hint = format!(
            "{}  {}  {}",
            m.filepicker_hint_nav, m.filepicker_hint_pick, m.filepicker_hint_updir
        );
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(hint, Style::default().fg(pal.dim))))
                .wrap(Wrap { trim: false }),
            rows[5],
        );
    }

    let jumps = format!(
        "{}: {}={}  {}={}  {}={}  {}={}",
        m.filepicker_hint_jumps,
        "~",
        m.filepicker_jump_home,
        "d",
        m.filepicker_jump_downloads,
        "M",
        m.filepicker_jump_media,
        "/",
        m.filepicker_jump_root,
    );
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(jumps, Style::default().fg(pal.dim))))
            .wrap(Wrap { trim: false }),
        rows[6],
    );
}

fn draw_list(app: &App, pal: &Palette, f: &mut Frame, area: Rect, s: &FilePickerState) {
    let m = app.lang.messages();
    if s.entries.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                m.filepicker_empty_dir,
                Style::default().fg(pal.dim),
            ))),
            area,
        );
        return;
    }

    let visible = area.height as usize;
    let start = s.selected.saturating_sub(visible.saturating_sub(1));
    let mut lines: Vec<Line> = Vec::with_capacity(visible);
    for (idx, entry) in s.entries.iter().enumerate().skip(start).take(visible) {
        let selected = idx == s.selected;
        let base_style = if selected {
            Style::default()
                .fg(pal.bg)
                .bg(pal.accent)
                .add_modifier(Modifier::BOLD)
        } else if entry.is_dir {
            Style::default().fg(pal.ok)
        } else if is_pickable_image(&entry.name) {
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(pal.fg)
        };
        let prefix = if entry.is_dir { "/" } else { " " };
        let size = if entry.is_dir {
            String::new()
        } else {
            human_bytes(entry.size)
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{prefix} {} ", entry.name), base_style),
            Span::styled(size, base_style.fg(pal.dim)),
        ]));
    }
    f.render_widget(Paragraph::new(lines), area);
}
