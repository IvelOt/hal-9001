
pub mod input;

use std::path::PathBuf;

use crate::backend::audio::AudioSnapshot;
use crate::backend::bluetooth::BluetoothSnapshot;
use crate::backend::display::DisplaySnapshot;
use crate::backend::network::NetworkSnapshot;
use crate::backend::storage::{StorageSnapshot, VentoyIsoEntry};
use crate::backend::system::SystemSnapshot;

pub type EventTx = tokio::sync::mpsc::UnboundedSender<AppEvent>;

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeviceId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub level: ToastLevel,
    pub text: String,
}

impl Toast {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Info,
            text: text.into(),
        }
    }
    pub fn success(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Success,
            text: text.into(),
        }
    }
    pub fn warn(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Warning,
            text: text.into(),
        }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            level: ToastLevel::Error,
            text: text.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppEvent {

    System(Box<SystemSnapshot>),

    Storage(Box<StorageSnapshot>),

    Network(Box<NetworkSnapshot>),

    NetworkScanning(bool),

    Bluetooth(Box<BluetoothSnapshot>),

    BluetoothScanning(bool),

    Audio(Box<AudioSnapshot>),

    Display(Box<DisplaySnapshot>),

    Toast(Toast),

    ServiceDegraded { name: &'static str, reason: String },

    StorageChecksumProgress { path: PathBuf, pct: f32 },

    StorageChecksumDone { path: PathBuf, sha256: String },

    StorageFlashProgress {
        bytes_written: u64,
        total_bytes: u64,
        speed_mbps: f64,
        eta_secs: u64,
    },

    StorageFlashDone {
        device_id: String,
        result: Result<String, String>,
    },

    StorageMultibootIsoList {
        device_id: String,
        entries: Vec<VentoyIsoEntry>,
        free_bytes: Option<u64>,
    },

    StorageMultibootIsoCopyProgress {
        device_id: String,
        bytes_written: u64,
        total_bytes: u64,
    },

    StorageMultibootIsoCopyDone {
        device_id: String,
        result: Result<String, String>,
    },

    StorageMultibootIsoRemoveDone {
        device_id: String,
        result: Result<String, String>,
    },

    StorageAnalyzerSnapshot(Box<crate::backend::disk_analyzer::DiskUsageSnapshot>),

    StorageAnalyzerError { path: PathBuf, message: String },

    StorageAnalyzerProgress {
        current_item: String,
        items_scanned: usize,
        total_bytes: u64,
    },
}

pub struct SudoPasswordRequest {

    pub label: String,

    pub retry_error: Option<String>,
    pub respond: tokio::sync::oneshot::Sender<Option<String>>,
}

pub type SudoPasswordTx = tokio::sync::mpsc::UnboundedSender<SudoPasswordRequest>;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Quit,
    NextTab,
    PrevTab,
    SelectTab(usize),
    Up,
    Down,
    Left,
    Right,
    Enter,
    Refresh,
    CheckUpdates,
    ToggleHelp,

    ToggleConfig,

    SaveConfig,

    ToggleDetail,

    KillTopProcess,

    BrightnessUp,

    BrightnessDown,

    KbdBrightnessUp,

    KbdBrightnessDown,

    ToggleAirplaneMode,

    VolumeUp,

    VolumeDown,

    ToggleMute,

    CyclePowerProfile,

    Redraw,

    StorageMount(DeviceId),

    StorageUnmount(DeviceId),

    StorageEject(DeviceId),

    StorageRefresh,

    StorageMountToggleSelected,

    StorageEjectSelected,

    StorageFormatOpen,

    StorageFlasherOpen,

    StorageFormat {
        device_id: String,
        fs_type: String,
        label: String,
    },

    StorageChecksumIso(String),

    StorageFlashIso {
        device_id: String,
        iso_path: String,
    },

    StorageFlashCancel {
        device_id: String,
    },

    StorageMultibootPrepareOpen,

    StorageMultibootPrepare {
        device_id: String,
    },

    StorageMultibootIsoManagerOpen,

    StorageMultibootListIsos {
        device_id: String,
    },

    StorageMultibootAddIso {
        device_id: String,
        src_path: String,
    },

    StorageMultibootRemoveIso {
        device_id: String,
        file_name: String,
    },

    StorageModalChar(char),

    StorageModalBackspace,

    StorageModalDelete,

    StorageModalOpenPicker,

    StorageOpenAnalyzer(Option<PathBuf>),

    StorageAnalyzerDrillDown,

    StorageAnalyzerGoUp,

    StorageAnalyzerRescan,

    StorageAnalyzerClose,

    StorageAnalyzerScan(PathBuf),

    NetworkRescan,
    NetworkToggleRadio,
    NetworkConnect {
        ap_id: String,
        ssid: String,
        password: Option<String>,
    },
    NetworkDisconnect(DeviceId),
    NetworkForget(String),
    NetworkModalChar(char),
    NetworkModalBackspace,

    BluetoothRescan,
    BluetoothToggleRadio,
    BluetoothConnect(DeviceId),
    BluetoothDisconnect(DeviceId),
    BluetoothPair(DeviceId),
    BluetoothForget(DeviceId),
    BluetoothToggleBlock(DeviceId),

    AudioSetVolume { node_id: u32, volume: f32 },
    AudioVolumeUp(u32, f32),
    AudioVolumeDown(u32, f32),
    AudioToggleMute(u32),
    AudioSetDefault(u32),
    AudioSelectCategory(usize),

    DisplaySetLayout(crate::backend::display::DisplayLayoutMode),
    DisplaySetResolution { display: String, mode: String, rate: Option<f32> },
    DisplaySetPrimary(String),
}
