//! Backend de Discos & Armazenamento via UDisks2 (D-Bus / `zbus`).
//! Stub do Módulo 0 — implementação no Módulo 4.

use tokio::sync::broadcast;

use crate::events::{Action, EventTx};

pub async fn run(tx: EventTx, _actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    super::pending_stub("storage", "Módulo 4 (UDisks2)", tx).await
}
