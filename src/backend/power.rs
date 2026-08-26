use tokio::sync::broadcast;

use crate::events::{Action, EventTx};

pub async fn run(tx: EventTx, _actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    super::pending_stub("power", "Módulo 5 (UPower)", tx).await
}
