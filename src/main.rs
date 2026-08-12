//! HAL-9001 — Central TUI de controle de sistema.
//!
//! Inicializa o terminal Crossterm (raw mode + alternate screen), instancia a
//! camada backend (Power, Storage, Bluetooth, Network), o servidor IPC, a
//! sessão PTY do AI Terminal Deck e o agregador de eventos — rodando o loop
//! principal de renderização Ratatui multi-abas.

mod ai_agent;
mod backend;
mod events;
mod ui;

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use tokio::sync::mpsc;

use crate::ai_agent::ipc_server::{Gatekeeper, IpcServer};
use crate::ai_agent::pty_session::{AgentCommand, AgentKind, PtySession, PtyTarget};
use crate::ai_agent::widget::AiDeckState;
use crate::events::{collect_snapshot, AppEvent, Backends, EventLoop};
use crate::ui::dashboard::Tab;
use crate::ui::Dashboard;

#[tokio::main]
async fn main() -> Result<()> {
    // ------------------------------------------------------------------
    // 1. Terminal em raw mode + alternate screen
    // ------------------------------------------------------------------
    enable_raw_mode().context("falha ao ativar raw mode do terminal")?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen).context("falha ao entrar em alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal =
        ratatui::Terminal::new(backend).context("falha ao criar terminal Ratatui")?;
    terminal.hide_cursor()?;

    // ------------------------------------------------------------------
    // 2. Backends de sistema (D-Bus / CLI), tolerando indisponibilidade
    // ------------------------------------------------------------------
    let backends = Arc::new(Backends::init().await);

    // ------------------------------------------------------------------
    // 3. Servidor IPC JSON-RPC + Gatekeeper de consentimento
    // ------------------------------------------------------------------
    let ipc = match IpcServer::bind_default().await {
        Ok(server) => Some(server),
        Err(e) => {
            eprintln!("[ipc] servidor IPC indisponível: {e}");
            None
        }
    };
    let gatekeeper = ipc.as_ref().map(|server| server.gatekeeper());
    if let Some(gatekeeper) = &gatekeeper {
        gatekeeper.attach_listener();
    }

    // ------------------------------------------------------------------
    // 4. Sessão PTY do AI Terminal Deck
    // ------------------------------------------------------------------
    let session = spawn_pty_session();
    if let (Some(server), Some(session)) = (&ipc, &session) {
        server.attach_pty(session.clone());
    }

    // ------------------------------------------------------------------
    // 5. Agregador de eventos + dashboard
    // ------------------------------------------------------------------
    let events = EventLoop::new();
    events.spawn(backends.clone(), gatekeeper.clone());

    let mut dashboard = Dashboard::new();
    dashboard.gatekeeper = gatekeeper.clone();
    dashboard.deck = AiDeckState {
        session,
        gatekeeper: gatekeeper.clone(),
        ipc_socket: ipc.as_ref().map(|server| server.socket_path().to_path_buf()),
        ipc_listening: ipc.is_some(),
    };

    // ------------------------------------------------------------------
    // 6. Servidor IPC em segundo plano
    // ------------------------------------------------------------------
    if let Some(server) = ipc {
        tokio::spawn(async move {
            if let Err(e) = server.serve().await {
                eprintln!("[ipc] servidor IPC encerrado com erro: {e}");
            }
        });
    }

    // ------------------------------------------------------------------
    // 7. Loop principal de eventos e renderização
    // ------------------------------------------------------------------
    let tx = events.sender();
    let result = run_loop(&mut terminal, &mut dashboard, events, backends, &tx, gatekeeper).await;

    // ------------------------------------------------------------------
    // 8. Restauração do terminal
    // ------------------------------------------------------------------
    terminal.show_cursor()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, LeaveAlternateScreen).context("falha ao sair de alternate screen")?;
    disable_raw_mode().context("falha ao restaurar raw mode do terminal")?;
    stdout.flush()?;

    result
}

