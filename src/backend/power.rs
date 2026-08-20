//! Backend de Energia & Bateria via UPower (D-Bus / `zbus`).
//! Stub do Módulo 0 — implementação no Módulo 5.

use tokio::sync::broadcast;

use crate::events::{Action, EventTx};

pub async fn run(tx: EventTx, _actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    super::pending_stub("power", "Módulo 5 (UPower)", tx).await
}
