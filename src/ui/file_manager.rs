//! Aba **Navegador de Arquivos** (File Manager) do HAL-9001.
//!
//! Renderiza o conteúdo do diretório atual numa lista navegável, com um painel
//! lateral à direita para pré-visualizar arquivos de texto. A navegação é feita
//! no loop principal (`src/main.rs`): `[j/k]`/setas para mover, `[Enter]` para
//! entrar numa pasta e `[Backspace/h]` para voltar.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Paragraph, Widget, Wrap};

use crate::ui::{ACCENT, BG, CYAN, DIM, GRAY, TEXT, WARN};

/// Uma entrada do diretório sendo exibido.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Estado do navegador de arquivos mantido pelo dashboard.
#[derive(Debug, Clone, Default)]
pub struct FileManagerState {
    /// Diretório atual (caminho absoluto).
    pub current: String,
    /// Entradas do diretório atual (pastas primeiro, então ordenadas).
    pub entries: Vec<FileEntry>,
    /// Índice do item selecionado na lista.
    pub selected: usize,
    /// Conteúdo a ser pré-visualizado no painel lateral (se arquivo de texto).
    pub preview: Option<String>,
    /// Nome do arquivo sendo pré-visualizado (para o título do painel).
    pub preview_name: String,
}

impl FileManagerState {
    /// Inicializa o navegador na pasta home do usuário, se possível.
    pub fn load() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/".to_string());
        let mut state = Self::default();
        state.current = home;
        state.refresh();
        state
    }

    /// Re-lê o diretório atual, atualizando entradas e preview.
    pub fn refresh(&mut self) {
        self.entries = list_entries(&self.current);
        self.selected = self.selected.min(self.entries.len().saturating_sub(1));
        self.load_preview();
    }

    /// Move a seleção por `delta` (-1 / +1), com clamp.
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.entries.len();
        if len == 0 {
            return;
        }
        self.selected =
            (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.load_preview();
    }

    /// `[Enter]` — se o item selecionado for uma pasta, entra nela.
    /// Retorna `true` se o diretório mudou.
    pub fn open_selected(&mut self) -> bool {
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            return false;
        };
        if !entry.is_dir {
            return false;
        }
        let next = join(&self.current, &entry.name);
        if std::fs::metadata(&next).map(|m| m.is_dir()).unwrap_or(false) {
            self.current = next;
            self.selected = 0;
            self.refresh();
            return true;
        }
        false
    }

    /// `[Backspace/h]` — sobe um nível no diretório.
    /// Retorna `true` se o diretório mudou.
    pub fn go_up(&mut self) -> bool {
        if self.current == "/" {
            return false;
        }
        let parent = parent_dir(&self.current);
        if parent == self.current {
            return false;
        }
        let previous = self.current.clone();
        self.current = parent;
        self.selected = 0;
        self.refresh();
        // Seleciona a pasta de onde viemos, se ainda existir.
        if let Some(pos) = self
            .entries
            .iter()
            .position(|e| e.is_dir && e.name == basename(&previous))
        {
            self.selected = pos;
        }
        true
    }

    /// Carrega o preview de um arquivo de texto, se o item selecionado for um.
    fn load_preview(&mut self) {
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            self.preview = None;
            self.preview_name = String::new();
            return;
        };
        if entry.is_dir {
            self.preview = None;
            self.preview_name = entry.name;
            return;
        }
        let path = join(&self.current, &entry.name);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                self.preview = Some(content);
                self.preview_name = entry.name;
            }
            Err(_) => {
                self.preview = None;
                self.preview_name = entry.name;
            }
        }
    }
}

/// Widget que renderiza o navegador de arquivos.
pub struct FileManagerWidget<'a> {
    state: &'a FileManagerState,
}

impl<'a> FileManagerWidget<'a> {
    pub fn new(state: &'a FileManagerState) -> Self {
        Self { state }
    }
}

impl Widget for FileManagerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 8 || area.height < 3 {
            return;
        }

        // Painéis: lista (65%) + preview (35%).
        let chunks = Layout::horizontal([
            Constraint::Percentage(65),
            Constraint::Percentage(35),
        ])
        .split(area);

        self.render_list(chunks[0], buf);
        self.render_preview(chunks[1], buf);
    }
}

