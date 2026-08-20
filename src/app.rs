//! Estado global do Assistente de Sistema e roteamento de [`Action`]/[`AppEvent`].
//!
//! `App` é a única fonte da verdade consumida pelo render. A UI é uma função
//! pura de `&App`.

use std::time::Instant;

use tokio::sync::broadcast;

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
    /// Overview em modo detalhado (alternado pela tecla `.`).
    pub detailed_overview: bool,

    /// Índice selecionado por aba (para listas navegáveis).
    pub selection: [usize; 8],

    /// Último snapshot de sistema.
    pub system: Option<SystemSnapshot>,

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
            detailed_overview: false,
            selection: [0; 8],
            system: None,
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
        if self.phase == Phase::Splash
            && self.elapsed_ms() as u64 >= self.config.splash.min_ms
        {
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
            AppEvent::Toast(toast) => self.toast = Some((toast, Instant::now())),
            AppEvent::ServiceDegraded { name, reason } => {
                self.services
                    .insert(name, ServiceStatus { degraded: Some(reason) });
            }
        }
    }

    /// Traduz uma ação de input em mutação de estado e/ou broadcast a backends.
    pub fn dispatch(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        // Durante a splash, qualquer tecla pula para o dashboard.
        if self.phase == Phase::Splash {
            self.phase = Phase::Running;
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
                let i = self.active.index();
                self.selection[i] = self.selection[i].saturating_sub(1);
            }
            Action::Down => {
                let i = self.active.index();
                self.selection[i] = self.selection[i].saturating_add(1);
            }
            Action::ToggleHelp => self.show_help = !self.show_help,
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
            Action::Raw(_key) => {
                // Reservado para foco de terminal (aba 8) — repasse ao PTY.
            }
        }
    }
}
