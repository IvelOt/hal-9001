//! **PTY Dock** embarcado para o Yazi File Manager e Terminal Shell (Aba Arquivos).
//!
//! Roda dentro de um pseudo-terminal filho (`portable-pty`) interpretado
//! pelo parser ANSI `vt100`, embarcado no próprio container do Ratatui — no
//! estilo `:terminal` do Neovim.
//!
//! Possui dois modos de foco:
//! * **Terminal Focado (`is_focused = true`)**: 100% das teclas são enviadas ao PTY.
//!   Para sair do foco e navegar nas abas do HAL-9001, pressione `Esc Esc`, `F12` ou `Ctrl+Q`.
//! * **Modo Navegação (`is_focused = false`)**: Teclas normais (`1-5`, `Tab`, `q`)
//!   navegam na central. Pressione `i`, `a` ou `Enter` para focar novamente no terminal.

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget};

use crate::ai_agent::pty_session::{render_screen, AgentCommand, PtySession, PtyTarget};
use crate::ui::{ACCENT, BG, CYAN, DANGER, DIM, GRAY, TEXT, WARN};

/// Intervalo mínimo entre tentativas de reinício após uma falha de spawn.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Modo de operação do dock de terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockMode {
    /// Yazi — Gerenciador de arquivos gráfico no terminal.
    Yazi,
    /// Shell interativo do usuário (`$SHELL` / `zsh` / `bash`).
    Shell,
}

impl DockMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Yazi => "YAZI (GERENCIADOR DE ARQUIVOS)",
            Self::Shell => "TERMINAL ($SHELL)",
        }
    }
}

/// Dock que hospeda o Yazi ou Terminal Shell num PTY embarcado (`portable-pty` + `vt100`).
pub struct YaziDock {
    /// Sessão PTY ativa.
    session: Option<Arc<PtySession>>,
    /// Modo atual (Yazi ou Shell).
    mode: DockMode,
    /// Se o terminal está capturando ativamente todas as teclas do teclado.
    is_focused: bool,
    /// Timestamp do último Esc pressionado para detectar Esc duplo.
    last_esc: Cell<Option<Instant>>,
    /// Última dimensão (rows, cols) aplicada ao PTY.
    last_size: Cell<(u16, u16)>,
    /// Momento da última tentativa de início (backoff de reinício).
    last_attempt: Cell<Option<Instant>>,
    /// Mensagem do último erro de inicialização.
    error: Option<String>,
}

impl YaziDock {
    /// Cria um dock vazio com foco ativo por padrão.
    pub fn new() -> Self {
        Self {
            session: None,
            mode: DockMode::Yazi,
            is_focused: true,
            last_esc: Cell::new(None),
            last_size: Cell::new((0, 0)),
            last_attempt: Cell::new(None),
            error: None,
        }
    }

    /// Retorna se o terminal está em modo focado (capturando todas as teclas).
    pub fn is_focused(&self) -> bool {
        self.is_focused
    }

    /// Altera o estado de foco do terminal.
    pub fn set_focused(&mut self, focused: bool) {
        self.is_focused = focused;
    }

    /// Alterna o modo entre Yazi e Shell interativo.
    pub fn set_mode(&mut self, mode: DockMode) {
        if self.mode != mode {
            self.mode = mode;
            self.session = None; // Reinicia o processo com o novo modo
            self.ensure_running();
        }
    }

    /// Retorna o modo atual do dock.
    pub fn mode(&self) -> DockMode {
        self.mode
    }

    /// Registra um pressionamento de Esc e retorna `true` se foi um Esc duplo (dentro de 450ms).
    pub fn check_double_esc(&self) -> bool {
        let now = Instant::now();
        if let Some(last) = self.last_esc.get() {
            if now.duration_since(last) < Duration::from_millis(450) {
                self.last_esc.set(None);
                return true;
            }
        }
        self.last_esc.set(Some(now));
        false
    }