/// Loop principal: consome eventos unificados, atualiza o dashboard e redesenha.
async fn run_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<std::io::Stdout>>,
    dashboard: &mut Dashboard,
    mut events: EventLoop,
    backends: Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
    _gatekeeper: Option<Gatekeeper>,
) -> Result<()> {
    loop {
        if let Some(event) = events.next().await {
            match event {
                AppEvent::Key(key) => {
                    if handle_key(dashboard, key, &backends, tx).await {
                        break;
                    }
                }
                AppEvent::Resize(cols, rows) => {
                    dashboard.on_resize(cols, rows);
                }
                AppEvent::Snapshot(snapshot) => {
                    dashboard.snapshot = Some(snapshot);
                }
                AppEvent::ConsentChanged => {}
                AppEvent::Notice(message) => {
                    dashboard.message = Some(message);
                }
                AppEvent::Refresh => {
                    let snapshot = Arc::new(collect_snapshot(&backends).await);
                    dashboard.snapshot = Some(snapshot);
                }
            }
        }
        dashboard.draw(terminal)?;
    }
    Ok(())
}

/// Processa uma tecla pressionada. Retorna `true` quando o aplicativo deve sair.
async fn handle_key(
    dashboard: &mut Dashboard,
    key: KeyEvent,
    backends: &Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
) -> bool {
    // Gatekeeper: com pedidos pendentes, apenas decidir ou sair.
    let pending = dashboard
        .gatekeeper
        .as_ref()
        .map(|gk| gk.pending().len())
        .unwrap_or(0);
    if pending > 0 {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                resolve_consent(dashboard, true);
                false
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                resolve_consent(dashboard, false);
                false
            }
            KeyCode::Char('q') | KeyCode::Char('Q') => true,
            _ => false,
        };
    }

    // AI Terminal Deck: encaminha as teclas ao agente (PTY).
    if dashboard.tab == Tab::AiDeck {
        if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        match key.code {
            KeyCode::Tab => {
                dashboard.next_tab();
                false
            }
            KeyCode::BackTab => {
                dashboard.prev_tab();
                false
            }
            _ => {
                if let Some(session) = &dashboard.deck.session {
                    if let Some(bytes) = key_to_bytes(&key) {
                        let _ = session.write_input(&bytes);
                    }
                }
                false
            }
        }
    } else {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
            KeyCode::Char('1') => select_tab(dashboard, Tab::Overview),
            KeyCode::Char('2') => select_tab(dashboard, Tab::Storage),
            KeyCode::Char('3') => select_tab(dashboard, Tab::Network),
            KeyCode::Char('4') => select_tab(dashboard, Tab::Bluetooth),
            KeyCode::Char('5') => select_tab(dashboard, Tab::AiDeck),
            KeyCode::Tab | KeyCode::Right => {
                dashboard.next_tab();
                false
            }
            KeyCode::BackTab | KeyCode::Left => {
                dashboard.prev_tab();
                false
            }
            KeyCode::Up => {
                dashboard.move_selection(-1);
                false
            }
            KeyCode::Down => {
                dashboard.move_selection(1);
                false
            }
            KeyCode::Char('m') => {
                if dashboard.tab == Tab::Storage {
                    toggle_mount(dashboard, backends, tx).await;
                }
                false
            }
            KeyCode::Char('w') => {
                if dashboard.tab == Tab::Network {
                    toggle_wifi(dashboard, backends, tx).await;
                }
                false
            }
            KeyCode::Char('b') => {
                if dashboard.tab == Tab::Bluetooth {
                    toggle_bluetooth(dashboard, backends, tx).await;
                }
                false
            }
            _ => false,
        }
    }
}

fn select_tab(dashboard: &mut Dashboard, tab: Tab) -> bool {
    dashboard.select_tab(tab);
    false
}

/// Resolve o primeiro pedido de consentimento pendente do gatekeeper.
fn resolve_consent(dashboard: &mut Dashboard, approved: bool) {
    let Some(gatekeeper) = &dashboard.gatekeeper else {
        return;
    };
    let Some(request) = gatekeeper.pending().first().cloned() else {
        return;
    };
    match gatekeeper.resolve(request.id, approved) {
        Ok(()) => {
            dashboard.message = Some(if approved {
                format!("aprovado: {}", request.method)
            } else {
                format!("negado: {}", request.method)
            });
        }
        Err(e) => {
            dashboard.message = Some(format!("gatekeeper: {e}"));
        }
    }
}

