//! Backend de sistema (sysinfo) → [`SystemSnapshot`] para o Overview.

use std::time::Duration;

use sysinfo::System;
use tokio::sync::broadcast;

use crate::events::{Action, AppEvent, EventTx};

/// Snapshot enxuto e pronto-para-render do estado do sistema.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub host: String,
    pub user: String,
    pub shell: String,
    pub os: String,
    pub kernel: String,
    pub uptime_secs: u64,
    pub cpu_name: String,
    /// Uso global de CPU em 0.0..=100.0.
    pub cpu_usage: f32,
    pub mem_used: u64,
    pub mem_total: u64,
}

impl SystemSnapshot {
    /// Coleta a partir de um `System` já refreshado.
    fn collect(sys: &System) -> Self {
        let cpu_name = sys
            .cpus()
            .first()
            .map(|c| c.brand().trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "CPU desconhecida".to_string());

        SystemSnapshot {
            host: System::host_name().unwrap_or_else(|| "localhost".into()),
            user: std::env::var("USER").unwrap_or_else(|_| "user".into()),
            shell: std::env::var("SHELL")
                .ok()
                .and_then(|s| s.rsplit('/').next().map(str::to_string))
                .unwrap_or_else(|| "sh".into()),
            os: System::long_os_version()
                .or_else(System::name)
                .unwrap_or_else(|| "Linux".into()),
            kernel: System::kernel_version().unwrap_or_else(|| "?".into()),
            uptime_secs: System::uptime(),
            cpu_name,
            cpu_usage: sys.global_cpu_usage(),
            mem_used: sys.used_memory(),
            mem_total: sys.total_memory(),
        }
    }

    /// Fração de memória usada em 0.0..=1.0.
    pub fn mem_ratio(&self) -> f64 {
        if self.mem_total == 0 {
            0.0
        } else {
            (self.mem_used as f64 / self.mem_total as f64).clamp(0.0, 1.0)
        }
    }

    /// Fração de CPU usada em 0.0..=1.0.
    pub fn cpu_ratio(&self) -> f64 {
        (self.cpu_usage as f64 / 100.0).clamp(0.0, 1.0)
    }
}

/// Task de polling do sistema.
pub async fn run(
    poll_ms: u64,
    tx: EventTx,
    mut actions: broadcast::Receiver<Action>,
) -> anyhow::Result<()> {
    let mut sys = System::new_all();
    let mut ticker = tokio::time::interval(Duration::from_millis(poll_ms.max(250)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Primeiro refresh estabelece a baseline de CPU; o valor de uso só é
    // significativo a partir do segundo tick.
    loop {
        ticker.tick().await;

        // Drena ações pendentes (ex.: Refresh) sem bloquear.
        while actions.try_recv().is_ok() {}

        sys.refresh_cpu_usage();
        sys.refresh_memory();

        if tx.send(AppEvent::System(SystemSnapshot::collect(&sys))).is_err() {
            // App encerrou: nada a fazer.
            break;
        }
    }
    Ok(())
}
