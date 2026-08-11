//! Leitura do estado de energia via daemon **UPower** (D-Bus `org.freedesktop.UPower`).
//!
//! Fornece estado da bateria, percentual e estimativa de tempo restante
//! (carregando / descarregando), conforme seção 1.4 de `docs/backend_architecture.md`.

use std::time::Duration;

use anyhow::{Context, Result};
use serde::Serialize;
use zbus::zvariant::OwnedObjectPath;
use zbus::{proxy, Connection};

/// Estados de energia reportados pela propriedade `State` do UPower.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u32)]
pub enum PowerState {
    /// Estado desconhecido / indisponível.
    Unknown = 0,
    /// Carregando.
    Charging = 1,
    /// Descarregando.
    Discharging = 2,
    /// Vazia.
    Empty = 3,
    /// Totalmente carregada.
    FullyCharged = 4,
    /// Carga pendente.
    PendingCharge = 5,
    /// Descarga pendente.
    PendingDischarge = 6,
}

impl From<u32> for PowerState {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::Charging,
            2 => Self::Discharging,
            3 => Self::Empty,
            4 => Self::FullyCharged,
            5 => Self::PendingCharge,
            6 => Self::PendingDischarge,
            _ => Self::Unknown,
        }
    }
}

/// Tipos de dispositivos de energia reportados pela propriedade `Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[repr(u32)]
pub enum DeviceType {
    LinePower = 1,
    Battery = 2,
    Ups = 3,
    Monitor = 4,
    Mouse = 5,
    Keyboard = 6,
    Pda = 7,
    Phone = 8,
    Unknown = 0,
}

impl From<u32> for DeviceType {
    fn from(value: u32) -> Self {
        match value {
            1 => Self::LinePower,
            2 => Self::Battery,
            3 => Self::Ups,
            4 => Self::Monitor,
            5 => Self::Mouse,
            6 => Self::Keyboard,
            7 => Self::Pda,
            8 => Self::Phone,
            _ => Self::Unknown,
        }
    }
}

/// Informações agregadas de um dispositivo de energia (bateria).
#[derive(Debug, Clone, Serialize)]
pub struct BatteryInfo {
    pub object_path: String,
    pub device_type: DeviceType,
    pub state: PowerState,
    pub percentage: f64,
    pub time_to_empty: Option<Duration>,
    pub time_to_full: Option<Duration>,
    pub capacity: f64,
    pub power_supply: bool,
    pub is_present: bool,
}

impl BatteryInfo {
    /// Retorna `true` se a estimativa de tempo restante faz sentido (descarregando).
    pub fn estimated_time_remaining(&self) -> Option<Duration> {
        match self.state {
            PowerState::Discharging => self.time_to_empty,
            PowerState::Charging => self.time_to_full,
            _ => None,
        }
    }
}

/// Proxy para a interface raiz `org.freedesktop.UPower`.
#[proxy(
    interface = "org.freedesktop.UPower",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower"
)]
trait UPower {
    /// Lista os caminhos de objeto de todos os dispositivos de energia.
    fn enumerate_devices(&self) -> zbus::Result<Vec<OwnedObjectPath>>;

    /// `true` se o sistema está rodando em bateria.
    #[zbus(property)]
    fn on_battery(&self) -> zbus::Result<bool>;
}

/// Proxy para a interface `org.freedesktop.UPower.Device`.
#[proxy(
    interface = "org.freedesktop.UPower.Device",
    default_service = "org.freedesktop.UPower"
)]
trait Device {
    /// Condição de energia (`u32`, ver [`PowerState`]).
    #[zbus(property)]
    fn state(&self) -> zbus::Result<u32>;

    /// Nível de bateria atual (0.0 a 100.0).
    #[zbus(property)]
    fn percentage(&self) -> zbus::Result<f64>;

    /// Segundos restantes para esvaziar a bateria (0 se desconhecido).
    #[zbus(property)]
    fn time_to_empty(&self) -> zbus::Result<i64>;

    /// Segundos restantes para carregar totalmente (0 se desconhecido).
    #[zbus(property)]
    fn time_to_full(&self) -> zbus::Result<i64>;

    /// Saúde da bateria em % da capacidade de projeto.
    #[zbus(property)]
    fn capacity(&self) -> zbus::Result<f64>;

    /// Tipo do dispositivo (`u32`, ver [`DeviceType`]).
    #[zbus(property)]
    fn type_(&self) -> zbus::Result<u32>;

    /// `true` se o dispositivo é a fonte de alimentação (bateria principal).
    #[zbus(property)]
    fn power_supply(&self) -> zbus::Result<bool>;

    /// `true` se o dispositivo está presente no sistema.
    #[zbus(property)]
    fn is_present(&self) -> zbus::Result<bool>;
}

/// Backend de energia que encapsula a conexão D-Bus ao UPower.
pub struct Power {
    connection: Connection,
}

impl Power {
    /// Abre uma conexão com o barramento do sistema e retorna o backend de energia.
    pub async fn new() -> Result<Self> {
        let connection = Connection::system()
            .await
            .context("falha ao conectar ao barramento D-Bus do sistema")?;
        Ok(Self { connection })
    }

    /// Retorna `true` se o sistema está atualmente em bateria.
    pub async fn on_battery(&self) -> Result<bool> {
        let upower = UPowerProxy::new(&self.connection).await?;
        upower
            .on_battery()
            .await
            .context("falha ao ler propriedade OnBattery do UPower")
    }

    /// Enumera todos os dispositivos de energia e agrega suas propriedades.
    pub async fn batteries(&self) -> Result<Vec<BatteryInfo>> {
        let upower = UPowerProxy::new(&self.connection).await?;
        let paths = upower
            .enumerate_devices()
            .await
            .context("falha ao chamar UPower.EnumerateDevices")?;

        let mut batteries = Vec::new();
        for path in paths {
            let device = DeviceProxy::new(&self.connection, path.clone())
                .await
                .with_context(|| format!("falha ao criar proxy para dispositivo {path}"))?;

            let device_type = DeviceType::from(device.type_().await.unwrap_or(0));
            // Foca apenas em baterias reais, ignorando alimentação de linha e periféricos.
            if device_type != DeviceType::Battery {
                continue;
            }

            batteries.push(BatteryInfo {
                object_path: path.to_string(),
                device_type,
                state: PowerState::from(device.state().await.unwrap_or(0)),
                percentage: device.percentage().await.unwrap_or(0.0),
                time_to_empty: to_duration(device.time_to_empty().await.unwrap_or(0)),
                time_to_full: to_duration(device.time_to_full().await.unwrap_or(0)),
                capacity: device.capacity().await.unwrap_or(0.0),
                power_supply: device.power_supply().await.unwrap_or(false),
                is_present: device.is_present().await.unwrap_or(false),
            });
        }

        Ok(batteries)
    }

    /// Retorna a bateria principal (fonte de alimentação), se existir.
    pub async fn primary_battery(&self) -> Result<Option<BatteryInfo>> {
        Ok(self.batteries().await?.into_iter().find(|b| b.power_supply))
    }
}

/// Converte segundos (>= 0) em `Duration`; valores inválidos viram `None`.
fn to_duration(seconds: i64) -> Option<Duration> {
    if seconds <= 0 {
        None
    } else {
        Some(Duration::from_secs(seconds as u64))
    }
}
