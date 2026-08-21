//! HAL-9001 — biblioteca central da TUI de controle de sistema.
//!
//! Exposto como `lib` para permitir testes de integração e reuso do loop
//! principal fora do binário.

pub mod app;
pub mod ascii;
pub mod backend;
pub mod config;
pub mod events;
pub mod i18n;
pub mod logging;
pub mod ui;

use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use tokio::sync::{broadcast, mpsc};

use crate::app::App;
use crate::config::Config;
use crate::events::input::InputStream;
use crate::events::{Action, AppEvent, SuspendTerminalRequest};

/// Executa o loop principal do Assistente de Sistema até o usuário sair.
///
/// - `event_rx`: dados vindos dos backends (`AppEvent`).
/// - `action_tx`: comandos difundidos para os backends (`Action`).
/// - Render é *tick-driven*, desacoplado da chegada de dados.
pub async fn run(mut terminal: DefaultTerminal, config: Config) -> Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (action_tx, _action_rx) = broadcast::channel::<Action>(64);
    let (term_tx, mut term_rx) = mpsc::unbounded_channel::<SuspendTerminalRequest>();

    // Sobe uma task Tokio por serviço de backend.
    backend::spawn_all(&config, event_tx.clone(), &action_tx, term_tx);

    let mut app = App::new(config);
    let mut input = InputStream::new();
    let mut render_tick = tokio::time::interval(Duration::from_millis(app.config.ui.frame_ms));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(ev) = event_rx.recv() => {
                // Ações de acompanhamento (ex.: relistar ISOs do Ventoy após
                // uma cópia/remoção concluída) devolvidas pelo `App` são
                // repassadas aos backends como se tivessem vindo do input.
                for follow_up in app.handle_event(ev) {
                    let _ = action_tx.send(follow_up);
                }
            }
            Some(action) = input.next(app.active, app.storage_modal_open(), app.text_input_active()) => app.dispatch(action, &action_tx),
            Some(req) = term_rx.recv() => {
                // Suspende a TUI (sai do raw mode/alt-screen) para que um
                // prompt interativo de `pkexec`/`sudo` possa ser exibido sem
                // corromper a grade do Ratatui, aguarda o backend sinalizar
                // que o comando elevado terminou, e reinicializa o terminal.
                ratatui::restore();
                let _ = req.ack.send(());
                let _ = req.restore.await;
                terminal = ratatui::init();
            }
            _ = render_tick.tick() => {
                app.on_tick();
                terminal.draw(|f| ui::draw(&app, f))?;
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}
