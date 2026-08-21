//! Estado global do Assistente de Sistema e roteamento de [`Action`]/[`AppEvent`].
//!
//! `App` é a única fonte da verdade consumida pelo render. A UI é uma função
//! pura de `&App`.

use std::time::Instant;

use tokio::sync::broadcast;

use crate::backend::storage::{StorageRow, StorageSnapshot};
use crate::backend::system::SystemSnapshot;
use crate::config::Config;
use crate::events::{Action, AppEvent, Toast};

/// Abas do Assistente de Sistema, na ordem da tabbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Network,
    Bluetooth,
    Storage,
    Power,
    Updates,
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
        Tab::Power,
        Tab::Updates,
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
            Tab::Power => m.tab_power,
            Tab::Updates => m.tab_updates,
            Tab::Files => m.tab_files,
            Tab::Terminal => m.tab_terminal,
        }
    }

    /// Título curto para a tabbar (fallback/padrão).
    pub fn title(self) -> &'static str {
        self.title_in(Language::default())
    }
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
    /// Linha selecionada na árvore achatada da aba Storage (ver
    /// `StorageSnapshot::rows`).
    pub storage_selected: usize,

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

    /// Consome um evento de backend, mutando o estado.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::System(snap) => self.system = Some(*snap),
            AppEvent::Storage(snap) => self.storage = Some(*snap),
            AppEvent::Toast(toast) => self.toast = Some((toast, Instant::now())),
            AppEvent::ServiceDegraded { name, reason } => {
                self.services.insert(
                    name,
                    ServiceStatus {
                        degraded: Some(reason),
                    },
                );
            }
        }
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

    /// Linha atualmente selecionada na árvore da aba Storage, já clampeada ao
    /// tamanho da lista (evita índice fora dos limites após um refresh que
    /// encolheu a árvore).
    pub fn storage_row(&self) -> Option<StorageRow> {
        let snap = self.storage.as_ref()?;
        let rows = snap.rows();
        if rows.is_empty() {
            return None;
        }
        let idx = self.storage_selected.min(rows.len() - 1);
        rows.get(idx).copied()
    }

    /// Drive (e partição, se o item selecionado for uma partição) atualmente
    /// realçados na aba Storage.
    pub fn storage_selection(
        &self,
    ) -> Option<(
        &crate::backend::storage::DriveInfo,
        Option<&crate::backend::storage::PartitionInfo>,
    )> {
        let snap = self.storage.as_ref()?;
        match self.storage_row()? {
            StorageRow::Drive(di) => snap.drive(di).map(|d| (d, None)),
            StorageRow::Partition(di, pi) => {
                let drive = snap.drive(di)?;
                let partition = snap.partition(di, pi)?;
                Some((drive, Some(partition)))
            }
        }
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
            self.toast = Some((
                Toast::error("operação bloqueada: disco de sistema"),
                Instant::now(),
            ));
            return;
        }
        let _ = action_tx.send(Action::StorageEject(drive.id.clone()));
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
            }
            Action::PrevTab => {
                let n = Tab::ALL.len();
                self.active = Tab::from_index((self.active.index() + n - 1) % n);
            }
            Action::SelectTab(i) => self.active = Tab::from_index(i),
            Action::Up => {
                if self.active == Tab::Storage {
                    self.storage_selected = self.storage_selected.saturating_sub(1);
                } else {
                    let i = self.active.index();
                    self.selection[i] = self.selection[i].saturating_sub(1);
                }
            }
            Action::Down => {
                if self.active == Tab::Storage {
                    self.storage_selected = self.storage_selected.saturating_add(1);
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
            Action::Enter
            | Action::Refresh
            | Action::BrightnessUp
            | Action::BrightnessDown
            | Action::VolumeUp
            | Action::VolumeDown
            | Action::ToggleMute
            | Action::CyclePowerProfile => {
                // Repassa aos backends; cada worker filtra o que lhe interessa.
                // Brilho/volume são aplicados pela task `system`, que emite um
                // toast e um snapshot atualizado imediatamente.
                let _ = action_tx.send(action);
            }
            Action::Redraw => {}
            Action::StorageMountToggleSelected => self.storage_mount_toggle(action_tx),
            Action::StorageEjectSelected => self.storage_eject_selected(action_tx),
            Action::StorageMount(_)
            | Action::StorageUnmount(_)
            | Action::StorageEject(_)
            | Action::StorageRefresh => {
                // Já totalmente formadas (com DeviceId resolvido); repassa
                // direto ao backend de storage.
                let _ = action_tx.send(action);
            }
            Action::Raw(_key) => {
                // Reservado para foco de terminal (aba 8) — repasse ao PTY.
            }
        }
    }
}
