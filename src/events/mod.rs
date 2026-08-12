//! Agregador de eventos assíncronos da TUI.
//!
//! Unifica, através de um canal Tokio `mpsc`, as três fontes de eventos do
//! `hall-9001` para alimentar o loop principal de renderização do Ratatui:
//!
//! * **Teclado** — fluxo de eventos Crossterm (`crossterm::event::EventStream`).
//! * **Telemetria** — amostragem periódica dos backends D-Bus (Power, Storage,
//!   Bluetooth, Network) mais leituras de `/proc` (CPU/memória/uptime).
//! * **IPC / Gatekeeper** — notificações do servidor IPC, incluindo mudanças na
//!   fila de consentimentos que acendem o modal na TUI.
//!
//! Conforme seção 3 (`src/events/`) de `docs/backend_architecture.md`.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{Event as CrosstermEvent, KeyEvent};
use futures_util::StreamExt;
use tokio::sync::mpsc;

use crate::ai_agent::ipc_server::Gatekeeper;
use crate::backend::bluetooth::BluetoothDevice;
use crate::backend::network::{AccessPointInfo, Network, WifiInfo};
use crate::backend::power::BatteryInfo;
use crate::backend::{bluetooth::Bluetooth, controls::Controls, power::Power, storage::Storage};

/// Intervalo entre amostras de telemetria dos backends.
pub const TELEMETRY_INTERVAL: Duration = Duration::from_secs(3);

/// Evento unificado que o loop principal consome.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Tecla pressionada no terminal.
    Key(KeyEvent),
    /// Redimensionamento do terminal (colunas, linhas).
    Resize(u16, u16),
    /// Nova amostra de telemetria do sistema.
    Snapshot(Arc<SystemSnapshot>),
    /// A fila de consentimentos do gatekeeper mudou (modal deve reavaliar).
    ConsentChanged,
    /// Notificação assíncrona (resultado de uma ação disparada pela TUI).
    Notice(String),
    /// Solicita a coleta imediata de uma nova amostra de telemetria.
    Refresh,
}

/// Conjunto de backends de sistema, cada um opcionalmente ausente quando o
/// respectivo serviço D-Bus está indisponível.
pub struct Backends {
    pub power: Option<Power>,
    pub storage: Option<Storage>,
    pub bluetooth: Option<Bluetooth>,
    pub network: Option<Network>,
    pub controls: Option<Controls>,
}

impl Backends {
    /// Instancia todos os backends, tolerando falhas individuais de D-Bus/CLI.
    pub async fn init() -> Self {
        Self {
            power: Power::new().await.ok(),
            storage: Storage::new().await.ok(),
            bluetooth: Bluetooth::new().await.ok(),
            network: Network::new().await.ok(),
            controls: Controls::new().await.ok(),
        }
    }
}

/// Visão de um dispositivo de bloco com estado de montagem (para a aba Discos).
#[derive(Debug, Clone)]
pub struct StorageDeviceView {
    pub object_path: String,
    pub device: String,
    pub label: String,
    pub size: u64,
    pub mounted: bool,
}

/// Estado agregado da rede sem fio.
#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    pub wireless_enabled: Option<bool>,
    pub active: Option<WifiInfo>,
    pub access_points: Vec<AccessPointInfo>,
}

impl Default for NetworkSnapshot {
    fn default() -> Self {
        Self {
            wireless_enabled: None,
            active: None,
            access_points: Vec::new(),
        }
    }
}

/// Estado agregado do Bluetooth.
#[derive(Debug, Clone)]
pub struct BluetoothSnapshot {
    pub adapters: Vec<String>,
    pub devices: Vec<BluetoothDevice>,
}

impl Default for BluetoothSnapshot {
    fn default() -> Self {
        Self {
            adapters: Vec::new(),
            devices: Vec::new(),
        }
    }
}

/// Resumo básico do sistema lido de `/proc`.
#[derive(Debug, Clone, Default)]
pub struct SystemStats {
    /// Carga média de 1 minuto (`/proc/loadavg`).
    pub load1: f64,
    /// Memória total em KiB (`/proc/meminfo`).
    pub mem_total_kb: u64,
    /// Memória disponível em KiB (`/proc/meminfo`).
    pub mem_available_kb: u64,
    /// Tempo de atividade do sistema em segundos (`/proc/uptime`).
    pub uptime_secs: u64,
}

