//! Tipos de mensagem do fluxo unidirecional.
//!
//! - [`AppEvent`]: backend → app (dados novos, toasts, degradação).
//! - [`Action`]: input/app → backends (comandos).

pub mod input;

use crossterm::event::KeyEvent;

use crate::backend::storage::StorageSnapshot;
use crate::backend::system::SystemSnapshot;

/// Sender de eventos usado pelos backends.
pub type EventTx = tokio::sync::mpsc::UnboundedSender<AppEvent>;

/// Identidade estável de um dispositivo/objeto UDisks2 — o caminho do objeto
/// D-Bus (ex.: `/org/freedesktop/UDisks2/block_devices/sdb1`). Usado em vez do
/// nó `/dev/sdX` (que pode trocar entre replugs) para referenciar drives e
/// partições através dos canais de evento.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeviceId(pub String);

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
    /// Novo snapshot da árvore de discos/partições (UDisks2). Boxed pelo
    /// mesmo motivo de `System`.
    Storage(Box<StorageSnapshot>),
    /// Notificação para a statusline.
    Toast(Toast),
    /// Um serviço de sistema está indisponível/pendente.
    ServiceDegraded { name: &'static str, reason: String },
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
    Left,
    Right,
    Enter,
    Refresh,
    ToggleHelp,
    /// Abre/fecha o modal interativo de configurações (tecla `c`/`C`).
    ToggleConfig,
    /// Salva as configurações em disco (tecla `s` no modal).
    SaveConfig,
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
    /// Monta a partição/filesystem identificado (broadcast para o backend).
    StorageMount(DeviceId),
    /// Desmonta a partição/filesystem identificado (broadcast para o backend).
    StorageUnmount(DeviceId),
    /// Desmonta tudo e ejeta o drive identificado (broadcast para o backend).
    StorageEject(DeviceId),
    /// Solicita um refresh imediato da árvore de discos (tecla `r`).
    StorageRefresh,
    /// Intenção de tecla (`m`) sobre o item selecionado na aba Storage; o
    /// `App` resolve a seleção atual e emite `StorageMount`/`StorageUnmount`.
    StorageMountToggleSelected,
    /// Intenção de tecla (`e`) sobre o drive selecionado na aba Storage; o
    /// `App` resolve a seleção atual e emite `StorageEject`.
    StorageEjectSelected,
    /// Tecla não mapeada — repassada para PTY quando a aba tem foco de terminal.
    Raw(KeyEvent),
}
