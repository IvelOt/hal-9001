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
use crate::events::{Action, AppEvent, SudoPasswordRequest};

pub async fn run(mut terminal: DefaultTerminal, config: Config) -> Result<()> {
    let (event_tx, mut event_rx) = mpsc::unbounded_channel::<AppEvent>();
    let (action_tx, _action_rx) = broadcast::channel::<Action>(64);
    let (sudo_tx, mut sudo_rx) = mpsc::unbounded_channel::<SudoPasswordRequest>();

    backend::spawn_all(&config, event_tx.clone(), &action_tx, sudo_tx);

    let mut app = App::new(config);
    let mut input = InputStream::new();
    let mut render_tick = tokio::time::interval(Duration::from_millis(250));
    render_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    terminal.draw(|f| ui::draw(&app, f))?;

    loop {
        tokio::select! {
            Some(ev) = event_rx.recv() => {
                for follow_up in app.handle_event(ev) {
                    let _ = action_tx.send(follow_up);
                }
                terminal.draw(|f| ui::draw(&app, f))?;
            }
            Some(action) = input.next(
                app.active,
                app.storage_modal_open(),
                app.text_input_active(),
                app.sudo_prompt_open(),
                app.storage_analyzer_open(),
            ) => {
                app.dispatch(action, &action_tx);
                terminal.draw(|f| ui::draw(&app, f))?;
            }
            Some(req) = sudo_rx.recv() => {
                app.open_sudo_prompt(req);
                terminal.draw(|f| ui::draw(&app, f))?;
            }
            _ = render_tick.tick(), if app.needs_continuous_tick() => {
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
