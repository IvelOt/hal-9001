//! Fundação de PTY (`portable-pty`) para o Terminal Deck (aba 8), o Yazi
//! (aba 7) e o runner de updates (aba 6).
//! Stub do Módulo 0 — implementação nos Módulos 6/7/8.

use tokio::sync::broadcast;

use crate::events::{Action, EventTx};

pub async fn run(tx: EventTx, _actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    super::pending_stub("pty", "Módulos 6/7/8 (Terminal Deck & Yazi)", tx).await
}
