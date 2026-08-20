//! Tipos de mensagem do fluxo unidirecional.
//!
//! - [`AppEvent`]: backend → app (dados novos, toasts, degradação).
//! - [`Action`]: input/app → backends (comandos).

pub mod input;

use crossterm::event::KeyEvent;

use crate::backend::system::SystemSnapshot;

/// Sender de eventos usado pelos backends.
pub type EventTx = tokio::sync::mpsc::UnboundedSender<AppEvent>;

/// Nível de severidade de um toast/notificação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

/// Notificação efêmera exibida na statusline.
#[derive(Debug, Clone)]
pub struct Toast {
    pub level: ToastLevel,
    pub text: String,
}

impl Toast {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Info,
            text: text.into(),
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Error,
            text: text.into(),
        }
    }
}

/// Eventos produzidos pelos backends e consumidos pelo `App`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Novo snapshot de sistema (sysinfo). Boxed por ser bem maior que os
    /// demais variantes, evitando inflar o tamanho do enum.
    System(Box<SystemSnapshot>),
    /// Notificação para a statusline.
    Toast(Toast),
    /// Um serviço de sistema está indisponível/pendente.
    ServiceDegraded {
        name: &'static str,
        reason: String,
    },
}

/// Comandos difundidos para os backends. Precisa ser `Clone` para o
/// canal `broadcast`.
#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    NextTab,
    PrevTab,
    SelectTab(usize),
    Up,
    Down,
    Enter,
    Refresh,
    ToggleHelp,
    /// Alterna o Overview entre exibição Padrão e Detalhada (tecla `.`).
    ToggleDetail,
    /// Aumenta o brilho da tela em um passo (tecla `B`/`+`/`=`).
    BrightnessUp,
    /// Diminui o brilho da tela em um passo (tecla `b`/`-`).
    BrightnessDown,
    /// Aumenta o volume do áudio em um passo (tecla `V`/`]`).
    VolumeUp,
    /// Diminui o volume do áudio em um passo (tecla `v`/`[`).
    VolumeDown,
    /// Alterna o mudo do áudio padrão (tecla `m`).
    ToggleMute,
    /// Cicla o perfil de energia (Economia→Equilibrado→Desempenho, tecla `p`/`P`).
    CyclePowerProfile,
    /// Redesenho solicitado (ex.: resize). Sem efeito de estado.
    Redraw,
    /// Tecla não mapeada — repassada para PTY quando a aba tem foco de terminal.
    Raw(KeyEvent),
}
