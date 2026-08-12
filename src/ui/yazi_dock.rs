//! **PTY Dock** embarcado para o Yazi File Manager (Aba Arquivos).
//!
//! O Yazi roda dentro de um pseudo-terminal filho (`portable-pty`) interpretado
//! pelo parser ANSI `vt100`, embarcado no próprio container do Ratatui — no
//! estilo `:terminal` do Neovim. A TUI **não suspende** o raw mode: teclas são
//! convertidas em bytes crus e injetadas no processo, e a tela virtual é
//! renderizada célula a célula dentro da aba.
//!
//! Se o processo do Yazi terminar (ex.: usuário pressiona `q`), o dock pode ser
//! reiniciado automaticamente ao focar a aba de Arquivos novamente
//! ([`YaziDock::ensure_running`]).

use std::cell::Cell;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};

use crate::ai_agent::pty_session::{render_screen, AgentCommand, PtySession, PtyTarget};
use crate::ui::{ACCENT, BG, DANGER, DIM, GRAY, TEXT};

/// Intervalo mínimo entre tentativas de reinício após uma falha de spawn.
const RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// Dock que hospeda o Yazi num PTY embarcado (`portable-pty` + `vt100`).
pub struct YaziDock {
    /// Sessão PTY ativa do Yazi (reconstruída quando o processo termina).
    session: Option<Arc<PtySession>>,
    /// Última dimensão (rows, cols) aplicada ao PTY — evita resizes redundantes.
    last_size: Cell<(u16, u16)>,
    /// Momento da última tentativa de início (backoff de reinício).
    last_attempt: Cell<Option<Instant>>,
    /// Mensagem do último erro de inicialização (ex.: binário ausente).
    error: Option<String>,
}

impl YaziDock {
    /// Cria um dock vazio (ainda sem processo do Yazi).
    pub fn new() -> Self {
        Self {
            session: None,
            last_size: Cell::new((0, 0)),
            last_attempt: Cell::new(None),
            error: None,
        }
    }

    /// Garante que uma sessão do Yazi esteja rodando.
    ///
    /// Reinicia automaticamente quando o processo anterior saiu (ex.: usuário
    /// pressionou `q` dentro do Yazi) ou nunca foi iniciado. Falhas de spawn
    /// respeitam um backoff de [`RETRY_INTERVAL`] para não spammar o fork.
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

    /// Dispara o processo do Yazi dentro de um novo PTY.
    fn start(&mut self) {
        let mut session = PtySession::new(AgentCommand::yazi());
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

    /// Encaminha bytes crus (teclas) para o processo do Yazi.
    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        match &self.session {
            Some(session) => session.write_input(bytes),
            None => Ok(()),
        }
    }

    /// Aplica um fechamento sobre a tela virtual atual do Yazi, se ativo.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> Option<R> {
        self.session
            .as_ref()
            .map(|session| session.with_screen(f))
    }

    /// `true` quando há um processo do Yazi vivo.
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

/// Widget que desenha o PTY Dock do Yazi num `Rect` do Ratatui.
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

        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(DIM))
            .style(Style::new().bg(BG))
            .title(Line::from(
                " ARQUIVOS · YAZI — PTY DOCK ",
            ))
            .title_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD));
        let inner = block.inner(area);
        block.render(area, buf);

        if !self.dock.is_running() {
            let message = match self.dock.error() {
                Some(e) => format!(" Falha ao iniciar o Yazi: {e}"),
                None => " O Yazi não está em execução. Saindo e voltando à aba, ele reinicia."
                    .to_string(),
            };
            render_placeholder(
                &[
                    Line::from(Span::styled(message, Style::new().fg(DANGER))),
                    Line::from(Span::styled(
                        " [Alt+1-5] trocar de aba · [Ctrl+Q] sair",
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
