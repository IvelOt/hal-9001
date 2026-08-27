use tokio::sync::broadcast;

use crate::events::{Action, EventTx};
use crate::i18n::SharedLang;

pub async fn run(
    lang: SharedLang,
    tx: EventTx,
    _actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    super::pending_stub("power", &lang, lang.messages().pending_module_power, tx).await
}
