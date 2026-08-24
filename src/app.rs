//! Estado global do Assistente de Sistema e roteamento de [`Action`]/[`AppEvent`].
//!
//! `App` é a única fonte da verdade consumida pelo render. A UI é uma função
//! pura de `&App`.

use std::path::PathBuf;
use std::time::Instant;

use tokio::sync::broadcast;

use crate::backend::storage::{primary_partition, DriveInfo, PartitionInfo, StorageSnapshot, VentoyIsoEntry};
use crate::backend::system::SystemSnapshot;
use crate::config::Config;
use crate::events::{Action, AppEvent, Toast};
use crate::ui::file_picker::{self, FileEntry};

/// Sistemas de arquivos ofertados pelo modal de formatação (Épico G).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsChoice {
    Vfat,
    Exfat,
    Ext4,
    Ntfs,
    Btrfs,
}

impl FsChoice {
    pub const ALL: [FsChoice; 5] = [
        FsChoice::Vfat,
        FsChoice::Exfat,
        FsChoice::Ext4,
        FsChoice::Ntfs,
        FsChoice::Btrfs,
    ];

    /// Valor de `type` esperado por `Block.Format` do UDisks2.
    pub fn udisks_type(self) -> &'static str {
        match self {
            FsChoice::Vfat => "vfat",
            FsChoice::Exfat => "exfat",
            FsChoice::Ext4 => "ext4",
            FsChoice::Ntfs => "ntfs",
            FsChoice::Btrfs => "btrfs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FsChoice::Vfat => "FAT32 (vfat)",
            FsChoice::Exfat => "exFAT",
            FsChoice::Ext4 => "ext4",
            FsChoice::Ntfs => "NTFS",
            FsChoice::Btrfs => "btrfs",
        }
    }
}

/// Campo com foco no modal de formatação.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatField {
    Fs,
    Label,
    Confirm,
}

/// Estado do modal de formatação (Épico G).
#[derive(Debug, Clone, PartialEq)]
pub struct FormatModalState {
    pub device_id: String,
    pub target_label: String,
    pub fs_idx: usize,
    pub label: String,
    pub field: FormatField,
}

/// Máquina de estados do wizard do ISO Flasher (Épico H, seção 4.1 do plano).
#[derive(Debug, Clone, PartialEq)]
pub enum FlasherStage {
    SelectIso {
        input: String,
        error: Option<String>,
    },
    Checksumming {
        pct: f32,
    },
    Ready {
        sha256: Option<String>,
    },
    Confirm1,
    Confirm2 {
        typed: String,
    },
    Flashing {
        bytes_written: u64,
        total_bytes: u64,
        speed_mbps: f64,
        eta_secs: u64,
    },
    Done {
        ok: bool,
        message: String,
    },
}

/// Estado do modal do ISO Flasher (Épico H).
#[derive(Debug, Clone, PartialEq)]
pub struct FlasherModalState {
    pub device_id: String,
    pub target_label: String,
    pub target_dev_node: String,
    pub target_size: u64,
    pub iso_path: PathBuf,
    pub iso_size: u64,
    pub stage: FlasherStage,
}

/// Para onde o arquivo escolhido no seletor (Yazi-style) deve ser
/// encaminhado — carrega os dados necessários para reconstruir o modal de
/// origem (Flasher ou gerenciador de ISOs multi-boot) sem precisar manter uma
/// pilha de "modal anterior": ao escolher o arquivo, `App` reconstrói o modal
/// alvo diretamente a partir destes campos.
#[derive(Debug, Clone, PartialEq)]
pub enum FilePickerPurpose {
    FlasherIso {
        device_id: String,
        target_label: String,
        target_dev_node: String,
        target_size: u64,
    },
    MultibootAddIso {
        device_id: String,
        target_label: String,
    },
}

/// Resultado de confirmar a seleção (`Enter`/`l`/`→`) no seletor de arquivos.
#[derive(Debug, Clone, PartialEq)]
pub enum FilePickerOutcome {
    /// Navegação pura (entrou num diretório, ou nada aconteceu).
    None,
    /// Arquivo com extensão reconhecida (`.iso`/`.img`/`.vhd`) escolhido.
    Picked(PathBuf),
    /// Arquivo com extensão não suportada — permanece no seletor com erro.
    Unsupported,
}

/// Estado do seletor de arquivos estilo Yazi (ver `ui::file_picker`).
#[derive(Debug, Clone, PartialEq)]
pub struct FilePickerState {
    pub cwd: PathBuf,
    /// Listagem do diretório atual, já ordenada (diretórios primeiro, depois
    /// arquivos, alfabeticamente sem diferenciar caixa).
    pub entries: Vec<FileEntry>,
    pub selected: usize,
    pub error: Option<String>,
    pub purpose: FilePickerPurpose,
}

impl FilePickerState {
    /// Abre o seletor em `start_dir` (ou no diretório temporário do sistema,
    /// caso `start_dir` não seja um diretório válido).
    pub fn open(start_dir: PathBuf, purpose: FilePickerPurpose) -> Self {
        let cwd = if start_dir.is_dir() {
            start_dir
        } else {
            std::env::temp_dir()
        };
        let mut s = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            error: None,
            purpose,
        };
        s.reload();
        s
    }

    /// Relista o diretório atual, clampeando a seleção ao novo tamanho.
    pub fn reload(&mut self) {
        match file_picker::list_dir(&self.cwd) {
            Ok(entries) => {
                self.selected = if entries.is_empty() {
                    0
                } else {
                    self.selected.min(entries.len() - 1)
                };
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.selected = 0;
                self.error = Some(e);
            }
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }

    /// Sobe para o diretório pai, se houver (sem efeito na raiz `/`).
    pub fn go_up(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.selected = 0;
            self.reload();
        }
    }

    /// Salta diretamente para `path` (atalhos `~`/`d`/`M`/`/`). Superfícia um
    /// erro em vez de entrar num diretório inexistente/ilegível.
    pub fn jump_to(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.cwd = path;
            self.selected = 0;
            self.reload();
        } else {
            self.error = Some(format!("{}", path.display()));
        }
    }

    /// Confirma a seleção atual: desce em diretórios, ou "escolhe" arquivos
    /// com extensão de imagem reconhecida.
    pub fn enter_selected(&mut self) -> FilePickerOutcome {
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            return FilePickerOutcome::None;
        };
        if entry.is_dir {
            self.cwd = entry.path;
            self.selected = 0;
            self.reload();
            FilePickerOutcome::None
        } else if file_picker::is_pickable_for(&self.purpose, &entry.name) {
            FilePickerOutcome::Picked(entry.path)
        } else {
            FilePickerOutcome::Unsupported
        }
    }
}

/// Fase do gerenciador de ISOs multi-boot (tecla `G`).
#[derive(Debug, Clone, PartialEq)]
pub enum MultibootIsoManagerStage {
    Loading,
    Listing {
        entries: Vec<VentoyIsoEntry>,
        selected: usize,
        free_bytes: Option<u64>,
    },
    ConfirmRemove {
        file_name: String,
    },
    Copying {
        bytes_written: u64,
        total_bytes: u64,
        file_name: String,
    },
    Removing {
        file_name: String,
    },
    Error {
        message: String,
    },
}

/// Estado do gerenciador de ISOs multi-boot.
#[derive(Debug, Clone, PartialEq)]
pub struct MultibootIsoManagerState {
    pub device_id: String,
    pub target_label: String,
    pub stage: MultibootIsoManagerStage,
}

