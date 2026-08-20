//! Estado global do cockpit e roteamento de [`Action`]/[`AppEvent`].
//!
//! `App` é a única fonte da verdade consumida pelo render. A UI é uma função
//! pura de `&App`.

use std::time::Instant;

use tokio::sync::broadcast;

use crate::backend::system::SystemSnapshot;
use crate::config::Config;
use crate::events::{Action, AppEvent, Toast};

/// Abas do cockpit, na ordem da tabbar.
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

    /// Título curto para a tabbar (sem ícone).
    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Network => "Wi-Fi",
            Tab::Bluetooth => "Bluetooth",
            Tab::Storage => "Discos",
            Tab::Power => "Energia",
            Tab::Updates => "Updates",
            Tab::Files => "Arquivos",
            Tab::Terminal => "Terminal",
        }
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
    pub should_quit: bool,
    pub phase: Phase,
    pub active: Tab,
    pub show_help: bool,

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
        Self {
            config,
            should_quit: false,
            phase,
            active: Tab::Overview,
            show_help: false,
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
            AppEvent::System(snap) => self.system = Some(snap),
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
            Action::Enter | Action::Refresh => {
                // Repassa aos backends; cada worker filtra o que lhe interessa.
                let _ = action_tx.send(action);
            }
            Action::Redraw => {}
            Action::Raw(_key) => {
                // Reservado para foco de terminal (aba 8) — repasse ao PTY.
            }
        }
    }
}
