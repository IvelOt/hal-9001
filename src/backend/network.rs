//! Backend de Wi-Fi/Rede via NetworkManager (D-Bus / `zbus`).
//! Stub do Módulo 0 — implementação no Módulo 2 (ver `docs/03_...`).

use tokio::sync::broadcast;

use crate::events::{Action, EventTx};

pub async fn run(tx: EventTx, _actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    super::pending_stub("network", "Módulo 2 (NetworkManager)", tx).await
}