/// Modal interativo ativo na aba Storage (mutuamente exclusivo).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum StorageModal {
    #[default]
    None,
    Format(FormatModalState),
    Flasher(FlasherModalState),
    FilePicker(FilePickerState),
    MultibootIsoManager(MultibootIsoManagerState),
}

/// Abas do Assistente de Sistema, na ordem da tabbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Network,
    Bluetooth,
    Storage,
    Audio,
    Displays,
    Files,
    Terminal,
}

use crate::i18n::Language;

impl Tab {
    pub const ALL: [Tab; 8] = [
        Tab::Overview,
        Tab::Network,
        Tab::Bluetooth,
        Tab::Storage,
        Tab::Audio,
        Tab::Displays,
        Tab::Files,
        Tab::Terminal,
    ];

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Tab {
        Tab::ALL.get(i).copied().unwrap_or(Tab::Overview)
    }

    /// Título traduzido para a tabbar no idioma informado.
    pub fn title_in(self, lang: Language) -> &'static str {
        let m = lang.messages();
        match self {
            Tab::Overview => m.tab_overview,
            Tab::Network => m.tab_network,
            Tab::Bluetooth => m.tab_bluetooth,
            Tab::Storage => m.tab_storage,
            Tab::Audio => m.tab_audio,
            Tab::Displays => m.tab_displays,
            Tab::Files => m.tab_files,
            Tab::Terminal => m.tab_terminal,
        }
    }

    /// Título curto para a tabbar (fallback/padrão).
    pub fn title(self) -> &'static str {
        self.title_in(Language::default())
    }
}

/// Estado de uma sessão PTY (Terminal Deck ou Yazi) do ponto de vista da UI.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PtyState {
    /// Sessão solicitada, aguardando o primeiro evento do backend.
    #[default]
    Starting,
    /// A sessão não pôde ser iniciada (ex.: `yazi` ausente do `$PATH`).
    Unavailable(String),
    /// Sessão ativa, com o último frame renderizado.
    Running(Box<crate::events::PtyScreenSnapshot>),
    /// O processo filho encerrou.
    Exited,
}

/// Fase de apresentação: splash animada antes do dashboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Splash,
    Running,
}

/// Estado de um serviço de backend para exibição na UI.
#[derive(Debug, Clone, Default)]
pub struct ServiceStatus {
    pub degraded: Option<String>,
}

/// Estado do modal nativo (senha mascarada com `•`) de autenticação sudo.
#[derive(Debug, Clone, PartialEq)]
pub struct SudoPromptState {
    pub label: String,
    pub password: String,
    pub error: Option<String>,
}

/// Estado do modal de senha Wi-Fi (WPA2/WPA3 PSK).
#[derive(Debug, Clone, PartialEq)]
pub struct WifiPasswordPromptState {
    pub ap_id: String,
    pub ssid: String,
    pub password: String,
    pub error: Option<String>,
}

/// Estado global do aplicativo.
pub struct App {
    pub config: Config,
    /// Idioma ativo resolvido da interface.
    pub lang: Language,
    pub should_quit: bool,
    pub phase: Phase,
    pub active: Tab,
    pub show_help: bool,
    /// Modal interativo de configurações (alternado pela tecla `c`/`C`).
    pub show_config: bool,
    /// Índice da linha selecionada no modal de configurações (0..5).
    pub config_cursor: usize,
    /// Overview em modo detalhado (alternado pela tecla `.`).
    pub detailed_overview: bool,

    /// Índice selecionado por aba (para listas navegáveis).
    pub selection: [usize; 8],

    /// Último snapshot de sistema.
    pub system: Option<SystemSnapshot>,

    /// Último snapshot da árvore de discos/partições (Módulo 4).
    pub storage: Option<StorageSnapshot>,
    /// Índice do drive selecionado na lista da aba Storage.
    pub storage_selected: usize,
    /// Modal interativo ativo na aba Storage (formatação ou ISO Flasher).
    pub storage_modal: StorageModal,
    /// Modal nativo de senha de sudo.
    pub sudo_prompt: Option<SudoPromptState>,
    sudo_respond: Option<tokio::sync::oneshot::Sender<Option<String>>>,

    /// Estado do Wi-Fi e rede (Módulo 2).
    pub network: Option<Box<crate::backend::network::NetworkSnapshot>>,
    pub network_selected: usize,
    pub network_scanning: bool,
    pub wifi_prompt: Option<WifiPasswordPromptState>,

    /// Estado do Bluetooth e dispositivos (Módulo 3).
    pub bluetooth: Option<Box<crate::backend::bluetooth::BluetoothSnapshot>>,
    pub bluetooth_selected: usize,
    pub bluetooth_scanning: bool,

    /// Estado do Mixer de Áudio e dispositivos (Módulo 5).
    pub audio: Option<Box<crate::backend::audio::AudioSnapshot>>,
    pub audio_selected: usize,
    pub audio_category: usize,

    /// Estado das Telas e Monitores (Módulo 6).
    pub displays: Option<Box<crate::backend::display::DisplaySnapshot>>,
    pub display_selected: usize,

    /// Estado da sessão PTY da aba Arquivos (Módulo 7 — Yazi).
    pub files_pty: PtyState,
    /// Estado da sessão PTY da aba Terminal Deck (Módulo 8).
    pub terminal_pty: PtyState,
    /// `true` quando o teclado está capturado pela sessão PTY da aba ativa
    /// (Arquivos ou Terminal) em vez do chrome/atalhos globais da TUI.
    pub pty_focused: bool,
    /// Último tamanho de grade (cols, rows) informado às sessões PTY, usado
    /// para só emitir `Action::PtyResize` quando o tamanho realmente muda.
    pub last_pty_size: (u16, u16),

    /// Status por nome de serviço (network, bluetooth, ...).
    pub services: std::collections::HashMap<&'static str, ServiceStatus>,

    /// Toast atual (o mais recente) e quando expira.
    pub toast: Option<(Toast, Instant)>,

    started: Instant,
}

impl App {
    pub fn new(config: Config) -> Self {
        let phase = if config.splash.enabled {
            Phase::Splash
        } else {
            Phase::Running
        };
        let lang = config.ui.resolved_language();
        Self {
            config,
            lang,
            should_quit: false,
            phase,
            active: Tab::Overview,
            show_help: false,
            show_config: false,
            config_cursor: 0,
            detailed_overview: false,
            selection: [0; 8],
            system: None,
            storage: None,
            storage_selected: 0,
            storage_modal: StorageModal::None,
            sudo_prompt: None,
            sudo_respond: None,
            network: None,
            network_selected: 0,
            network_scanning: false,
            wifi_prompt: None,
            bluetooth: None,
            bluetooth_selected: 0,
            bluetooth_scanning: false,
            audio: None,
            audio_selected: 0,
            audio_category: 0,
            displays: None,
            display_selected: 0,
            files_pty: PtyState::Starting,
            terminal_pty: PtyState::Starting,
            pty_focused: false,
            last_pty_size: (0, 0),
            services: std::collections::HashMap::new(),
            toast: None,
            started: Instant::now(),
        }
    }

    /// Milissegundos desde o boot — usado pela animação da splash.
    pub fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    /// Chamado a cada frame: gerencia transições temporais.
    pub fn on_tick(&mut self) {
        if self.phase == Phase::Splash && self.elapsed_ms() as u64 >= self.config.splash.min_ms {
            self.phase = Phase::Running;
        }
        // Expira toast após 4s.
        if let Some((_, at)) = &self.toast {
            if at.elapsed().as_secs() >= 4 {
                self.toast = None;
            }
        }
    }