    /// Garante que o processo do PTY esteja rodando.
    pub fn ensure_running(&mut self) {
        let running = self
            .session
            .as_ref()
            .map(|session| !session.has_exited())
            .unwrap_or(false);
        if running {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_attempt.get() {
            if now.duration_since(last) < RETRY_INTERVAL {
                return;
            }
        }
        self.last_attempt.set(Some(now));
        self.start();
    }

    /// Dispara o processo dentro de um novo PTY.
    fn start(&mut self) {
        let cmd = match self.mode {
            DockMode::Yazi => AgentCommand::yazi(),
            DockMode::Shell => AgentCommand::shell(),
        };
        let mut session = PtySession::new(cmd);
        match session.start() {
            Ok(()) => {
                let (rows, cols) = self.last_size.get();
                if rows > 0 && cols > 0 {
                    let _ = session.resize(rows, cols);
                }
                self.session = Some(Arc::new(session));
                self.error = None;
            }
            Err(e) => {
                self.session = None;
                self.error = Some(e.to_string());
            }
        }
    }

    /// Redimensiona o PTY para acompanhar exatamente a área disponível na TUI.
    pub fn resize(&self, rows: u16, cols: u16) {
        if self.last_size.get() == (rows, cols) {
            return;
        }
        self.last_size.set((rows, cols));
        if let Some(session) = &self.session {
            let _ = session.resize(rows, cols);
        }
    }

    /// Encaminha bytes crus (teclas) para o processo.
    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        match &self.session {
            Some(session) => session.write_input(bytes),
            None => Ok(()),
        }
    }

    /// Aplica um fechamento sobre a tela virtual atual.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> Option<R> {
        self.session
            .as_ref()
            .map(|session| session.with_screen(f))
    }

    /// `true` quando há um processo vivo.
    pub fn is_running(&self) -> bool {
        self.session
            .as_ref()
            .map(|session| !session.has_exited())
            .unwrap_or(false)
    }

    /// Mensagem do último erro de inicialização, se houver.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Default for YaziDock {
    fn default() -> Self {
        Self::new()
    }
}

/// Widget que desenha o PTY Dock no Ratatui com moldura dinâmica de foco.
pub struct YaziDockWidget<'a> {
    dock: &'a YaziDock,
}

impl<'a> YaziDockWidget<'a> {
    pub fn new(dock: &'a YaziDock) -> Self {
        Self { dock }
    }
}

impl Widget for YaziDockWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let is_focused = self.dock.is_focused();
        let mode_label = self.dock.mode().label();

        let (border_color, title_spans) = if is_focused {
            (
                ACCENT,
                vec![
                    Span::styled(format!("  {mode_label} "), Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
                    Span::styled("· [FOCADO] ", Style::new().fg(CYAN).add_modifier(Modifier::BOLD)),
                    Span::styled("(Pressione Esc Esc ou F12 para desviar foco) ", Style::new().fg(GRAY)),
                ],
            )
        } else {
            (
                DIM,
                vec![
                    Span::styled(format!("  {mode_label} "), Style::new().fg(GRAY).add_modifier(Modifier::BOLD)),
                    Span::styled("· [MODO NAVEGAÇÃO] ", Style::new().fg(WARN)),
                    Span::styled("(Pressione 'i' ou Enter para focar) ", Style::new().fg(ACCENT)),
                ],
            )
        };

        let block = Block::bordered()
            .border_type(if is_focused { BorderType::Thick } else { BorderType::Rounded })
            .border_style(Style::new().fg(border_color))
            .style(Style::new().bg(BG))
            .title(Line::from(title_spans));

        let inner = block.inner(area);
        block.render(area, buf);

        // Limpa explicitamente a área interna antes de renderizar células do PTY
        Clear.render(inner, buf);

        // Atualiza a dimensão do PTY para as linhas/colunas exatas da área interna
        self.dock.resize(inner.height, inner.width);

        if !self.dock.is_running() {
            let message = match self.dock.error() {
                Some(e) => format!(" Falha ao iniciar terminal: {e}"),
                None => " O processo terminou. Pressione Enter para reiniciar.".to_string(),
            };
            render_placeholder(
                &[
                    Line::from(Span::styled(message, Style::new().fg(DANGER))),
                    Line::from(Span::styled(
                        " [1-5] trocar de aba · [i/Enter] focar · [Ctrl+Q] sair",
                        Style::new().fg(GRAY),
                    )),
                ],
                inner,
                buf,
            );
            return;
        }

        self.dock.with_screen(|screen| render_screen(screen, inner, buf));
    }
}

/// Escreve linhas de texto simples dentro da área do dock.
fn render_placeholder(lines: &[Line<'_>], area: Rect, buf: &mut Buffer) {
    for (i, line) in lines.iter().enumerate() {
        if i as u16 >= area.height {
            break;
        }
        let mut x = area.x;
        for span in &line.spans {
            if x >= area.x + area.width {
                break;
            }
            let style = Style::new().fg(TEXT).patch(span.style);
            buf.set_stringn(
                x,
                area.y + i as u16,
                &span.content,
                (area.x + area.width - x) as usize,
                style,
            );
            x = x.saturating_add(span.width() as u16);
        }
    }
}

