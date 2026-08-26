
pub mod audio;
pub mod bluetooth;
pub mod disk_analyzer;
pub mod display;
pub mod multiboot;
pub mod network;
pub mod power;
pub mod storage;
pub mod system;
pub mod updates;

use tokio::sync::broadcast;

use crate::config::Config;
use crate::events::{Action, EventTx, SudoPasswordTx};

pub fn spawn_all(
    config: &Config,
    tx: EventTx,
    action_tx: &broadcast::Sender<Action>,
    sudo_tx: SudoPasswordTx,
) {

    tokio::spawn(system::run(
        config.polling.system_ms,
        tx.clone(),
        action_tx.subscribe(),
    ));

    tokio::spawn(storage::run(
        config.polling.storage_ms,
        tx.clone(),
        action_tx.subscribe(),
        sudo_tx,
    ));

    tokio::spawn(network::run(
        config.polling.network_ms,
        tx.clone(),
        action_tx.subscribe(),
    ));

    tokio::spawn(bluetooth::run(
        config.polling.bluetooth_ms,
        tx.clone(),
        action_tx.subscribe(),
    ));

    tokio::spawn(audio::run(
        config.polling.audio_ms,
        tx.clone(),
        action_tx.subscribe(),
    ));

    tokio::spawn(display::run(
        config.polling.display_ms,
        tx.clone(),
        action_tx.subscribe(),
    ));
}

pub(crate) async fn pending_stub(
    name: &'static str,
    modulo: &'static str,
    tx: EventTx,
) -> anyhow::Result<()> {
    let _ = tx.send(crate::events::AppEvent::ServiceDegraded {
        name,
        reason: format!("{modulo} — pendente de implementação"),
    });
    std::future::pending::<()>().await;
    Ok(())
}