    /// Consome um evento de backend, mutando o estado. Devolve ações de
    /// acompanhamento (ex.: relistar ISOs multi-boot após uma cópia/remoção
    /// concluída) que o chamador deve repassar ao `action_tx` — `handle_event`
    /// não recebe o `Sender` diretamente para não quebrar a assinatura usada
    /// em dezenas de testes existentes.
    pub fn handle_event(&mut self, event: AppEvent) -> Vec<Action> {
        let mut follow_up: Vec<Action> = Vec::new();
        match event {
            AppEvent::System(snap) => self.system = Some(*snap),
            AppEvent::Storage(snap) => self.storage = Some(*snap),
            AppEvent::Network(snap) => {
                let ap_count = snap.access_points.len();
                self.network = Some(snap);
                if ap_count > 0 && self.network_selected >= ap_count {
                    self.network_selected = ap_count - 1;
                }
            }
            AppEvent::NetworkScanning(flag) => self.network_scanning = flag,
            AppEvent::Bluetooth(snap) => {
                let dev_count = snap.devices.len();
                self.bluetooth = Some(snap);
                if dev_count > 0 && self.bluetooth_selected >= dev_count {
                    self.bluetooth_selected = dev_count - 1;
                }
            }
            AppEvent::BluetoothScanning(flag) => self.bluetooth_scanning = flag,
            AppEvent::Audio(snap) => {
                let cat = match self.audio_category {
                    0 => crate::backend::audio::AudioCategory::Sink,
                    1 => crate::backend::audio::AudioCategory::AppStream,
                    _ => crate::backend::audio::AudioCategory::Source,
                };
                let node_count = snap.nodes_for_category(cat).len();
                self.audio = Some(snap);
                if node_count > 0 && self.audio_selected >= node_count {
                    self.audio_selected = node_count - 1;
                }
            }
            AppEvent::Display(snap) => {
                let count = snap.displays.len();
                self.displays = Some(snap);
                if count > 0 && self.display_selected >= count {
                    self.display_selected = count - 1;
                }
            }
            AppEvent::Toast(toast) => self.toast = Some((toast, Instant::now())),
            AppEvent::ServiceDegraded { name, reason } => {
                self.services.insert(
                    name,
                    ServiceStatus {
                        degraded: Some(reason),
                    },
                );
            }
            AppEvent::StorageChecksumProgress { path, pct } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if s.iso_path == path {
                        if let FlasherStage::Checksumming { pct: p } = &mut s.stage {
                            *p = pct;
                        }
                    }
                }
            }
            AppEvent::StorageChecksumDone { path, sha256 } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if s.iso_path == path {
                        s.stage = FlasherStage::Ready {
                            sha256: Some(sha256),
                        };
                    }
                }
            }
            AppEvent::StorageFlashProgress {
                bytes_written,
                total_bytes,
                speed_mbps,
                eta_secs,
            } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if let FlasherStage::Flashing {
                        bytes_written: bw,
                        total_bytes: tb,
                        speed_mbps: sp,
                        eta_secs: eta,
                    } = &mut s.stage
                    {
                        *bw = bytes_written;
                        *tb = total_bytes;
                        *sp = speed_mbps;
                        *eta = eta_secs;
                    }
                }
            }
            AppEvent::StorageFlashDone { device_id, result } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        s.stage = match result {
                            Ok(msg) => FlasherStage::Done {
                                ok: true,
                                message: msg,
                            },
                            Err(err) => FlasherStage::Done {
                                ok: false,
                                message: err,
                            },
                        };
                    }
                }
            }
            AppEvent::StorageMultibootIsoList {
                device_id,
                entries,
                free_bytes,
            } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        s.stage = MultibootIsoManagerStage::Listing {
                            entries,
                            selected: 0,
                            free_bytes,
                        };
                    }
                }
            }
            AppEvent::StorageMultibootIsoCopyProgress {
                device_id,
                bytes_written,
                total_bytes,
            } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        if let MultibootIsoManagerStage::Copying {
                            bytes_written: bw,
                            total_bytes: tb,
                            ..
                        } = &mut s.stage
                        {
                            *bw = bytes_written;
                            *tb = total_bytes;
                        }
                    }
                }
            }
            AppEvent::StorageMultibootIsoCopyDone { device_id, result } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        match result {
                            Ok(_) => {
                                s.stage = MultibootIsoManagerStage::Loading;
                                follow_up.push(Action::StorageMultibootListIsos {
                                    device_id: device_id.clone(),
                                });
                            }
                            Err(e) => {
                                s.stage = MultibootIsoManagerStage::Error { message: e };
                            }
                        }
                    }
                }
            }
            AppEvent::StorageMultibootIsoRemoveDone { device_id, result } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        match result {
                            Ok(_) => {
                                s.stage = MultibootIsoManagerStage::Loading;
                                follow_up.push(Action::StorageMultibootListIsos {
                                    device_id: device_id.clone(),
                                });
                            }
                            Err(e) => {
                                s.stage = MultibootIsoManagerStage::Error { message: e };
                            }
                        }
                    }
                }
            }
            AppEvent::PtyScreenUpdate { target, screen } => match target {
                crate::events::PtyTarget::Files => self.files_pty = PtyState::Running(screen),
                crate::events::PtyTarget::Terminal => self.terminal_pty = PtyState::Running(screen),
            },
            AppEvent::PtyUnavailable { target, reason } => match target {
                crate::events::PtyTarget::Files => self.files_pty = PtyState::Unavailable(reason),
                crate::events::PtyTarget::Terminal => {
                    self.terminal_pty = PtyState::Unavailable(reason)
                }
            },
            AppEvent::PtyExited { target } => {
                let is_active_target = (target == crate::events::PtyTarget::Files
                    && self.active == Tab::Files)
                    || (target == crate::events::PtyTarget::Terminal
                        && self.active == Tab::Terminal);
                if is_active_target {
                    self.pty_focused = false;
                }
                match target {
                    crate::events::PtyTarget::Files => self.files_pty = PtyState::Exited,
                    crate::events::PtyTarget::Terminal => self.terminal_pty = PtyState::Exited,
                }
            }
        }
        follow_up
    }

    /// Navega para o campo anterior no modal de configuração.
    pub fn config_prev_field(&mut self) {
        self.config_cursor = if self.config_cursor == 0 {
            5
        } else {
            self.config_cursor - 1
        };
    }

    /// Navega para o próximo campo no modal de configuração.
    pub fn config_next_field(&mut self) {
        self.config_cursor = (self.config_cursor + 1) % 6;
    }

    /// Cicla o valor da opção selecionada para a esquerda/anterior.
    pub fn config_prev_value(&mut self) {
        self.cycle_config_value(false);
    }

    /// Cicla o valor da opção selecionada para a direita/próximo.
    pub fn config_next_value(&mut self) {
        self.cycle_config_value(true);
    }

    fn cycle_config_value(&mut self, forward: bool) {
        match self.config_cursor {
            0 => {
                // Language: ["auto", "pt-BR", "en-US", "es-ES"]
                let options = ["auto", "pt-BR", "en-US", "es-ES"];
                let cur = options
                    .iter()
                    .position(|&s| s.eq_ignore_ascii_case(&self.config.ui.language))
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.ui.language = options[next].to_string();
                self.lang = self.config.ui.resolved_language();
            }
            1 => {
                // Theme: ["hal", "mono"]
                let options = ["hal", "mono"];
                let cur = options
                    .iter()
                    .position(|&s| s.eq_ignore_ascii_case(&self.config.theme.name))
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.theme.name = options[next].to_string();
            }
            2 => {
                // Logo: ["auto", "main", "medium", "compact", "none"]
                let options = ["auto", "main", "medium", "compact", "none"];
                let cur = options
                    .iter()
                    .position(|&s| s.eq_ignore_ascii_case(&self.config.overview.ascii))
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.overview.ascii = options[next].to_string();
            }
            3 => {
                // Icons: [true, false]
                self.config.ui.icons = !self.config.ui.icons;
            }
            4 => {
                // FPS / frame_ms: [33 (30fps), 16 (60fps), 66 (15fps)]
                let options = [33u64, 16, 66];
                let cur = options
                    .iter()
                    .position(|&ms| ms == self.config.ui.frame_ms)
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.ui.frame_ms = options[next];
            }
            5 => {
                // Splash: [true, false]
                self.config.splash.enabled = !self.config.splash.enabled;
            }
            _ => {}
        }
    }

    /// Índice do drive atualmente selecionado na lista da aba Storage, já
    /// clampeado ao tamanho atual (evita índice fora dos limites após um
    /// refresh que encolheu a lista de drives).
    pub fn storage_drive_index(&self) -> Option<usize> {
        let snap = self.storage.as_ref()?;
        if snap.drives.is_empty() {
            return None;
        }
        Some(self.storage_selected.min(snap.drives.len() - 1))
    }

    /// Drive selecionado e sua partição "primária" (ver
    /// [`crate::backend::storage::primary_partition`]) na aba Storage — a
    /// visão simplificada de um item por drive não expõe mais navegação por
    /// partição individual; ações como montar/desmontar e multi-boot sempre
    /// operam sobre a partição primária resolvida automaticamente.
    pub fn storage_selection(&self) -> Option<(&DriveInfo, Option<&PartitionInfo>)> {
        let snap = self.storage.as_ref()?;
        let idx = self.storage_drive_index()?;
        let drive = snap.drive(idx)?;
        Some((drive, primary_partition(drive)))
    }

    /// Tecla `m`: monta a partição selecionada, ou a desmonta se já montada.
    /// Sem efeito sobre a linha de um drive (sem partição selecionada).
    fn storage_mount_toggle(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((_, Some(partition))) = self.storage_selection() else {
            return;
        };
        let action = if partition.is_mounted() {
            Action::StorageUnmount(partition.id.clone())
        } else {
            Action::StorageMount(partition.id.clone())
        };
        let _ = action_tx.send(action);
    }

    /// Tecla `e`: ejeta o drive selecionado (ou o drive-pai de uma partição
    /// selecionada), com a trava de segurança como primeira camada de defesa.
    fn storage_eject_selected(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((drive, _)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let _ = action_tx.send(Action::StorageEject(drive.id.clone()));
    }

    /// `true` quando um modal de storage (formatação, flasher ou gerenciador
    /// aberto — usado para desviar a navegação/teclado da aba.
    pub fn storage_modal_open(&self) -> bool {
        !matches!(self.storage_modal, StorageModal::None)
    }

    /// `true` quando o modal nativo de senha de sudo está aberto — tem
    /// prioridade máxima no roteamento de teclado (`InputStream::next`).
    pub fn sudo_prompt_open(&self) -> bool {
        self.sudo_prompt.is_some()
    }

    /// Abre o modal nativo de senha de sudo a partir de uma solicitação do
    /// backend de Storage, guardando o canal de resposta para ser respondido
    /// diretamente ao confirmar (`Enter`) ou cancelar (`Esc`) — ver
    /// [`SudoPromptState`] e `crate::events::SudoPasswordRequest`.
    pub fn open_sudo_prompt(&mut self, req: crate::events::SudoPasswordRequest) {
        self.sudo_prompt = Some(SudoPromptState {
            label: req.label,
            password: String::new(),
            error: req.retry_error,
        });
        self.sudo_respond = Some(req.respond);
    }

    /// Roteia uma `Action` para o modal nativo de senha de sudo (digitação
    /// mascarada, confirmação e cancelamento). Chamado com prioridade máxima
    /// por `dispatch`, antes de qualquer outro modal.
    fn dispatch_sudo_prompt(&mut self, action: Action) {
        let Some(state) = &mut self.sudo_prompt else {
            return;
        };
        match action {
            Action::Quit => self.should_quit = true,
            Action::StorageModalChar(c) => {
                if !c.is_control() {
                    state.password.push(c);
                }
            }
            Action::StorageModalBackspace => {
                state.password.pop();
            }
            Action::Enter => {
                let password = state.password.clone();
                self.sudo_prompt = None;
                if let Some(respond) = self.sudo_respond.take() {
                    let _ = respond.send(Some(password));
                }
            }
            Action::ToggleConfig => {
                // Esc: cancela a operação privilegiada em curso.
                self.sudo_prompt = None;
                if let Some(respond) = self.sudo_respond.take() {
                    let _ = respond.send(None);
                }
            }
            _ => {}
        }
    }

    /// Ícone de cadeado (trava de segurança) — Nerd Font quando `icons =
    /// true`, ou o token ASCII `[LOCKED]` caso contrário (Zero Emojis
    /// Policy: nenhum emoji é usado em toda a base de código).
    fn lock_tag(&self) -> String {
        if self.config.ui.icons {
            "\u{f023} ".to_string()
        } else {
            "[LOCKED] ".to_string()
        }
    }

    /// Emite o toast de recusa por trava de segurança (disco de sistema),
    /// prefixado pelo ícone/tag de cadeado.
    fn toast_system_locked(&mut self) {
        let msg = format!(
            "{}{}",
            self.lock_tag(),
            self.lang.messages().storage_err_system
        );
        self.toast = Some((Toast::error(msg), Instant::now()));
    }

    pub fn wifi_prompt_open(&self) -> bool {
        self.wifi_prompt.is_some()
    }

    /// `true` quando o teclado está capturado pela sessão PTY da aba ativa
    /// (Arquivos/Terminal) — usado por `InputStream::next` para desviar toda
    /// a digitação, com prioridade análoga a `storage_modal_open`.
    pub fn pty_focused(&self) -> bool {
        self.pty_focused
    }

    /// Recalcula o tamanho de grade (cols, rows) disponível para as sessões
    /// PTY a partir do tamanho `term_w`x`term_h` do terminal e, se mudou
    /// desde a última chamada, devolve as `Action::PtyResize` a difundir para
    /// o backend (uma por sessão). Chamado uma vez por tick em `lib.rs`, não
    /// a partir de `draw()` — a camada `ui` permanece uma função pura de
    /// `&App`.
    pub fn sync_pty_size(&mut self, term_w: u16, term_h: u16) -> Vec<Action> {
        let size = crate::ui::pty_grid_size_for_terminal(term_w, term_h);
        if size == self.last_pty_size {
            return Vec::new();
        }
        self.last_pty_size = size;
        let (cols, rows) = size;
        vec![
            Action::PtyResize {
                target: crate::events::PtyTarget::Files,
                cols,
                rows,
            },
            Action::PtyResize {
                target: crate::events::PtyTarget::Terminal,
                cols,
                rows,
            },
        ]
    }

    fn dispatch_wifi_prompt(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        let Some(state) = &mut self.wifi_prompt else {
            return;
        };
        match action {
            Action::Quit => self.should_quit = true,
            Action::NetworkModalChar(c) => {
                if !c.is_control() {
                    state.password.push(c);
                }
            }
            Action::NetworkModalBackspace => {
                state.password.pop();
            }
            Action::Enter => {
                let ap_id = state.ap_id.clone();
                let ssid = state.ssid.clone();
                let password = state.password.clone();
                self.wifi_prompt = None;
                let _ = action_tx.send(Action::NetworkConnect {
                    ap_id,
                    ssid,
                    password: Some(password),
                });
            }
            Action::ToggleConfig => {
                // Esc: cancela o modal
                self.wifi_prompt = None;
            }
            _ => {}
        }
    }

    /// `true` quando o campo com foco no modal de storage ou wifi é um campo de
    /// texto livre.
    pub fn text_input_active(&self) -> bool {
        if self.sudo_prompt.is_some() || self.wifi_prompt.is_some() {
            return true;
        }
        match &self.storage_modal {
            StorageModal::Format(s) => s.field == FormatField::Label,
            StorageModal::Flasher(s) => matches!(
                s.stage,
                FlasherStage::SelectIso { .. } | FlasherStage::Confirm2 { .. }
            ),
            StorageModal::FilePicker(_) => false,
            StorageModal::MultibootIsoManager(_) => false,
            StorageModal::None => false,
        }
    }

    /// Tecla `f`: abre o modal de formatação para o drive selecionado (na
    /// visão simplificada de um item por drive, formatar sempre opera sobre
    /// o disco inteiro, não numa partição isolada). Recusa discos de sistema
    /// (camada 1 da trava).
    fn storage_format_open(&mut self) {
        let Some((drive, _)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let target_label = drive.friendly_label();
        self.storage_modal = StorageModal::Format(FormatModalState {
            device_id: drive.id.0.clone(),
            target_label,
            fs_idx: 0,
            label: "PENDRIVE".to_string(),
            field: FormatField::Fs,
        });
    }

    /// Tecla `g`/`b`: abre diretamente o seletor de arquivos estilo Yazi para
    /// escolher a ISO do drive selecionado. Recusa discos de sistema (camada
    /// 1 da trava). Ao escolher um arquivo `.iso`/`.img`/`.vhd` (`Enter`), o
    /// seletor reconstrói o modal do Flasher já no estágio de confirmação
    /// (ver `file_picker_enter`).
    fn storage_flasher_open(&mut self) {
        let Some((drive, _)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let target_label = drive.friendly_label();
        self.storage_modal = StorageModal::FilePicker(FilePickerState::open(
            Self::home_dir(),
            FilePickerPurpose::FlasherIso {
                device_id: drive.id.0.clone(),
                target_label,
                target_dev_node: drive.dev_node.clone(),
                target_size: drive.size,
            },
        ));
    }

    /// Tecla `B`: prepara (não-destrutivamente) a partição primária do drive
    /// selecionado para o multi-boot leve embarcado. Recusa discos de
    /// sistema (camada 1 da trava); exige uma partição primária resolvível
    /// (o backend revalida e recusa se ela não estiver formatada FAT32).
    fn storage_multiboot_prepare_open(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((drive, partition)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let Some(partition) = partition else {
            let m = self.lang.messages();
            self.toast = Some((Toast::error(m.storage_multiboot_no_partition), Instant::now()));
            return;
        };
        let _ = action_tx.send(Action::StorageMultibootPrepare {
            device_id: partition.id.0.clone(),
        });
    }

    /// Tecla `G`: abre o gerenciador de ISOs multi-boot (`<mount>/ISOs/`) da
    /// partição primária do drive selecionado.
    fn storage_multiboot_iso_manager_open(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((drive, partition)) = self.storage_selection() else {
            return;
        };
        let Some(partition) = partition else {
            let m = self.lang.messages();
            self.toast = Some((Toast::error(m.storage_multiboot_no_partition), Instant::now()));
            return;
        };
        let target_label = drive.friendly_label();
        let device_id = partition.id.0.clone();
        self.storage_modal = StorageModal::MultibootIsoManager(MultibootIsoManagerState {
            device_id: device_id.clone(),
            target_label,
            stage: MultibootIsoManagerStage::Loading,
        });
        let _ = action_tx.send(Action::StorageMultibootListIsos { device_id });
    }

    /// Roteia uma `Action` para o modal de storage ativo (formatação,
    /// flasher ou gerenciador de ISOs multi-boot), retornando o controle ao
    /// fechar (`Esc`/conclusão).
    fn dispatch_storage_modal(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        let modal = std::mem::take(&mut self.storage_modal);
        self.storage_modal = match modal {
            StorageModal::None => StorageModal::None,
            StorageModal::Format(s) => self.dispatch_format_modal(s, action, action_tx),
            StorageModal::Flasher(s) => self.dispatch_flasher_modal(s, action, action_tx),
            StorageModal::FilePicker(s) => self.dispatch_file_picker_modal(s, action, action_tx),
            StorageModal::MultibootIsoManager(s) => {
                self.dispatch_multiboot_iso_manager_modal(s, action, action_tx)
            }
        };
    }

    /// Diretório inicial do seletor de arquivos: `$HOME`, ou o diretório de
    /// trabalho atual quando `$HOME` não puder ser resolvido.
    fn home_dir() -> PathBuf {
        directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
    }

    /// Atalho de salto `d`/`D`: pasta de downloads do usuário, com fallback
    /// para `$HOME` quando não detectável.
    fn downloads_dir() -> PathBuf {
        directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(Self::home_dir)
    }

    fn dispatch_format_modal(
        &mut self,
        mut s: FormatModalState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        match action {
            Action::Quit => self.should_quit = true,
            // `Esc` é mapeado globalmente para `ToggleConfig`; dentro de um
            // modal de storage, reaproveitamos o sinal para fechar o modal.
            Action::ToggleConfig => return StorageModal::None,
            Action::Up => {
                s.field = match s.field {
                    FormatField::Label => FormatField::Fs,
                    FormatField::Confirm => FormatField::Label,
                    FormatField::Fs => FormatField::Fs,
                };
            }
            Action::Down => {
                s.field = match s.field {
                    FormatField::Fs => FormatField::Label,
                    FormatField::Label => FormatField::Confirm,
                    FormatField::Confirm => FormatField::Confirm,
                };
            }
            // `Tab`/`Shift-Tab` também alternam o foco entre os três campos,
            // ciclando (ao contrário de `↑`/`↓`, que travam nas pontas).
            Action::NextTab => {
                s.field = match s.field {
                    FormatField::Fs => FormatField::Label,
                    FormatField::Label => FormatField::Confirm,
                    FormatField::Confirm => FormatField::Fs,
                };
            }
            Action::PrevTab => {
                s.field = match s.field {
                    FormatField::Fs => FormatField::Confirm,
                    FormatField::Label => FormatField::Fs,
                    FormatField::Confirm => FormatField::Label,
                };
            }
            Action::Left => {
                if s.field == FormatField::Fs {
                    let n = FsChoice::ALL.len();
                    s.fs_idx = (s.fs_idx + n - 1) % n;
                }
            }
            Action::Right => {
                if s.field == FormatField::Fs {
                    s.fs_idx = (s.fs_idx + 1) % FsChoice::ALL.len();
                }
            }
            Action::StorageModalChar(c) => {
                if s.field == FormatField::Label && !c.is_control() && s.label.chars().count() < 32
                {
                    s.label.push(c);
                }
            }
            Action::StorageModalBackspace => {
                if s.field == FormatField::Label {
                    s.label.pop();
                }
            }
            // `Enter` dispara a formatação imediatamente, em qualquer campo
            // com foco (seletor de FS, rótulo ou botão Formatar) — não é
            // mais necessário navegar até o botão de confirmação primeiro.
            Action::Enter => {
                let fs = FsChoice::ALL[s.fs_idx];
                let label = if s.label.trim().is_empty() {
                    "PENDRIVE".to_string()
                } else {
                    s.label.clone()
                };
                let m = self.lang.messages();
                self.toast = Some((
                    Toast::info(format!(
                        "{} {} ({})",
                        m.storage_format_started,
                        s.target_label,
                        fs.label()
                    )),
                    Instant::now(),
                ));
                let _ = action_tx.send(Action::StorageFormat {
                    device_id: s.device_id.clone(),
                    fs_type: fs.udisks_type().to_string(),
                    label,
                });
                return StorageModal::None;
            }
            _ => {}
        }
        StorageModal::Format(s)
    }

    fn dispatch_flasher_modal(
        &mut self,
        mut s: FlasherModalState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return StorageModal::Flasher(s);
        }
        // `Esc`: cancela uma gravação em curso (se houver) e fecha o modal.
        if matches!(action, Action::ToggleConfig) {
            if matches!(s.stage, FlasherStage::Flashing { .. }) {
                let _ = action_tx.send(Action::StorageFlashCancel {
                    device_id: s.device_id.clone(),
                });
            }
            return StorageModal::None;
        }

        let m = self.lang.messages();
        match &mut s.stage {
            FlasherStage::SelectIso { input, error } => match action {
                // Tecla dedicada (F3, ver `events/input.rs`): abre o seletor
                // de arquivos estilo Yazi em vez de digitar o caminho.
                Action::StorageModalOpenPicker => {
                    return StorageModal::FilePicker(FilePickerState::open(
                        Self::home_dir(),
                        FilePickerPurpose::FlasherIso {
                            device_id: s.device_id.clone(),
                            target_label: s.target_label.clone(),
                            target_dev_node: s.target_dev_node.clone(),
                            target_size: s.target_size,
                        },
                    ));
                }
                Action::StorageModalChar(c) if !c.is_control() => input.push(c),
                Action::StorageModalBackspace => {
                    input.pop();
                }
                Action::Enter => match std::fs::metadata(input.trim()) {
                    Ok(meta) if meta.is_file() && meta.len() > 0 => {
                        let size = meta.len();
                        if size > s.target_size {
                            *error = Some(m.storage_flash_err_too_big.to_string());
                        } else {
                            s.iso_path = PathBuf::from(input.trim());
                            s.iso_size = size;
                            s.stage = FlasherStage::Ready { sha256: None };
                        }
                    }
                    Ok(_) => *error = Some(m.storage_flash_err_not_file.to_string()),
                    Err(_) => *error = Some(m.storage_flash_err_not_found.to_string()),
                },
                _ => {}
            },
            // Aguarda `AppEvent::StorageChecksumProgress`/`Done`; sem input direto.
            FlasherStage::Checksumming { .. } => {}
            FlasherStage::Ready { sha256 } => match action {
                Action::StorageModalChar('c') if sha256.is_none() => {
                    let _ = action_tx.send(Action::StorageChecksumIso(
                        s.iso_path.to_string_lossy().to_string(),
                    ));
                    s.stage = FlasherStage::Checksumming { pct: 0.0 };
                }
                Action::Enter => s.stage = FlasherStage::Confirm1,
                _ => {}
            },
            FlasherStage::Confirm1 => {
                if matches!(action, Action::Enter) {
                    s.stage = FlasherStage::Confirm2 {
                        typed: String::new(),
                    };
                }
            }
            FlasherStage::Confirm2 { typed } => match action {
                Action::StorageModalChar(c) if !c.is_control() => typed.push(c),
                Action::StorageModalBackspace => {
                    typed.pop();
                }
                Action::Enter => {
                    if typed.trim() == s.target_dev_node {
                        let _ = action_tx.send(Action::StorageFlashIso {
                            device_id: s.device_id.clone(),
                            iso_path: s.iso_path.to_string_lossy().to_string(),
                        });
                        s.stage = FlasherStage::Flashing {
                            bytes_written: 0,
                            total_bytes: s.iso_size,
                            speed_mbps: 0.0,
                            eta_secs: 0,
                        };
                    } else {
                        self.toast =
                            Some((Toast::error(m.storage_flash_err_mismatch), Instant::now()));
                    }
                }
                _ => {}
            },
            // Progresso chega via `AppEvent::StorageFlashProgress`/`Done`.
            FlasherStage::Flashing { .. } => {}
            FlasherStage::Done { .. } => {
                if matches!(action, Action::Enter) {
                    return StorageModal::None;
                }
            }
        }
        StorageModal::Flasher(s)
    }

    /// Roteia navegação/seleção dentro do seletor de arquivos estilo Yazi.
    /// `h/j/k/l` chegam como `Action::StorageModalChar` (ver generalização em
    /// `events/input.rs`); as setas continuam chegando como `Action::Up/Down/
    /// Left/Right` por usarem `KeyCode` dedicado — ambos são aceitos.
    fn dispatch_file_picker_modal(
        &mut self,
        mut s: FilePickerState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return StorageModal::FilePicker(s);
        }
        // `Esc`: cancela a seleção e fecha o seletor sem escolher nada.
        if matches!(action, Action::ToggleConfig) {
            return StorageModal::None;
        }

        match action {
            Action::Down | Action::StorageModalChar('j') => s.move_down(),
            Action::Up | Action::StorageModalChar('k') => s.move_up(),
            Action::Left | Action::StorageModalChar('h') | Action::StorageModalBackspace => {
                s.go_up()
            }
            Action::Right | Action::StorageModalChar('l') | Action::Enter => {
                return self.file_picker_enter(s, action_tx);
            }
            // Atalhos de salto rápido: `~` casa, `d`/`D` downloads, `M`
            // pasta de mídia removível, `/` raiz do filesystem.
            Action::StorageModalChar('~') => s.jump_to(Self::home_dir()),
            Action::StorageModalChar('d') | Action::StorageModalChar('D') => {
                s.jump_to(Self::downloads_dir())
            }
            Action::StorageModalChar('M') => s.jump_to(PathBuf::from("/media")),
            Action::StorageModalChar('/') => s.jump_to(PathBuf::from("/")),
            _ => {}
        }
        StorageModal::FilePicker(s)
    }

    /// Confirma a seleção do seletor de arquivos: navega para dentro de
    /// diretórios, ou — ao escolher um arquivo de imagem válido — reconstrói
    /// o modal de origem (Flasher ou gerenciador de ISOs multi-boot) já com o
    /// caminho escolhido.
    fn file_picker_enter(
        &mut self,
        mut s: FilePickerState,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        match s.enter_selected() {
            FilePickerOutcome::None => StorageModal::FilePicker(s),
            FilePickerOutcome::Unsupported => {
                let m = self.lang.messages();
                s.error = Some(m.filepicker_err_unsupported.to_string());
                StorageModal::FilePicker(s)
            }
            FilePickerOutcome::Picked(path) => {
                let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                match s.purpose.clone() {
                    FilePickerPurpose::FlasherIso {
                        device_id,
                        target_label,
                        target_dev_node,
                        target_size,
                    } => {
                        if size > target_size {
                            let m = self.lang.messages();
                            StorageModal::Flasher(FlasherModalState {
                                device_id,
                                target_label,
                                target_dev_node,
                                target_size,
                                iso_path: PathBuf::new(),
                                iso_size: 0,
                                stage: FlasherStage::SelectIso {
                                    input: path.to_string_lossy().to_string(),
                                    error: Some(m.storage_flash_err_too_big.to_string()),
                                },
                            })
                        } else {
                            StorageModal::Flasher(FlasherModalState {
                                device_id,
                                target_label,
                                target_dev_node,
                                target_size,
                                iso_path: path,
                                iso_size: size,
                                stage: FlasherStage::Ready { sha256: None },
                            })
                        }
                    }
                    FilePickerPurpose::MultibootAddIso {
                        device_id,
                        target_label,
                    } => {
                        let file_name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let _ = action_tx.send(Action::StorageMultibootAddIso {
                            device_id: device_id.clone(),
                            src_path: path.to_string_lossy().to_string(),
                        });
                        StorageModal::MultibootIsoManager(MultibootIsoManagerState {
                            device_id,
                            target_label,
                            stage: MultibootIsoManagerStage::Copying {
                                bytes_written: 0,
                                total_bytes: size,
                                file_name,
                            },
                        })
                    }
                }
            }
        }
    }

    /// Roteia ações dentro do gerenciador de ISOs multi-boot
    /// (tecla `i`/`I`): listar, adicionar (via seletor de arquivos) e remover
    /// (com confirmação) ISOs na partição de dados.
    fn dispatch_multiboot_iso_manager_modal(
        &mut self,
        mut s: MultibootIsoManagerState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return StorageModal::MultibootIsoManager(s);
        }

        match &mut s.stage {
            MultibootIsoManagerStage::Loading => {
                if matches!(action, Action::ToggleConfig) {
                    return StorageModal::None;
                }
            }
            MultibootIsoManagerStage::Listing {
                entries, selected, ..
            } => match action {
                Action::ToggleConfig => return StorageModal::None,
                Action::Down | Action::StorageModalChar('j') => {
                    if *selected + 1 < entries.len() {
                        *selected += 1;
                    }
                }
                Action::Up | Action::StorageModalChar('k') => {
                    *selected = selected.saturating_sub(1);
                }
                Action::StorageModalChar('a') | Action::StorageModalChar('A') => {
                    return StorageModal::FilePicker(FilePickerState::open(
                        Self::home_dir(),
                        FilePickerPurpose::MultibootAddIso {
                            device_id: s.device_id.clone(),
                            target_label: s.target_label.clone(),
                        },
                    ));
                }
                Action::StorageModalChar('d')
                | Action::StorageModalChar('x')
                | Action::StorageModalDelete => {
                    if let Some(e) = entries.get(*selected) {
                        let file_name = e.name.clone();
                        s.stage = MultibootIsoManagerStage::ConfirmRemove { file_name };
                    }
                }
                _ => {}
            },
            MultibootIsoManagerStage::ConfirmRemove { file_name } => match action {
                Action::Enter | Action::StorageModalChar('y') | Action::StorageModalChar('Y') => {
                    let _ = action_tx.send(Action::StorageMultibootRemoveIso {
                        device_id: s.device_id.clone(),
                        file_name: file_name.clone(),
                    });
                    s.stage = MultibootIsoManagerStage::Removing {
                        file_name: file_name.clone(),
                    };
                }
                Action::ToggleConfig
                | Action::StorageModalChar('n')
                | Action::StorageModalChar('N') => {
                    let _ = action_tx.send(Action::StorageMultibootListIsos {
                        device_id: s.device_id.clone(),
                    });
                    s.stage = MultibootIsoManagerStage::Loading;
                }
                _ => {}
            },
            MultibootIsoManagerStage::Copying { .. } => {
                if matches!(action, Action::ToggleConfig) {
                    return StorageModal::None;
                }
            }
            MultibootIsoManagerStage::Removing { .. } => {
                if matches!(action, Action::ToggleConfig) {
                    return StorageModal::None;
                }
            }
            MultibootIsoManagerStage::Error { .. } => {
                if matches!(action, Action::ToggleConfig | Action::Enter) {
                    return StorageModal::None;
                }
            }
        }
        StorageModal::MultibootIsoManager(s)
    }

    /// Salva a configuração atual em disco e notifica via toast.
    pub fn save_config(&mut self) {
        match self.config.save() {
            Ok(path) => {
                let msg = match self.lang {
                    Language::EnUs => format!("Settings saved to {}", path.display()),
                    Language::EsEs => format!("Configuración guardada en {}", path.display()),
                    Language::PtBr => format!("Configurações salvas em {}", path.display()),
                };
                self.toast = Some((Toast::info(msg), Instant::now()));
            }
            Err(e) => {
                self.toast = Some((Toast::error(format!("Erro ao salvar: {e}")), Instant::now()));
            }
        }
    }

    /// Traduz uma ação de input em mutação de estado e/ou broadcast a backends.
    pub fn dispatch(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        // Durante a splash, qualquer tecla pula para o dashboard.
        if self.phase == Phase::Splash {
            self.phase = Phase::Running;
        }

        // O modal nativo de senha de sudo tem prioridade máxima: captura
        // toda a digitação antes de qualquer outro modal/roteamento, mesmo
        // enquanto um modal de storage (ex.: gravação de ISO, com seu
        // log de progresso) permanece aberto por trás dele.
        if self.sudo_prompt_open() {
            self.dispatch_sudo_prompt(action);
            return;
        }

        // Modal de senha de Wi-Fi: captura digitação antes da navegação comum.
        if self.wifi_prompt_open() {
            self.dispatch_wifi_prompt(action, action_tx);
            return;
        }

        // Se um modal de storage (formatação/flasher) estiver aberto, captura
        // a navegação e o input de texto antes de qualquer outro roteamento.
        if self.storage_modal_open() {
            self.dispatch_storage_modal(action, action_tx);
            return;
        }

        // Se o modal de configurações estiver aberto, captura a navegação e controles.
        if self.show_config {
            match action {
                Action::Quit => self.should_quit = true,
                Action::ToggleConfig => self.show_config = false,
                Action::Up => self.config_prev_field(),
                Action::Down => self.config_next_field(),
                Action::Left => self.config_prev_value(),
                Action::Right | Action::Enter => self.config_next_value(),
                Action::SaveConfig => self.save_config(),
                _ => {}
            }
            return;
        }

        match action {
            Action::Quit => self.should_quit = true,
            Action::NextTab => {
                self.active = Tab::from_index((self.active.index() + 1) % Tab::ALL.len());
                self.pty_focused = false;
            }
            Action::PrevTab => {
                let n = Tab::ALL.len();
                self.active = Tab::from_index((self.active.index() + n - 1) % n);
                self.pty_focused = false;
            }
            Action::SelectTab(i) => {
                self.active = Tab::from_index(i);
                self.pty_focused = false;
            }
            Action::Up => {
                if self.active == Tab::Storage {
                    self.storage_selected = self.storage_selected.saturating_sub(1);
                } else if self.active == Tab::Network {
                    self.network_selected = self.network_selected.saturating_sub(1);
                } else if self.active == Tab::Bluetooth {
                    self.bluetooth_selected = self.bluetooth_selected.saturating_sub(1);
                } else if self.active == Tab::Audio {
                    self.audio_selected = self.audio_selected.saturating_sub(1);
                } else if self.active == Tab::Displays {
                    self.display_selected = self.display_selected.saturating_sub(1);
                } else {
                    let i = self.active.index();
                    self.selection[i] = self.selection[i].saturating_sub(1);
                }
            }
            Action::Down => {
                if self.active == Tab::Storage {
                    self.storage_selected = self.storage_selected.saturating_add(1);
                } else if self.active == Tab::Network {
                    self.network_selected = self.network_selected.saturating_add(1);
                } else if self.active == Tab::Bluetooth {
                    self.bluetooth_selected = self.bluetooth_selected.saturating_add(1);
                } else if self.active == Tab::Audio {
                    self.audio_selected = self.audio_selected.saturating_add(1);
                } else if self.active == Tab::Displays {
                    self.display_selected = self.display_selected.saturating_add(1);
                } else {
                    let i = self.active.index();
                    self.selection[i] = self.selection[i].saturating_add(1);
                }
            }
            Action::Left | Action::Right => {}
            Action::ToggleHelp => {
                self.show_help = !self.show_help;
                if self.show_help {
                    self.show_config = false;
                }
            }
            Action::ToggleConfig => {
                self.show_config = !self.show_config;
                if self.show_config {
                    self.show_help = false;
                }
            }
            Action::SaveConfig => self.save_config(),
            Action::ToggleDetail => self.detailed_overview = !self.detailed_overview,
            Action::Enter => {
                if self.active == Tab::Network {
                    if let Some(net) = &self.network {
                        if let Some(ap) = net.access_points.get(self.network_selected) {
                            if ap.security.needs_password() && !ap.is_saved {
                                self.wifi_prompt = Some(WifiPasswordPromptState {
                                    ap_id: ap.id.0.clone(),
                                    ssid: ap.ssid.clone(),
                                    password: String::new(),
                                    error: None,
                                });
                            } else {
                                let _ = action_tx.send(Action::NetworkConnect {
                                    ap_id: ap.id.0.clone(),
                                    ssid: ap.ssid.clone(),
                                    password: None,
                                });
                            }
                        }
                    }
                } else if self.active == Tab::Bluetooth {
                    if let Some(bt) = &self.bluetooth {
                        if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                            if dev.connected {
                                let _ = action_tx.send(Action::BluetoothDisconnect(dev.id.clone()));
                            } else {
                                let _ = action_tx.send(Action::BluetoothConnect(dev.id.clone()));
                            }
                        }
                    }
                } else if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            if cat == crate::backend::audio::AudioCategory::AppStream {
                                let _ = action_tx.send(Action::AudioToggleMute(node.id));
                            } else {
                                let _ = action_tx.send(Action::AudioSetDefault(node.id));
                            }
                        }
                    }
                } else if self.active == Tab::Displays {
                    if let Some(snap) = &self.displays {
                        if let Some(d) = snap.displays.get(self.display_selected) {
                            let _ = action_tx.send(Action::DisplaySetPrimary(d.name.clone()));
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::NetworkRescan | Action::NetworkToggleRadio | Action::NetworkConnect { .. } => {
                let _ = action_tx.send(action);
            }
            Action::NetworkDisconnect(_) => {
                if let Some(net) = &self.network {
                    if let Some(dev) = &net.wifi_device {
                        let _ = action_tx.send(Action::NetworkDisconnect(dev.id.clone()));
                    }
                }
            }
            Action::NetworkForget(_) => {
                if let Some(net) = &self.network {
                    if let Some(ap) = net.access_points.get(self.network_selected) {
                        if let Some(saved_path) = &ap.saved_conn_path {
                            let _ = action_tx.send(Action::NetworkForget(saved_path.clone()));
                        }
                    }
                }
            }
            Action::NetworkModalChar(_) | Action::NetworkModalBackspace => {}
            Action::BluetoothRescan
            | Action::BluetoothToggleRadio
            | Action::BluetoothConnect(_)
            | Action::BluetoothDisconnect(_) => {
                let _ = action_tx.send(action);
            }
            Action::BluetoothPair(_) => {
                if let Some(bt) = &self.bluetooth {
                    if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                        let _ = action_tx.send(Action::BluetoothPair(dev.id.clone()));
                    }
                }
            }
            Action::BluetoothForget(_) => {
                if let Some(bt) = &self.bluetooth {
                    if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                        let _ = action_tx.send(Action::BluetoothForget(dev.id.clone()));
                    }
                }
            }
            Action::BluetoothToggleBlock(_) => {
                if let Some(bt) = &self.bluetooth {
                    if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                        let _ = action_tx.send(Action::BluetoothToggleBlock(dev.id.clone()));
                    }
                }
            }
            Action::AudioSelectCategory(cat_idx) => {
                if cat_idx == 99 {
                    // Ciclo circular 0 -> 1 -> 2 -> 0
                    self.audio_category = (self.audio_category + 1) % 3;
                } else {
                    self.audio_category = cat_idx.min(2);
                }
                self.audio_selected = 0;
            }
            Action::AudioSetVolume { .. }
            | Action::AudioVolumeUp(_, _)
            | Action::AudioVolumeDown(_, _)
            | Action::AudioToggleMute(_)
            | Action::AudioSetDefault(_) => {
                let _ = action_tx.send(action);
            }
            Action::VolumeUp => {
                if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            let _ = action_tx.send(Action::AudioVolumeUp(node.id, 0.05));
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::VolumeDown => {
                if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            let _ = action_tx.send(Action::AudioVolumeDown(node.id, 0.05));
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::ToggleMute => {
                if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            let _ = action_tx.send(Action::AudioToggleMute(node.id));
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::DisplaySetLayout(_)
            | Action::DisplaySetResolution { .. }
            | Action::DisplaySetPrimary(_) => {
                let _ = action_tx.send(action);
            }
            Action::Refresh
            | Action::BrightnessUp
            | Action::BrightnessDown
            | Action::CyclePowerProfile => {
                let _ = action_tx.send(action);
            }
            Action::Redraw => {}
            Action::StorageMountToggleSelected => self.storage_mount_toggle(action_tx),
            Action::StorageEjectSelected => self.storage_eject_selected(action_tx),
            Action::StorageFormatOpen => self.storage_format_open(),
            Action::StorageFlasherOpen => self.storage_flasher_open(),
            Action::StorageMultibootPrepareOpen => self.storage_multiboot_prepare_open(action_tx),
            Action::StorageMultibootIsoManagerOpen => {
                self.storage_multiboot_iso_manager_open(action_tx)
            }
            Action::StorageMount(_)
            | Action::StorageUnmount(_)
            | Action::StorageEject(_)
            | Action::StorageRefresh
            | Action::StorageFormat { .. }
            | Action::StorageChecksumIso(_)
            | Action::StorageFlashIso { .. }
            | Action::StorageFlashCancel { .. }
            | Action::StorageMultibootPrepare { .. }
            | Action::StorageMultibootListIsos { .. }
            | Action::StorageMultibootAddIso { .. }
            | Action::StorageMultibootRemoveIso { .. } => {
                // Já totalmente formadas (com DeviceId/paths resolvidos);
                // repassa direto ao backend de storage.
                let _ = action_tx.send(action);
            }
            // Sem modal de storage aberto, não há campo de texto/navegação
            // para receber estes atalhos; ignora.
            Action::StorageModalChar(_)
            | Action::StorageModalBackspace
            | Action::StorageModalDelete
            | Action::StorageModalOpenPicker => {}
            Action::Raw(_key) => {
                // Tecla sem mapeamento global e fora do foco de PTY; ignorada.
            }
            Action::PtyInput { .. } | Action::PtyResize { .. } => {
                let _ = action_tx.send(action);
            }
            Action::PtyFocus => {
                let running = match self.active {
                    Tab::Files => matches!(self.files_pty, PtyState::Running(_)),
                    Tab::Terminal => matches!(self.terminal_pty, PtyState::Running(_)),
                    _ => false,
                };
                if running {
                    self.pty_focused = true;
                }
            }
            Action::PtyUnfocus => self.pty_focused = false,
        }
    }
}
