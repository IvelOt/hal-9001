//! Camada de backends. Cada subsistema roda em sua própria task Tokio,
//! publica [`AppEvent`]s e reage a [`Action`]s.
//!
//! No Módulo 0 apenas `system` produz dados reais (via `sysinfo`); os demais
//! registram-se como *degradados/pendentes* para exercitar o fluxo de eventos
//! e a degradação graciosa da UI. Os módulos 2..8 preenchem esses stubs.

pub mod bluetooth;
pub mod multiboot;
pub mod network;
pub mod power;
pub mod pty;
pub mod storage;
pub mod system;
pub mod updates;

use tokio::sync::broadcast;

use crate::config::Config;
use crate::events::{Action, EventTx, SudoPasswordTx};

/// Sobe todas as tasks de backend.
pub fn spawn_all(
    config: &Config,
    tx: EventTx,
    action_tx: &broadcast::Sender<Action>,
    sudo_tx: SudoPasswordTx,
) {
    // Serviço com dados reais.
    tokio::spawn(system::run(
        config.polling.system_ms,
        tx.clone(),
        action_tx.subscribe(),
    ));

    // Serviço com dados reais (Módulo 4).
    tokio::spawn(storage::run(
        config.polling.storage_ms,
        tx.clone(),
        action_tx.subscribe(),
        sudo_tx,
    ));

    // Serviço com dados reais (Módulo 2).
    tokio::spawn(network::run(
        config.polling.network_ms,
        tx.clone(),
        action_tx.subscribe(),
    ));

    // Stubs — sobem e registram estado "pendente" (Módulos 3, 5).
    tokio::spawn(bluetooth::run(tx.clone(), action_tx.subscribe()));
    tokio::spawn(power::run(tx.clone(), action_tx.subscribe()));
    tokio::spawn(updates::run(tx.clone(), action_tx.subscribe()));
    tokio::spawn(pty::run(tx, action_tx.subscribe()));
}

/// Loop utilitário para um backend ainda não implementado: registra-se como
/// pendente e permanece ocioso (reservando o nome do serviço na UI).
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
