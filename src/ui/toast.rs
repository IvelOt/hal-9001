//! Sistema de notificações **Toast** do HAL-9001.
//!
//! Renderiza notificações flutuantes no canto inferior direito do terminal,
//! cada uma com um nível de severidade (Info/Success/Warning/Error) que define a
//! cor da borda e do ícone. As notificações se auto-dispensam após 4 segundos.

use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget, Wrap};

use crate::ui::{ACCENT, BG, CYAN, DANGER, TEXT, WARN};

/// Tempo de vida de uma notificação antes do auto-dismiss.
pub const TOAST_TTL: std::time::Duration = std::time::Duration::from_secs(4);

/// Nível de severidade de uma notificação, controlando cor e ícone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    /// Informação geral — azul/ciano.
    Info,
    /// Sucesso de uma operação — verde.
    Success,
    /// Aviso não crítico — amarelo.
    Warning,
    /// Erro — vermelho.
    Error,
}

impl ToastLevel {
    fn label(self) -> &'static str {
        match self {
            ToastLevel::Info => "INFO",
            ToastLevel::Success => "OK",
            ToastLevel::Warning => "AVISO",
            ToastLevel::Error => "ERRO",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            ToastLevel::Info => "i",
            ToastLevel::Success => "✓",
            ToastLevel::Warning => "!",
            ToastLevel::Error => "×",
        }
    }

    fn fg(self) -> ratatui::style::Color {
        match self {
            ToastLevel::Info => CYAN,
            ToastLevel::Success => ACCENT,
            ToastLevel::Warning => WARN,
            ToastLevel::Error => DANGER,
        }
    }
}

/// Uma única notificação em exibição.
#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub level: ToastLevel,
    pub timestamp: Instant,
}

impl Toast {
    pub fn new(message: impl Into<String>, level: ToastLevel) -> Self {
        Self {
            message: message.into(),
            level,
            timestamp: Instant::now(),
        }
    }

    pub fn info(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Info)
    }

    pub fn success(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Success)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Warning)
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self::new(message, ToastLevel::Error)
    }

    /// `true` se a notificação já venceu (deve ser dispensada).
    pub fn expired(&self) -> bool {
        self.timestamp.elapsed() >= TOAST_TTL
    }
}

/// Widget que renderiza uma pilha de toasts no canto inferior direito.
pub struct ToastBar {
    toasts: Vec<Toast>,
}

impl ToastBar {
    /// Cria o renderizador de toasts a partir de uma pilha de notificações
    /// (espera-se que a pilha chame `prune` antes de renderizar).
    pub fn new(toasts: Vec<Toast>) -> Self {
        Self { toasts }
    }

    /// Remove as notificações vencidas e retorna as restantes.
    pub fn prune(toasts: &mut Vec<Toast>) {
        toasts.retain(|toast| !toast.expired());
    }
}

impl Widget for ToastBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.toasts.is_empty() || area.width < 12 || area.height < 1 {
            return;
        }

        let width = (area.width.min(56)).max(12);
        let mut y = area.y + area.height.saturating_sub(1);

        for toast in self.toasts.iter().rev() {
            // Altura do toast = 3 (borda + conteúdo).
            if y < 3 {
                break;
            }
            let height = 3u16;
            let rect = Rect::new(
                area.x + area.width.saturating_sub(width),
                y.saturating_sub(height - 1),
                width,
                height,
            );
            render_toast(toast, rect, buf);
            if y >= height {
                y -= height;
            } else {
                break;
            }
        }
    }
}

/// Desenha uma única notificação (borda, ícone, rótulo e mensagem).
fn render_toast(toast: &Toast, area: Rect, buf: &mut Buffer) {
    let color = toast.level.fg();
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(color))
        .style(Style::new().bg(BG))
        .title(Line::from(Span::styled(
            format!(" {} ", toast.level.label()),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        )))
        .title_style(Style::new().fg(color));

    let inner = block.inner(area);
    buf.set_style(area, Style::new().bg(BG));
    block.render(area, buf);

    if inner.width < 2 {
        return;
    }

    let icon = Span::styled(
        format!(" {}", toast.level.icon()),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    );
    let message = Span::styled(toast.message.clone(), Style::new().fg(TEXT));
    let content = Line::from(vec![icon, Span::styled(" ", Style::new()), message]);

    let max_lines = inner.height.max(1) - 1;
    let paragraph = ratatui::widgets::Paragraph::new(content)
        .wrap(Wrap { trim: true })
        .style(Style::new().bg(BG));

    // Renderiza sobre Clear para não "vazar" células atrás do toast.
    let content_area = Rect::new(inner.x, inner.y, inner.width, inner.height.min(max_lines + 1));
    Clear.render(area, buf);
    paragraph.render(content_area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_has_level_and_message() {
        let toast = Toast::success("montado");
        assert_eq!(toast.message, "montado");
        assert_eq!(toast.level, ToastLevel::Success);
    }

    #[test]
    fn fresh_toast_not_expired() {
        let toast = Toast::info("ok");
        assert!(!toast.expired());
    }

    #[test]
    fn prune_removes_expired() {
        let mut toasts = vec![Toast::info("uma")];
        // Simula vencimento.
        toasts[0].timestamp = Instant::now() - TOAST_TTL - std::time::Duration::from_secs(1);
        toasts.push(Toast::error("duas"));
        ToastBar::prune(&mut toasts);
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].level, ToastLevel::Error);
    }

    #[test]
    fn empty_builds() {
        let bar = ToastBar::new(vec![]);
        assert!(bar.toasts.is_empty());
    }
}