/// Monta/desmonta o dispositivo selecionado (UDisks2) em segundo plano.
async fn toggle_mount(
    dashboard: &mut Dashboard,
    backends: &Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
) {
    let Some(snapshot) = &dashboard.snapshot else {
        return;
    };
    let Some(device) = snapshot.storage.get(dashboard.storage_index) else {
        return;
    };
    let object_path = device.object_path.clone();
    let label = device.label.clone();
    let mounted = device.mounted;

    let backends = backends.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let Some(storage) = &backends.storage else {
            let _ = tx.send(AppEvent::Notice("UDisks2 indisponível".to_string())).await;
            let _ = tx.send(AppEvent::Refresh).await;
            return;
        };
        let action = if mounted { "desmontar" } else { "montar" };
        let result: Result<Option<String>> = if mounted {
            storage.unmount(&object_path).await.map(|_| None)
        } else {
            storage.mount(&object_path).await.map(Some)
        };
        let notice = match result {
            Ok(Some(mount_point)) => {
                format!("{action}: {label} montada em {mount_point}")
            }
            Ok(None) => format!("{action}: {label} com sucesso"),
            Err(e) => format!("erro ao {action} {label}: {e}"),
        };
        let _ = tx.send(AppEvent::Notice(notice)).await;
        let _ = tx.send(AppEvent::Refresh).await;
    });
}

/// Liga/desliga o Wi-Fi globalmente (NetworkManager) em segundo plano.
async fn toggle_wifi(
    dashboard: &mut Dashboard,
    backends: &Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
) {
    let enabled = dashboard
        .snapshot
        .as_ref()
        .map(|s| s.network.wireless_enabled.unwrap_or(false))
        .unwrap_or(false);

    let backends = backends.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let Some(network) = &backends.network else {
            let _ = tx.send(AppEvent::Notice("NetworkManager indisponível".to_string())).await;
            let _ = tx.send(AppEvent::Refresh).await;
            return;
        };
        let result = network.set_wireless_enabled(!enabled).await;
        let notice = match result {
            Ok(()) => format!("wi-fi {}", if enabled { "desligado" } else { "ligado" }),
            Err(e) => format!("erro ao alternar wi-fi: {e}"),
        };
        let _ = tx.send(AppEvent::Notice(notice)).await;
        let _ = tx.send(AppEvent::Refresh).await;
    });
}

/// Conecta/desconecta o dispositivo Bluetooth selecionado em segundo plano.
async fn toggle_bluetooth(
    dashboard: &mut Dashboard,
    backends: &Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
) {
    let Some(device) = dashboard
        .snapshot
        .as_ref()
        .and_then(|s| s.bluetooth.devices.get(dashboard.bluetooth_index))
    else {
        return;
    };
    let object_path = device.object_path.clone();
    let name = device.name.clone();
    let connect = !device.connected;

    let backends = backends.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        let Some(bluetooth) = &backends.bluetooth else {
            let _ = tx.send(AppEvent::Notice("BlueZ indisponível".to_string())).await;
            let _ = tx.send(AppEvent::Refresh).await;
            return;
        };
        let action = if connect { "conectar" } else { "desconectar" };
        let result = if connect {
            bluetooth.connect_device(&object_path).await
        } else {
            bluetooth.disconnect_device(&object_path).await
        };
        let label = if name.is_empty() { object_path.clone() } else { name };
        let notice = match result {
            Ok(()) => format!("bluetooth: {action} {label}"),
            Err(e) => format!("erro ao {action} {label}: {e}"),
        };
        let _ = tx.send(AppEvent::Notice(notice)).await;
        let _ = tx.send(AppEvent::Refresh).await;
    });
}

/// Inicia a sessão PTY do AI Terminal Deck (bash por padrão).
fn spawn_pty_session() -> Option<Arc<PtySession>> {
    let mut pty = PtySession::new(AgentCommand::new(AgentKind::Bash, Vec::new()));
    if let Err(e) = pty.start() {
        eprintln!("[deck] falha ao iniciar PTY do AI Terminal Deck: {e}");
        return None;
    }
    Some(Arc::new(pty))
}

/// Converte um `KeyEvent` Crossterm nos bytes ANSI a enviar ao agente.
fn key_to_bytes(key: &KeyEvent) -> Option<Vec<u8>> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = key.code {
            if c.is_ascii_lowercase() {
                return Some(vec![c as u8 - b'a' + 1]);
            }
        }
        return None;
    }
    match key.code {
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Char(c) => {
            let mut buf = [0u8; 4];
            Some(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[1~".to_vec()),
        KeyCode::End => Some(b"\x1b[4~".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        _ => None,
    }
}
