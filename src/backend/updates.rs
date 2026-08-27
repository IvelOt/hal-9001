use tokio::sync::broadcast;

use crate::events::{Action, EventTx};
use crate::i18n::SharedLang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distro {
    Arch,
    Debian,
    Unknown,
}

impl Distro {
    pub fn detect() -> Distro {
        let Ok(text) = std::fs::read_to_string("/etc/os-release") else {
            return Distro::Unknown;
        };
        let lower = text.to_lowercase();
        if lower.contains("arch") || lower.contains("id_like=arch") {
            Distro::Arch
        } else if lower.contains("debian") || lower.contains("ubuntu") {
            Distro::Debian
        } else {
            Distro::Unknown
        }
    }
}

pub async fn run(
    lang: SharedLang,
    tx: EventTx,
    _actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    super::pending_stub("updates", &lang, lang.messages().pending_module_updates, tx).await
}