impl SystemStats {
    pub fn mem_used_kb(&self) -> u64 {
        self.mem_total_kb.saturating_sub(self.mem_available_kb)
    }
}

/// Amostra completa de telemetria apresentada nas abas do dashboard.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub on_battery: Option<bool>,
    pub primary_battery: Option<BatteryInfo>,
    pub storage: Vec<StorageDeviceView>,
    pub network: NetworkSnapshot,
    pub bluetooth: BluetoothSnapshot,
    pub system: SystemStats,
    /// Volume do sink padrão (0.0 a 1.0), via `wpctl`.
    pub volume: Option<f64>,
    /// Brilho em percentual (0 a 100), via `brightnessctl`.
    pub brightness: Option<u8>,
}

impl SystemSnapshot {
    fn default_snapshot() -> Self {
        Self {
            on_battery: None,
            primary_battery: None,
            storage: Vec::new(),
            network: NetworkSnapshot::default(),
            bluetooth: BluetoothSnapshot::default(),
            system: SystemStats::default(),
            volume: None,
            brightness: None,
        }
    }
}

impl Default for SystemSnapshot {
    fn default() -> Self {
        Self::default_snapshot()
    }
}

/// Coleta uma amostra completa de telemetria consultando todos os backends.
///
/// Falhas individuais são toleradas: cada subsistema degrada para o estado
/// vazio/`None`, nunca propagando erro para o loop da TUI.
pub async fn collect_snapshot(backends: &Backends) -> SystemSnapshot {
    let mut snapshot = SystemSnapshot::default_snapshot();

    if let Some(power) = &backends.power {
        snapshot.on_battery = power.on_battery().await.ok();
        snapshot.primary_battery = power.primary_battery().await.ok().flatten();
    }

    if let Some(storage) = &backends.storage {
        if let Ok(devices) = storage.block_devices().await {
            for device in devices {
                let mounted = storage.is_mounted(&device.object_path).await.unwrap_or(false);
                snapshot.storage.push(StorageDeviceView {
                    object_path: device.object_path,
                    device: device.device,
                    label: device.label,
                    size: device.size,
                    mounted,
                });
            }
        }
    }

    if let Some(network) = &backends.network {
        snapshot.network.wireless_enabled = network.wireless_enabled().await.ok();
        snapshot.network.active = network.active_wifi().await.ok().flatten();
        snapshot.network.access_points = network.access_points().await.unwrap_or_default();
    }

    if let Some(bluetooth) = &backends.bluetooth {
        snapshot.bluetooth.adapters = bluetooth.adapter_paths().await.unwrap_or_default();
        snapshot.bluetooth.devices = bluetooth.devices().await.unwrap_or_default();
    }

    if let Some(controls) = &backends.controls {
        snapshot.volume = controls.get_volume().await.ok();
        snapshot.brightness = controls.get_brightness_percent().await.ok();
    }

    snapshot.system = read_system_stats();
    snapshot
}

/// Loop principal de eventos: unifica teclado, telemetria e gatekeeper num
/// canal único que o `main` consome com `next()`.
pub struct EventLoop {
    tx: mpsc::Sender<AppEvent>,
    rx: mpsc::Receiver<AppEvent>,
}

impl EventLoop {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(128);
        Self { tx, rx }
    }

    /// Clone do remetente para disparar eventos assíncronos (ações da TUI).
    pub fn sender(&self) -> mpsc::Sender<AppEvent> {
        self.tx.clone()
    }

    /// Inicia as tarefas de fundo (teclado + telemetria + gatekeeper).
    ///
    /// `gatekeeper` opcional: quando presente, monitora a fila de consentimentos
    /// e emite `AppEvent::ConsentChanged` sempre que ela muda.
    pub fn spawn(&self, backends: Arc<Backends>, gatekeeper: Option<Gatekeeper>) {
        let tx = self.tx.clone();
        tokio::spawn(async move {
            if let Err(e) = keyboard_loop(tx.clone()).await {
                eprintln!("[events] loop de teclado encerrado: {e}");
            }
        });

        let tx = self.tx.clone();
        tokio::spawn(async move {
            telemetry_loop(tx.clone(), backends, gatekeeper).await;
        });
    }

    /// Aguarda o próximo evento unificado.
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.rx.recv().await
    }
}

