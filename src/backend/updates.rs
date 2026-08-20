//! Backend de Atualizações do Sistema (detecção de distro + contagem).
//! Stub do Módulo 0 — implementação no Módulo 6.
//!
//! Já expõe [`Distro::detect`] (usada no roadmap) para detecção da família.

use tokio::sync::broadcast;

use crate::events::{Action, EventTx};

/// Família de distribuição detectada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distro {
    Arch,
    Debian,
    Unknown,
}

impl Distro {
    /// Detecção heurística a partir de `/etc/os-release`.
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

pub async fn run(tx: EventTx, _actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    super::pending_stub("updates", "Módulo 6 (checkupdates/apt)", tx).await
}
