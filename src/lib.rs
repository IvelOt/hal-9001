//! HAL-9001 — biblioteca central da TUI de controle de sistema.
//!
//! Exposto como `lib` para permitir testes de integração e reuso do loop
//! principal fora do binário.

pub mod app;
pub mod ascii;
pub mod backend;
pub mod config;
pub mod events;
pub mod logging;
pub mod ui;

use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use tokio::sync::{broadcast, mpsc};

use crate::app::App;
use crate::config::Config;
use crate::events::input::InputStream;
use crate::events::{Action, AppEvent};

/// Executa o loop principal do Assistente de Sistema até o usuário sair.
///
/// - `event_rx`: dados vindos dos backends (`AppEvent`).
/// - `action_tx`: comandos difundidos para os backends (`Action`).
/// - Render é *tick-driven*, desacoplado da chegada de dados.
pub async fn run(mut terminal: DefaultTerminal, config: Config) -> Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (action_tx, _action_rx) = broadcast::channel::<Action>(64);

    // Sobe uma task Tokio por serviço de backend.
    backend::spawn_all(&config, event_tx.clone(), &action_tx);

    let mut app = App::new(config);
    let mut input = InputStream::new();
    let mut render_tick = tokio::time::interval(Duration::from_millis(app.config.ui.frame_ms));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(ev) = event_rx.recv() => app.handle_event(ev),
            Some(action) = input.next() => app.dispatch(action, &action_tx),
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
