//! HAL-9001 — Central TUI de controle de sistema.
//!
//! Inicializa o terminal Crossterm (raw mode + alternate screen), instancia a
//! camada backend (Power, Storage, Bluetooth, Network), o servidor IPC, a
//! sessão PTY do AI Terminal Deck e o agregador de eventos — rodando o loop
//! principal de renderização Ratatui multi-abas.

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use hall_9001::ai_agent::ipc_server::{Gatekeeper, IpcServer};
use hall_9001::ai_agent::widget::AiDeckState;
use hall_9001::events::{collect_snapshot, AppEvent, Backends, EventLoop};
use hall_9001::ui::dashboard::Tab;
use hall_9001::ui::toast::Toast;
use hall_9001::ui::Dashboard;
use ratatui::backend::CrosstermBackend;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::mpsc;

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
    // 5. Agregador de eventos + dashboard
    // ------------------------------------------------------------------
    let events = EventLoop::new();
    events.spawn(backends.clone(), gatekeeper.clone());

    let mut dashboard = Dashboard::new();
    dashboard.gatekeeper = gatekeeper.clone();
    // AI Terminal Deck desativado na interface principal (substituído pela aba
    // de Arquivos). O servidor IPC/Gatekeeper segue ativo para o agente.
    dashboard.deck = AiDeckState {
        session: None,
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
///
/// Sai normalmente ao receber `SIGINT`/`SIGTERM`, permitindo que o encerramento
/// limpo (restauração do terminal e remoção do socket UNIX via `Drop`) aconteça.
async fn run_loop(
    terminal: &mut ratatui::Terminal<CrosstermBackend<std::io::Stdout>>,
    dashboard: &mut Dashboard,
    mut events: EventLoop,
    backends: Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
    _gatekeeper: Option<Gatekeeper>,
) -> Result<()> {
    let mut sigterm = signal(SignalKind::terminate())
        .context("falha ao registrar handler de SIGTERM")?;
    let mut sigint =
        signal(SignalKind::interrupt()).context("falha ao registrar handler de SIGINT")?;

    loop {
        let event = tokio::select! {
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
            event = events.next() => event,
        };
        if let Some(event) = event {
            match event {
                AppEvent::Key(key) => {
                    if handle_key(terminal, dashboard, &key, &backends, tx).await {
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
                    dashboard.message = Some(message.clone());
                    dashboard.push_toast(Toast::info(message));
                }
                AppEvent::Toast(toast) => {
                    dashboard.message = Some(toast.message.clone());
                    dashboard.push_toast(toast);
                }
                AppEvent::Refresh => {
                    let snapshot = Arc::new(collect_snapshot(&backends).await);
                    dashboard.snapshot = Some(snapshot);
                }
            }
        }
        // Remove toasts vencidos e redesenha.
        dashboard.prune_toasts();
        dashboard.draw(terminal)?;
    }
    Ok(())
}

/// Processa uma tecla pressionada. Retorna `true` quando o aplicativo deve sair.
///
/// O `terminal` é necessário para a aba de Arquivos: `[Enter]`/`[f]` suspendem
/// o raw mode do Ratatui, executam o Yazi nativamente e restauram a TUI ao
/// sair do subprocesso.
async fn handle_key(
    terminal: &mut ratatui::Terminal<CrosstermBackend<std::io::Stdout>>,
    dashboard: &mut Dashboard,
    key: &KeyEvent,
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

    let current = dashboard.tab;

    // Aba de Arquivos — integra o Yazi File Manager nativamente.
    if current == Tab::Files {
        match key.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => return true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
            KeyCode::Char('1') => return select_tab(dashboard, Tab::Home),
            KeyCode::Char('2') => return select_tab(dashboard, Tab::Storage),
            KeyCode::Char('3') => return select_tab(dashboard, Tab::Network),
            KeyCode::Char('4') => return select_tab(dashboard, Tab::Bluetooth),
            KeyCode::Char('5') => return select_tab(dashboard, Tab::Files),
            KeyCode::Char('f') | KeyCode::Char('F') | KeyCode::Enter => {
                launch_yazi(terminal, dashboard).await;
                return false;
            }
            _ => return handle_tab_switch(dashboard, *key),
        }
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
        KeyCode::Char('1') => select_tab(dashboard, Tab::Home),
        KeyCode::Char('2') => select_tab(dashboard, Tab::Storage),
        KeyCode::Char('3') => select_tab(dashboard, Tab::Network),
        KeyCode::Char('4') => select_tab(dashboard, Tab::Bluetooth),
        KeyCode::Char('5') => select_tab(dashboard, Tab::Files),
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
        KeyCode::Enter | KeyCode::Char('c') => {
            if dashboard.tab == Tab::Network {
                connect_wifi(dashboard, backends, tx).await;
            }
            false
        }
        KeyCode::Char('d') => {
            if dashboard.tab == Tab::Network {
                disconnect_wifi(dashboard, backends, tx).await;
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

/// Trata troca de aba usando Tab/BackTab (usada quando uma aba consome as
/// setas, ex.: File Manager).
fn handle_tab_switch(dashboard: &mut Dashboard, key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('1') => select_tab(dashboard, Tab::Home),
        KeyCode::Char('2') => select_tab(dashboard, Tab::Storage),
        KeyCode::Char('3') => select_tab(dashboard, Tab::Network),
        KeyCode::Char('4') => select_tab(dashboard, Tab::Bluetooth),
        KeyCode::Char('5') => select_tab(dashboard, Tab::Files),
        KeyCode::Tab | KeyCode::Right => {
            dashboard.next_tab();
            false
        }
        KeyCode::BackTab | KeyCode::Left => {
            dashboard.prev_tab();
            false
        }
        KeyCode::Char('q') | KeyCode::Char('Q') => true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => true,
        _ => false,
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
            use hall_9001::ui::toast::ToastLevel;
            let msg = if approved {
                format!("aprovado: {}", request.method)
            } else {
                format!("negado: {}", request.method)
            };
            dashboard.message = Some(msg.clone());
            let level = if approved { ToastLevel::Success } else { ToastLevel::Warning };
            dashboard.push_toast(Toast::new(msg, level));
        }
        Err(e) => {
            let msg = format!("gatekeeper: {e}");
            dashboard.message = Some(msg.clone());
            dashboard.push_toast(Toast::error(msg));
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
        use hall_9001::ui::toast::ToastLevel;
        let Some(storage) = &backends.storage else {
            let _ = tx.send(AppEvent::Toast(Toast::error("UDisks2 indisponível"))).await;
            let _ = tx.send(AppEvent::Refresh).await;
            return;
        };
        let action = if mounted { "desmontar" } else { "montar" };
        let result: Result<Option<String>> = if mounted {
            storage.unmount(&object_path).await.map(|_| None)
        } else {
            storage.mount(&object_path).await.map(Some)
        };
        let (message, level) = match result {
            Ok(Some(mount_point)) => {
                (
                    format!("{action}: {label} montada em {mount_point}"),
                    ToastLevel::Success,
                )
            }
            Ok(None) => (format!("{action}: {label} com sucesso"), ToastLevel::Success),
            Err(e) => (format!("erro ao {action} {label}: {e}"), ToastLevel::Error),
        };
        let _ = tx.send(AppEvent::Toast(Toast::new(message, level))).await;
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
        use hall_9001::ui::toast::ToastLevel;
        let Some(network) = &backends.network else {
            let _ = tx.send(AppEvent::Toast(Toast::error("NetworkManager indisponível"))).await;
            let _ = tx.send(AppEvent::Refresh).await;
            return;
        };
        let result = network.set_wireless_enabled(!enabled).await;
        let (message, level) = match result {
            Ok(()) => (
                format!("wi-fi {}", if enabled { "desligado" } else { "ligado" }),
                ToastLevel::Success,
            ),
            Err(e) => (format!("erro ao alternar wi-fi: {e}"), ToastLevel::Error),
        };
        let _ = tx.send(AppEvent::Toast(Toast::new(message, level))).await;
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
        use hall_9001::ui::toast::ToastLevel;
        let Some(bluetooth) = &backends.bluetooth else {
            let _ = tx.send(AppEvent::Toast(Toast::error("BlueZ indisponível"))).await;
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
        let (message, level) = match result {
            Ok(()) => (
                format!("bluetooth: {action} {label}"),
                ToastLevel::Success,
            ),
            Err(e) => (format!("erro ao {action} {label}: {e}"), ToastLevel::Error),
        };
        let _ = tx.send(AppEvent::Toast(Toast::new(message, level))).await;
        let _ = tx.send(AppEvent::Refresh).await;
    });
}

/// Conecta ao ponto de acesso Wi-Fi selecionado via NetworkManager.
async fn connect_wifi(
    dashboard: &mut Dashboard,
    backends: &Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
) {
    let Some(ap) = dashboard
        .snapshot
        .as_ref()
        .and_then(|s| s.network.access_points.get(dashboard.network_index))
    else {
        return;
    };
    let ap_path = ap.object_path.clone();
    let ssid = ap.ssid.clone();

    let backends = backends.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        use hall_9001::ui::toast::ToastLevel;
        let Some(network) = &backends.network else {
            let _ = tx.send(AppEvent::Toast(Toast::error("NetworkManager indisponível"))).await;
            let _ = tx.send(AppEvent::Refresh).await;
            return;
        };
        let label = if ssid.is_empty() { "(rede oculta)".to_string() } else { ssid };
        // Status intermediário enquanto a conexão ocorre.
        let _ = tx
            .send(AppEvent::Toast(Toast::new(
                format!("Conectando a {label}…"),
                ToastLevel::Info,
            )))
            .await;
        let result = network.connect_access_point(&ap_path).await;
        let (message, level) = match result {
            Ok(()) => (format!("Conectado com sucesso a {label}"), ToastLevel::Success),
            Err(e) => (format!("erro ao conectar a {label}: {e}"), ToastLevel::Error),
        };
        let _ = tx.send(AppEvent::Toast(Toast::new(message, level))).await;
        let _ = tx.send(AppEvent::Refresh).await;
    });
}

/// Suspende a TUI Ratatui e executa o Yazi File Manager nativamente na aba 5.
///
/// Desativa o raw mode e sai do alternate screen, roda `/usr/sbin/yazi` como
/// subprocesso bloqueante (fora da thread assíncrona) e, ao sair, restaura o
/// modo TUI e redesenha o dashboard.
async fn launch_yazi(
    terminal: &mut ratatui::Terminal<CrosstermBackend<std::io::Stdout>>,
    dashboard: &mut Dashboard,
) {
    // 1. Suspende o raw mode + alternate screen da TUI Ratatui.
    let _ = terminal.show_cursor();
    let _ = disable_raw_mode();
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, LeaveAlternateScreen);
    let _ = stdout.flush();

    // 2. Executa o Yazi como subprocesso bloqueante.
    let status = tokio::task::spawn_blocking(|| {
        std::process::Command::new("/usr/sbin/yazi")
            .status()
            .map(|s| s.success())
    })
    .await;

    match status {
        Ok(Ok(true)) => {
            dashboard.push_toast(Toast::info("Yazi encerrado"));
        }
        Ok(Ok(false)) => {
            dashboard.push_toast(Toast::warning("Yazi saiu com status de erro"));
        }
        Ok(Err(e)) => {
            dashboard.push_toast(Toast::error(format!("falha ao executar Yazi: {e}")));
        }
        Err(e) => {
            dashboard.push_toast(Toast::error(format!("falha ao aguardar Yazi: {e}")));
        }
    }

    // 3. Restaura suavemente o modo TUI do Ratatui.
    let _ = enable_raw_mode();
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, EnterAlternateScreen);
    let _ = stdout.flush();
    let _ = terminal.hide_cursor();
    let _ = terminal.clear();
    let _ = dashboard.draw(terminal);
}
async fn disconnect_wifi(
    dashboard: &mut Dashboard,
    backends: &Arc<Backends>,
    tx: &mpsc::Sender<AppEvent>,
) {
    let active_ssid = dashboard
        .snapshot
        .as_ref()
        .and_then(|s| s.network.active.as_ref())
        .map(|w| w.ssid.clone());

    let backends = backends.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        use hall_9001::ui::toast::ToastLevel;
        let Some(network) = &backends.network else {
            let _ = tx.send(AppEvent::Toast(Toast::error("NetworkManager indisponível"))).await;
            let _ = tx.send(AppEvent::Refresh).await;
            return;
        };
        let result = network.disconnect_wireless().await;
        let (message, level) = match result {
            Ok(()) => {
                let target = active_ssid.unwrap_or_else(|| "rede atual".to_string());
                (format!("wi-fi desconectado de {target}"), ToastLevel::Success)
            }
            Err(e) => (format!("erro ao desconectar wi-fi: {e}"), ToastLevel::Error),
        };
        let _ = tx.send(AppEvent::Toast(Toast::new(message, level))).await;
        let _ = tx.send(AppEvent::Refresh).await;
    });
}