impl Default for EventLoop {
    fn default() -> Self {
        Self::new()
    }
}

/// Encaminha eventos de teclado/redimensionamento do Crossterm para o canal.
async fn keyboard_loop(tx: mpsc::Sender<AppEvent>) -> Result<()> {
    let mut stream = crossterm::event::EventStream::new();
    while let Some(event) = stream.next().await {
        match event {
            Ok(CrosstermEvent::Key(key)) => {
                if tx.send(AppEvent::Key(key)).await.is_err() {
                    break;
                }
            }
            Ok(CrosstermEvent::Resize(cols, rows)) => {
                if tx.send(AppEvent::Resize(cols, rows)).await.is_err() {
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Amostra telemetria periodicamente e monitora a fila de consentimentos.
async fn telemetry_loop(
    tx: mpsc::Sender<AppEvent>,
    backends: Arc<Backends>,
    gatekeeper: Option<Gatekeeper>,
) {
    let mut last_pending = gatekeeper.as_ref().map(|gk| gk.pending().len()).unwrap_or(0);

    loop {
        // Coleta imediata para a primeira amostra e a cada `TELEMETRY_INTERVAL`.
        let snapshot = collect_snapshot(&backends).await;
        if tx.send(AppEvent::Snapshot(Arc::new(snapshot))).await.is_err() {
            return;
        }

        if let Some(gatekeeper) = &gatekeeper {
            let pending = gatekeeper.pending().len();
            if pending != last_pending {
                last_pending = pending;
                if tx.send(AppEvent::ConsentChanged).await.is_err() {
                    return;
                }
            }
        }

        tokio::time::sleep(TELEMETRY_INTERVAL).await;
    }
}

/// Lê estatísticas básicas do sistema a partir de `/proc`.
fn read_system_stats() -> SystemStats {
    let load1 = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|content| content.split_whitespace().next().map(str::to_string))
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(0.0);

    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mut mem_total_kb = 0u64;
    let mut mem_available_kb = 0u64;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            mem_total_kb = parse_kib(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            mem_available_kb = parse_kib(rest);
        }
    }

    let uptime_secs = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|content| content.split_whitespace().next().map(str::to_string))
        .and_then(|value| value.parse::<f64>().ok())
        .map(|seconds| seconds as u64)
        .unwrap_or(0);

    SystemStats {
        load1,
        mem_total_kb,
        mem_available_kb,
        uptime_secs,
    }
}

/// Extrai o valor numérico (em KiB) de uma linha do `/proc/meminfo`.
fn parse_kib(line: &str) -> u64 {
    line.split_whitespace()
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_meminfo_value() {
        assert_eq!(parse_kib(" 16.3 GB"), 0);
        assert_eq!(parse_kib(" 16263976 kB"), 16263976);
        assert_eq!(parse_kib(""), 0);
        assert_eq!(parse_kib("  n/a"), 0);
    }

    #[test]
    fn memory_usage_never_underflows() {
        let stats = SystemStats {
            load1: 0.5,
            mem_total_kb: 100,
            mem_available_kb: 0,
            uptime_secs: 60,
        };
        assert_eq!(stats.mem_used_kb(), 100);

        let stats = SystemStats {
            mem_total_kb: 10,
            mem_available_kb: 20,
            ..Default::default()
        };
        assert_eq!(stats.mem_used_kb(), 0);
    }

    #[test]
    fn snapshot_defaults_are_empty() {
        let snapshot = SystemSnapshot::default();
        assert!(snapshot.storage.is_empty());
        assert!(snapshot.network.access_points.is_empty());
        assert!(snapshot.bluetooth.devices.is_empty());
        assert!(snapshot.primary_battery.is_none());
        assert!(snapshot.volume.is_none());
        assert!(snapshot.brightness.is_none());
    }
}
