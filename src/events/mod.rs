//! Tipos de mensagem do fluxo unidirecional.
//!
//! - [`AppEvent`]: backend → app (dados novos, toasts, degradação).
//! - [`Action`]: input/app → backends (comandos).

pub mod input;

use std::path::PathBuf;

use crossterm::event::KeyEvent;

use crate::backend::storage::{StorageSnapshot, VentoyIsoEntry};
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
    /// Progresso do cálculo de SHA256 de uma ISO selecionada no Flasher.
    StorageChecksumProgress { path: PathBuf, pct: f32 },
    /// Cálculo de SHA256 da ISO concluído.
    StorageChecksumDone { path: PathBuf, sha256: String },
    /// Progresso contínuo da gravação de blocos do ISO Flasher (throttled a
    /// ~200ms), emitido pela task `flash_task`.
    StorageFlashProgress {
        bytes_written: u64,
        total_bytes: u64,
        speed_mbps: f64,
        eta_secs: u64,
    },
    /// Conclusão (sucesso ou falha) da gravação de ISO.
    StorageFlashDone {
        device_id: String,
        result: Result<String, String>,
    },
    /// Uma linha de saída (stdout/stderr) do `scripts/ventoy.sh` em execução.
    StorageVentoyProgress { device_id: String, line: String },
    /// Conclusão (sucesso ou falha) da instalação do Ventoy.
    StorageVentoyDone {
        device_id: String,
        result: Result<String, String>,
    },
    /// Listagem (atualizada) das ISOs presentes na partição de dados de um
    /// pendrive Ventoy, junto do espaço livre restante nela.
    StorageVentoyIsoList {
        device_id: String,
        entries: Vec<VentoyIsoEntry>,
        free_bytes: Option<u64>,
    },
    /// Progresso de cópia de uma ISO para a partição de dados do Ventoy.
    StorageVentoyIsoCopyProgress {
        device_id: String,
        bytes_written: u64,
        total_bytes: u64,
    },
    /// Conclusão (sucesso ou falha) da cópia de uma ISO para o Ventoy.
    StorageVentoyIsoCopyDone {
        device_id: String,
        result: Result<String, String>,
    },
    /// Conclusão (sucesso ou falha) da remoção de uma ISO do Ventoy.
    StorageVentoyIsoRemoveDone {
        device_id: String,
        result: Result<String, String>,
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
    /// Tecla `f`: abre o modal de formatação para o item selecionado.
    StorageFormatOpen,
    /// Tecla `g`/`b`: abre o wizard do ISO Flasher para o drive selecionado.
    StorageFlasherOpen,
    /// Formata `device_id` com o sistema de arquivos e rótulo informados.
    /// Rejeitada pelo backend (e pelo `App`) quando o alvo é um disco de
    /// sistema (ver `is_system_disk`).
    StorageFormat {
        device_id: String,
        fs_type: String,
        label: String,
    },
    /// Solicita o cálculo assíncrono do SHA256 do arquivo `.iso` informado.
    StorageChecksumIso(String),
    /// Inicia a gravação em streaming do `.iso` no dispositivo de bloco
    /// identificado. Rejeitada quando o alvo é um disco de sistema.
    StorageFlashIso {
        device_id: String,
        iso_path: String,
    },
    /// Cancela uma gravação de ISO em curso para o dispositivo identificado.
    StorageFlashCancel {
        device_id: String,
    },
    /// Tecla `V`: abre o modal de instalação do Ventoy para o drive selecionado.
    StorageVentoyOpen,
    /// Instala o Ventoy no `device_id` informado via `scripts/ventoy.sh`.
    /// Rejeitada pelo backend (e pelo `App`) quando o alvo é um disco de
    /// sistema (ver `is_system_disk`).
    StorageVentoyInstall {
        device_id: String,
    },
    /// Tecla `i`/`I`: abre o gerenciador de ISOs de um pendrive Ventoy
    /// selecionado (sem efeito se o drive selecionado não for Ventoy).
    StorageVentoyIsoManagerOpen,
    /// Lista (ou relista) as ISOs presentes na partição de dados do Ventoy
    /// identificado por `device_id`, montando-a primeiro se necessário.
    StorageVentoyListIsos { device_id: String },
    /// Copia `src_path` para a partição de dados do pendrive Ventoy
    /// identificado, com progresso em streaming. Rejeitada quando o alvo é
    /// um disco de sistema.
    StorageVentoyAddIso {
        device_id: String,
        src_path: String,
    },
    /// Remove `file_name` da raiz da partição de dados do pendrive Ventoy
    /// identificado. Rejeitada quando o alvo é um disco de sistema.
    StorageVentoyRemoveIso {
        device_id: String,
        file_name: String,
    },
    /// Caractere digitado num campo de texto de um modal de storage (rótulo
    /// do volume, caminho da ISO, confirmação digitada) — ou atalho de
    /// caractere único num modal de navegação (file picker, gerenciador de
    /// ISOs do Ventoy).
    StorageModalChar(char),
    /// Apaga o último caractere do campo de texto ativo num modal de storage.
    StorageModalBackspace,
    /// Tecla `Delete`: remove o item selecionado num modal de storage que
    /// suporte remoção (gerenciador de ISOs do Ventoy).
    StorageModalDelete,
    /// Tecla dedicada (F3) que abre o seletor de arquivos estilo Yazi a
    /// partir de um modal de storage que aceite selecionar uma imagem (ISO
    /// Flasher, adicionar ISO ao Ventoy).
    StorageModalOpenPicker,
    /// Tecla não mapeada — repassada para PTY quando a aba tem foco de terminal.
    Raw(KeyEvent),
}
