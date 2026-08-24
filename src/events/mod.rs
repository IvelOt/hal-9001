//! Tipos de mensagem do fluxo unidirecional.
//!
//! - [`AppEvent`]: backend → app (dados novos, toasts, degradação).
//! - [`Action`]: input/app → backends (comandos).

pub mod input;

use std::path::PathBuf;

use crossterm::event::KeyEvent;

use crate::backend::audio::AudioSnapshot;
use crate::backend::bluetooth::BluetoothSnapshot;
use crate::backend::display::DisplaySnapshot;
use crate::backend::network::NetworkSnapshot;
use crate::backend::storage::{StorageSnapshot, VentoyIsoEntry};
use crate::backend::system::SystemSnapshot;

/// Sender de eventos usado pelos backends.
pub type EventTx = tokio::sync::mpsc::UnboundedSender<AppEvent>;

/// Sessão de PTY endereçada por uma `Action`/`AppEvent` de terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PtyTarget {
    /// Aba 7 — Gerenciador de Arquivos (Yazi).
    Files,
    /// Aba 8 — Terminal Deck.
    Terminal,
}

/// Cor de uma célula VT100, em forma neutra (sem depender de `ratatui`) —
/// espelha `vt100::Color`. A conversão para `ratatui::style::Color` acontece
/// na camada de render (`ui::terminal`/`ui::files`), preservando `events`
/// livre de dependências de UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PtyColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Uma célula da grade VT100 (caractere + atributos visuais).
#[derive(Debug, Clone, PartialEq)]
pub struct PtyCell {
    pub ch: char,
    pub fg: PtyColor,
    pub bg: PtyColor,
    pub bold: bool,
    pub underline: bool,
    pub inverse: bool,
    pub italic: bool,
}

impl Default for PtyCell {
    /// Célula "vazia" — espaço em branco, sem atributos. Usado para células
    /// fora dos limites do grid do `vt100::Screen` (ex.: continuação de
    /// caractere largo). `char::default()` seria `'\0'`, que renderizaria
    /// como um glifo de controle em vez de um espaço em branco.
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: PtyColor::default(),
            bg: PtyColor::default(),
            bold: false,
            underline: false,
            inverse: false,
            italic: false,
        }
    }
}

/// Snapshot completo da grade de uma sessão PTY, pronto para render — gerado
/// pela thread leitora em `backend::pty` a partir do `vt100::Parser`.
#[derive(Debug, Clone, PartialEq)]
pub struct PtyScreenSnapshot {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Vec<PtyCell>>,
    pub cursor: (u16, u16),
    pub cursor_visible: bool,
}