impl FileManagerWidget<'_> {
    fn render_list(&self, area: Rect, buf: &mut Buffer) {
        let title = format!(" {} ", self.state.current);
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(DIM))
            .style(Style::new().bg(BG))
            .title(Line::from(title))
            .title_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD));
        let inner = block.inner(area);
        block.render(area, buf);

        if self.state.entries.is_empty() {
            buf.set_stringn(
                inner.x,
                inner.y,
                " (diretório vazio ou inacessível)",
                inner.width as usize,
                Style::new().fg(GRAY),
            );
            return;
        }

        let len = self.state.entries.len();
        let height = inner.height as usize;
        let start = self
            .state
            .selected
            .saturating_sub(height.saturating_sub(1))
            .min(len.saturating_sub(height));

        for (offset, index) in (start..len).enumerate().take(height) {
            let entry = &self.state.entries[index];
            let display = if entry.is_dir {
                format!("📁 {}", entry.name)
            } else {
                format!("  {}", entry.name)
            };
            let y = inner.y + offset as u16;
            if index == self.state.selected {
                buf.set_stringn(
                    inner.x,
                    y,
                    &display,
                    inner.width as usize,
                    Style::new().bg(ACCENT).fg(BG).add_modifier(Modifier::BOLD),
                );
            } else {
                buf.set_stringn(
                    inner.x,
                    y,
                    &display,
                    inner.width as usize,
                    Style::new().fg(if entry.is_dir { CYAN } else { TEXT }),
                );
            }
        }
    }

    fn render_preview(&self, area: Rect, buf: &mut Buffer) {
        let title = " PREVIEW ";
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(DIM))
            .style(Style::new().bg(BG))
            .title(Line::from(title))
            .title_style(Style::new().fg(DIM));
        let inner = block.inner(area);
        block.render(area, buf);

        match &self.state.preview {
            Some(content) => {
                // Mostra o nome do arquivo + conteúdo (truncado ao tamanho).
                let preview_paragraph = Paragraph::new(content.clone())
                    .wrap(Wrap { trim: false })
                    .style(Style::new().fg(TEXT));
                let layout = Layout::vertical([Constraint::Length(1), Constraint::Min(0)])
                    .split(inner);
                buf.set_stringn(
                    layout[0].x,
                    layout[0].y,
                    &self.state.preview_name,
                    layout[0].width as usize,
                    Style::new().fg(WARN).add_modifier(Modifier::BOLD),
                );
                preview_paragraph.render(layout[1], buf);
            }
            None => {
                if self.state.entries.is_empty() {
                    return;
                }
                if let Some(entry) = self.state.entries.get(self.state.selected) {
                    let msg = if entry.is_dir {
                        format!("Pasta: {}", entry.name)
                    } else {
                        "Arquivo binário ou não-legível".to_string()
                    };
                    buf.set_stringn(
                        inner.x,
                        inner.y,
                        &msg,
                        inner.width as usize,
                        Style::new().fg(GRAY),
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers de sistema de arquivos
// ---------------------------------------------------------------------------

/// Lista as entradas de um diretório, ordenando pastas primeiro.
fn list_entries(path: &str) -> Vec<FileEntry> {
    let Ok(read_dir) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut entries: Vec<FileEntry> = read_dir
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some(FileEntry { name, is_dir })
        })
        .collect();
    // Pastas primeiro, depois arquivos; ambos em ordem alfabética.
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    entries
}

/// Concatena um caminho `base` com um nome de componente.
fn join(base: &str, name: &str) -> String {
    if base == "/" {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// Retorna o diretório pai de um caminho.
fn parent_dir(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(idx) => trimmed[..idx].to_string(),
        None => "/".to_string(),
    }
}

/// Extrai o nome final (basename) de um caminho.
fn basename(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_of_nested_is_trimmed() {
        assert_eq!(parent_dir("/home/user/docs"), "/home/user");
        assert_eq!(parent_dir("/home/"), "/");
        assert_eq!(parent_dir("/"), "/");
        assert_eq!(parent_dir(""), "/");
    }

    #[test]
    fn join_handles_root() {
        assert_eq!(join("/", "tmp"), "/tmp");
        assert_eq!(join("/home", "user"), "/home/user");
    }

    #[test]
    fn basename_extracts_last_component() {
        assert_eq!(basename("/home/user/docs"), "docs");
        assert_eq!(basename("/"), "");
    }

    #[test]
    fn load_starts_at_home() {
        let state = FileManagerState::load();
        assert_eq!(state.current, std::env::var("HOME").unwrap_or_else(|_| "/".to_string()));
    }

    #[test]
    fn move_selection_clamps() {
        let mut state = FileManagerState::default();
        // move numa lista vazia não quebra
        state.move_selection(1);
        assert_eq!(state.selected, 0);
    }
}