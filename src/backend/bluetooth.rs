//! Backend de Bluetooth via bluez (D-Bus / `zbus`).
//! Stub do Módulo 0 — implementação no Módulo 3.

use tokio::sync::broadcast;

use crate::events::{Action, EventTx};

pub async fn run(tx: EventTx, _actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    super::pending_stub("bluetooth", "Módulo 3 (bluez)", tx).await
}