/// Identidade estável de um dispositivo/objeto UDisks2 — o caminho do objeto
/// D-Bus (ex.: `/org/freedesktop/UDisks2/block_devices/sdb1`). Usado em vez do
/// nó `/dev/sdX` (que pode trocar entre replugs) para referenciar drives e
/// partições através dos canais de evento.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Success,
            text: text.into(),
        }
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Warning,
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
    /// Novo snapshot de rede e Wi-Fi (NetworkManager). Boxed pelo mesmo motivo.
    Network(Box<NetworkSnapshot>),
    /// Flag de estado de escaneamento de redes sem fio.
    NetworkScanning(bool),
    /// Novo snapshot de dispositivos Bluetooth (BlueZ). Boxed pelo mesmo motivo.
    Bluetooth(Box<BluetoothSnapshot>),
    /// Flag de estado de escaneamento de dispositivos Bluetooth.
    BluetoothScanning(bool),
    /// Novo snapshot de áudio e mixer (PipeWire/PulseAudio). Boxed pelo mesmo motivo.
    Audio(Box<AudioSnapshot>),
    /// Novo snapshot de telas e monitores (X11 / xrandr). Boxed pelo mesmo motivo.
    Display(Box<DisplaySnapshot>),
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
    /// Listagem (atualizada) das ISOs presentes em `<mount>/ISOs/` da
    /// partição de dados identificada, junto do espaço livre restante nela.
    StorageMultibootIsoList {
        device_id: String,
        entries: Vec<VentoyIsoEntry>,
        free_bytes: Option<u64>,
    },
    /// Progresso de cópia de uma ISO para `<mount>/ISOs/`.
    StorageMultibootIsoCopyProgress {
        device_id: String,
        bytes_written: u64,
        total_bytes: u64,
    },
    /// Conclusão (sucesso ou falha) da cópia de uma ISO para o multi-boot.
    StorageMultibootIsoCopyDone {
        device_id: String,
        result: Result<String, String>,
    },
    /// Conclusão (sucesso ou falha) da remoção de uma ISO do multi-boot.
    StorageMultibootIsoRemoveDone {
        device_id: String,
        result: Result<String, String>,
    },
    /// Novo frame renderizado da grade VT100 de uma sessão PTY (Módulos 7/8).
    /// Boxed pelo mesmo motivo dos demais snapshots grandes.
    PtyScreenUpdate {
        target: PtyTarget,
        screen: Box<PtyScreenSnapshot>,
    },
    /// A sessão PTY solicitada não pôde ser iniciada (ex.: `yazi` ausente do
    /// `$PATH`) — distinto de `ServiceDegraded` para permitir que a aba
    /// Arquivos renderize o cartão de instruções de instalação específico.
    PtyUnavailable { target: PtyTarget, reason: String },
    /// O processo filho da sessão PTY encerrou (o backend pode reiniciá-lo
    /// automaticamente uma vez).
    PtyExited { target: PtyTarget },
}

/// Solicitação do backend de Storage para obter a senha de sudo através do
/// campo/modal nativo da TUI (mascarado com `•`), em vez de suspender o
/// terminal para exibir o prompt real de `pkexec`/`sudo` (fluxo antigo,
/// substituído pela execução via `sudo -S` com a senha enviada por stdin).
///
/// Trafega num canal dedicado (`SudoPasswordTx`), separado de `AppEvent`,
/// porque `oneshot::Sender` não implementa `Clone`/`Debug` — e `AppEvent`
/// precisa de ambos. O loop principal (`lib::run`) consome este canal e
/// repassa a solicitação ao `App`, que abre o modal; ao confirmar (`Enter`)
/// ou cancelar (`Esc`), o `App` responde diretamente pelo oneshot guardado em
/// `respond` — `Some(senha)` ou `None` (cancelado pelo usuário).
pub struct SudoPasswordRequest {
    /// Rótulo da operação/dispositivo exibido no modal (ex.: "Formatar
    /// /dev/sdb1").
    pub label: String,
    /// `Some(mensagem)` quando esta solicitação é uma nova tentativa após
    /// senha incorreta na tentativa anterior — exibida como erro no modal.
    pub retry_error: Option<String>,
    pub respond: tokio::sync::oneshot::Sender<Option<String>>,
}

/// Sender usado pelo backend de Storage para solicitar a senha de sudo (ver
/// [`SudoPasswordRequest`]).
pub type SudoPasswordTx = tokio::sync::mpsc::UnboundedSender<SudoPasswordRequest>;

/// Comandos difundidos para os backends. Precisa ser `Clone` para o
/// canal `broadcast`.
#[derive(Debug, Clone, PartialEq)]
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
    /// Intenção de tecla (`e`) sobre o item selecionado na aba Storage; o
    /// `App` resolve a seleção atual e emite `StorageEject`.
    StorageEjectSelected,
    /// Tecla `f`/`F`: abre o modal interativo de formatação para a partição
    /// ou drive selecionado.
    StorageFormatOpen,
    /// Tecla `i`/`I`/`g`/`b`: abre o wizard do gravador de ISO (ISO Flasher).
    StorageFlasherOpen,
    /// Dispara a formatação da partição/drive `device_id` com o filesystem
    /// `fs_type` e o rótulo `label` informados.
    StorageFormat {
        device_id: String,
        fs_type: String,
        label: String,
    },
    /// Solicita o cálculo assíncrono do SHA256 do arquivo `.iso` informado.
    StorageChecksumIso(String),
    /// Inicia a gravação em streaming do `.iso` no dispositivo de bloco
    /// identificado.
    StorageFlashIso {
        device_id: String,
        iso_path: String,
    },
    /// Cancela uma gravação de ISO em curso para o dispositivo identificado.
    StorageFlashCancel {
        device_id: String,
    },
    /// Tecla `B`: prepara partição para multi-boot leve.
    StorageMultibootPrepareOpen,
    /// Prepara `device_id` para multi-boot.
    StorageMultibootPrepare {
        device_id: String,
    },
    /// Tecla `G`: abre o gerenciador de ISOs multi-boot.
    StorageMultibootIsoManagerOpen,
    /// Lista ISOs presentes em `ISOs/`.
    StorageMultibootListIsos {
        device_id: String,
    },
    /// Copia `src_path` para `<mount>/ISOs/`.
    StorageMultibootAddIso {
        device_id: String,
        src_path: String,
    },
    /// Remove `file_name` de `<mount>/ISOs/`.
    StorageMultibootRemoveIso {
        device_id: String,
        file_name: String,
    },
    /// Caractere digitado num campo de texto de um modal de storage.
    StorageModalChar(char),
    /// Apaga o último caractere do campo de texto ativo num modal de storage.
    StorageModalBackspace,
    /// Tecla `Delete`: remove item num modal de storage.
    StorageModalDelete,
    /// Tecla dedicada (F3) que abre o seletor de arquivos.
    StorageModalOpenPicker,
    /// Ações de Rede e Wi-Fi (Módulo 2)
    NetworkRescan,
    NetworkToggleRadio,
    NetworkConnect {
        ap_id: String,
        ssid: String,
        password: Option<String>,
    },
    NetworkDisconnect(DeviceId),
    NetworkForget(String),
    NetworkModalChar(char),
    NetworkModalBackspace,
    /// Ações de Bluetooth (Módulo 3)
    BluetoothRescan,
    BluetoothToggleRadio,
    BluetoothConnect(DeviceId),
    BluetoothDisconnect(DeviceId),
    BluetoothPair(DeviceId),
    BluetoothForget(DeviceId),
    BluetoothToggleBlock(DeviceId),
    /// Ações do Mixer de Áudio (Módulo 5)
    AudioSetVolume { node_id: u32, volume: f32 },
    AudioVolumeUp(u32, f32),
    AudioVolumeDown(u32, f32),
    AudioToggleMute(u32),
    AudioSetDefault(u32),
    AudioSelectCategory(usize),
    /// Ações de Telas & Monitores (Módulo 6)
    DisplaySetLayout(crate::backend::display::DisplayLayoutMode),
    DisplaySetResolution { display: String, mode: String, rate: Option<f32> },
    DisplaySetPrimary(String),
    /// Tecla não mapeada — repassada para PTY quando a aba tem foco de terminal.
    Raw(KeyEvent),
    /// Bytes de input a escrever na sessão PTY `target` (Módulos 7/8),
    /// já codificados em sequências VT100/xterm por `events::input`.
    PtyInput { target: PtyTarget, bytes: Vec<u8> },
    /// Redimensiona a sessão PTY `target` para `cols`x`rows`.
    PtyResize {
        target: PtyTarget,
        cols: u16,
        rows: u16,
    },
    /// Tecla `Enter` nas abas Arquivos/Terminal com a sessão pronta: dá foco
    /// de teclado ao PTY ativo.
    PtyFocus,
    /// Leader `Ctrl-a` ou `Esc` com o PTY em foco: devolve o foco ao chrome
    /// da TUI (tabbar/atalhos globais).
    PtyUnfocus,
}
