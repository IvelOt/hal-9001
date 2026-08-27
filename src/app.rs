use std::path::PathBuf;
use std::time::Instant;

use tokio::sync::broadcast;

use crate::backend::disk_analyzer::DiskUsageItem;
use crate::backend::storage::{
    primary_partition, DriveInfo, PartitionInfo, StorageSnapshot, VentoyIsoEntry,
};
use crate::backend::system::SystemSnapshot;
use crate::config::Config;
use crate::events::{Action, AppEvent, Toast};
use crate::ui::file_picker::{self, FileEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsChoice {
    Vfat,
    Exfat,
    Ext4,
    Ntfs,
    Btrfs,
}

impl FsChoice {
    pub const ALL: [FsChoice; 5] = [
        FsChoice::Vfat,
        FsChoice::Exfat,
        FsChoice::Ext4,
        FsChoice::Ntfs,
        FsChoice::Btrfs,
    ];

    pub fn udisks_type(self) -> &'static str {
        match self {
            FsChoice::Vfat => "vfat",
            FsChoice::Exfat => "exfat",
            FsChoice::Ext4 => "ext4",
            FsChoice::Ntfs => "ntfs",
            FsChoice::Btrfs => "btrfs",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            FsChoice::Vfat => "FAT32 (vfat)",
            FsChoice::Exfat => "exFAT",
            FsChoice::Ext4 => "ext4",
            FsChoice::Ntfs => "NTFS",
            FsChoice::Btrfs => "btrfs",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatField {
    Fs,
    Label,
    Confirm,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FormatModalState {
    pub device_id: String,
    pub target_label: String,
    pub fs_idx: usize,
    pub label: String,
    pub field: FormatField,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlasherStage {
    SelectIso {
        input: String,
        error: Option<String>,
    },
    Checksumming {
        pct: f32,
    },
    Ready {
        sha256: Option<String>,
    },
    Confirm1,
    Confirm2 {
        typed: String,
    },
    Flashing {
        bytes_written: u64,
        total_bytes: u64,
        speed_mbps: f64,
        eta_secs: u64,
    },
    Done {
        ok: bool,
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlasherModalState {
    pub device_id: String,
    pub target_label: String,
    pub target_dev_node: String,
    pub target_size: u64,
    pub iso_path: PathBuf,
    pub iso_size: u64,
    pub stage: FlasherStage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilePickerPurpose {
    FlasherIso {
        device_id: String,
        target_label: String,
        target_dev_node: String,
        target_size: u64,
    },
    MultibootAddIso {
        device_id: String,
        target_label: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilePickerOutcome {
    None,

    Picked(PathBuf),

    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilePickerState {
    pub cwd: PathBuf,

    pub entries: Vec<FileEntry>,
    pub selected: usize,
    pub error: Option<String>,
    pub purpose: FilePickerPurpose,
}

impl FilePickerState {
    pub fn open(start_dir: PathBuf, purpose: FilePickerPurpose) -> Self {
        let cwd = if start_dir.is_dir() {
            start_dir
        } else {
            std::env::temp_dir()
        };
        let mut s = Self {
            cwd,
            entries: Vec::new(),
            selected: 0,
            error: None,
            purpose,
        };
        s.reload();
        s
    }

    pub fn reload(&mut self) {
        match file_picker::list_dir(&self.cwd) {
            Ok(entries) => {
                self.selected = if entries.is_empty() {
                    0
                } else {
                    self.selected.min(entries.len() - 1)
                };
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.selected = 0;
                self.error = Some(e);
            }
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.entries.len() {
            self.selected += 1;
        }
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.cwd.parent() {
            self.cwd = parent.to_path_buf();
            self.selected = 0;
            self.reload();
        }
    }

    pub fn jump_to(&mut self, path: PathBuf) {
        if path.is_dir() {
            self.cwd = path;
            self.selected = 0;
            self.reload();
        } else {
            self.error = Some(format!("{}", path.display()));
        }
    }

    pub fn enter_selected(&mut self) -> FilePickerOutcome {
        let Some(entry) = self.entries.get(self.selected).cloned() else {
            return FilePickerOutcome::None;
        };
        if entry.is_dir {
            self.cwd = entry.path;
            self.selected = 0;
            self.reload();
            FilePickerOutcome::None
        } else if file_picker::is_pickable_for(&self.purpose, &entry.name) {
            FilePickerOutcome::Picked(entry.path)
        } else {
            FilePickerOutcome::Unsupported
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum MultibootIsoManagerStage {
    Loading,
    Listing {
        entries: Vec<VentoyIsoEntry>,
        selected: usize,
        free_bytes: Option<u64>,
    },
    ConfirmRemove {
        file_name: String,
    },
    Copying {
        bytes_written: u64,
        total_bytes: u64,
        file_name: String,
    },
    Removing {
        file_name: String,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultibootIsoManagerState {
    pub device_id: String,
    pub target_label: String,
    pub stage: MultibootIsoManagerStage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiskAnalyzerState {
    pub current_path: PathBuf,
    pub total_bytes: u64,
    pub items: Vec<DiskUsageItem>,
    pub selected: usize,
    pub is_scanning: bool,
    pub error: Option<String>,

    pub current_scanning_item: Option<String>,

    pub files_scanned: usize,

    pub spinner_frame: usize,
}

impl DiskAnalyzerState {
    pub fn opening(path: PathBuf) -> Self {
        Self {
            current_path: path,
            total_bytes: 0,
            items: Vec::new(),
            selected: 0,
            is_scanning: true,
            error: None,
            current_scanning_item: None,
            files_scanned: 0,
            spinner_frame: 0,
        }
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.items.len() {
            self.selected += 1;
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum StorageModal {
    #[default]
    None,
    Format(FormatModalState),
    Flasher(FlasherModalState),
    FilePicker(FilePickerState),
    MultibootIsoManager(MultibootIsoManagerState),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Overview,
    Network,
    Bluetooth,
    Storage,
    Audio,
    Displays,
}

use crate::i18n::Language;

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Overview,
        Tab::Network,
        Tab::Bluetooth,
        Tab::Storage,
        Tab::Audio,
        Tab::Displays,
    ];

    pub fn index(self) -> usize {
        Tab::ALL.iter().position(|&t| t == self).unwrap_or(0)
    }

    pub fn from_index(i: usize) -> Tab {
        Tab::ALL.get(i).copied().unwrap_or(Tab::Overview)
    }

    pub fn title_in(self, lang: Language) -> &'static str {
        let m = lang.messages();
        match self {
            Tab::Overview => m.tab_overview,
            Tab::Network => m.tab_network,
            Tab::Bluetooth => m.tab_bluetooth,
            Tab::Storage => m.tab_storage,
            Tab::Audio => m.tab_audio,
            Tab::Displays => m.tab_displays,
        }
    }

    pub fn title(self) -> &'static str {
        self.title_in(Language::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Splash,
    Running,
}

#[derive(Debug, Clone, Default)]
pub struct ServiceStatus {
    pub degraded: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SudoPromptState {
    pub label: String,
    pub password: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WifiPasswordPromptState {
    pub ap_id: String,
    pub ssid: String,
    pub password: String,
    pub error: Option<String>,
}

pub struct App {
    pub config: Config,

    pub lang: Language,
    pub shared_lang: crate::i18n::SharedLang,
    pub should_quit: bool,
    pub phase: Phase,
    pub active: Tab,
    pub show_help: bool,

    pub show_config: bool,

    pub config_cursor: usize,

    pub detailed_overview: bool,

    pub selection: [usize; 6],

    pub system: Option<SystemSnapshot>,

    pub storage: Option<StorageSnapshot>,

    pub storage_selected: usize,

    pub storage_modal: StorageModal,

    pub storage_analyzer: Option<DiskAnalyzerState>,

    pub sudo_prompt: Option<SudoPromptState>,
    sudo_respond: Option<tokio::sync::oneshot::Sender<Option<String>>>,

    pub network: Option<Box<crate::backend::network::NetworkSnapshot>>,
    pub network_selected: usize,
    pub network_scanning: bool,
    pub wifi_prompt: Option<WifiPasswordPromptState>,

    pub bluetooth: Option<Box<crate::backend::bluetooth::BluetoothSnapshot>>,
    pub bluetooth_selected: usize,
    pub bluetooth_scanning: bool,

    pub audio: Option<Box<crate::backend::audio::AudioSnapshot>>,
    pub audio_selected: usize,
    pub audio_category: usize,

    pub displays: Option<Box<crate::backend::display::DisplaySnapshot>>,
    pub display_selected: usize,
    pub display_res_selected: usize,

    pub services: std::collections::HashMap<&'static str, ServiceStatus>,

    pub toast: Option<(Toast, Instant)>,

    started: Instant,
}

impl App {
    pub fn new(config: Config) -> Self {
        let phase = if config.splash.enabled {
            Phase::Splash
        } else {
            Phase::Running
        };
        let lang = config.ui.resolved_language();
        let shared_lang = crate::i18n::SharedLang::new(lang);
        Self {
            config,
            lang,
            shared_lang,
            should_quit: false,
            phase,
            active: Tab::Overview,
            show_help: false,
            show_config: false,
            config_cursor: 0,
            detailed_overview: false,
            selection: [0; 6],
            system: None,
            storage: None,
            storage_selected: 0,
            storage_modal: StorageModal::None,
            storage_analyzer: None,
            sudo_prompt: None,
            sudo_respond: None,
            network: None,
            network_selected: 0,
            network_scanning: false,
            wifi_prompt: None,
            bluetooth: None,
            bluetooth_selected: 0,
            bluetooth_scanning: false,
            audio: None,
            audio_selected: 0,
            audio_category: 0,
            displays: None,
            display_selected: 0,
            display_res_selected: 0,
            services: std::collections::HashMap::new(),
            toast: None,
            started: Instant::now(),
        }
    }

    pub fn needs_continuous_tick(&self) -> bool {
        self.phase == Phase::Splash
            || self.active == Tab::Overview
            || self.toast.is_some()
            || self
                .storage_analyzer
                .as_ref()
                .map(|a| a.is_scanning)
                .unwrap_or(false)
    }

    pub fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    pub fn on_tick(&mut self) {
        if self.phase == Phase::Splash && self.elapsed_ms() as u64 >= self.config.splash.min_ms {
            self.phase = Phase::Running;
        }

        if let Some((_, at)) = &self.toast {
            if at.elapsed().as_secs() >= 4 {
                self.toast = None;
            }
        }
        if let Some(state) = &mut self.storage_analyzer {
            if state.is_scanning {
                state.spinner_frame = state.spinner_frame.wrapping_add(1);
            }
        }
    }

    pub fn handle_event(&mut self, event: AppEvent) -> Vec<Action> {
        let mut follow_up: Vec<Action> = Vec::new();
        match event {
            AppEvent::System(snap) => {
                if let (Some(old), Some(new)) = (
                    self.system.as_ref().and_then(|s| s.battery.as_ref()),
                    snap.battery.as_ref(),
                ) {
                    use crate::backend::system::BatteryStatus;
                    let m = self.lang.messages();
                    if old.status != BatteryStatus::Charging
                        && new.status == BatteryStatus::Charging
                    {
                        self.toast = Some((
                            Toast::info(format!("{} {}", m.tag_power, m.toast_charger_connected)),
                            Instant::now(),
                        ));
                    } else if old.status == BatteryStatus::Charging
                        && new.status != BatteryStatus::Charging
                    {
                        self.toast = Some((
                            Toast::info(format!("{} {}", m.tag_power, m.toast_on_battery)),
                            Instant::now(),
                        ));
                    }
                    if new.status == BatteryStatus::Discharging
                        && new.percent < 15.0
                        && (old.percent >= 15.0 || old.status != BatteryStatus::Discharging)
                    {
                        self.toast = Some((
                            Toast::warn(format!(
                                "{} {}: {:.0}% {}",
                                m.tag_battery,
                                m.toast_battery_critical_label,
                                new.percent,
                                m.toast_battery_remaining_suffix
                            )),
                            Instant::now(),
                        ));
                    }
                }
                self.system = Some(*snap);
            }
            AppEvent::Storage(snap) => {
                if let Some(old_snap) = self.storage.as_ref() {
                    let m = self.lang.messages();
                    for new_drv in &snap.drives {
                        if new_drv.removable {
                            let was_present = old_snap.drives.iter().any(|d| d.id == new_drv.id);
                            if !was_present {
                                self.toast = Some((
                                    Toast::success(format!(
                                        "{} {} {}",
                                        m.tag_disk, m.toast_device_connected, new_drv.dev_node
                                    )),
                                    Instant::now(),
                                ));
                            }
                        }
                    }
                }
                self.storage = Some(*snap);
            }
            AppEvent::Network(snap) => {
                if let Some(old_snap) = self.network.as_ref() {
                    let m = self.lang.messages();
                    let old_has_ip = old_snap.active.is_some() && old_snap.telemetry.ipv4.is_some();
                    let new_has_ip = snap.active.is_some() && snap.telemetry.ipv4.is_some();

                    if !old_has_ip && new_has_ip {
                        if let (Some(active), Some(ip)) = (&snap.active, &snap.telemetry.ipv4) {
                            self.toast = Some((
                                Toast::success(format!(
                                    "{} {} '{}' (IP: {})",
                                    m.tag_network, m.toast_connected_at, active.ssid, ip
                                )),
                                Instant::now(),
                            ));
                        }
                    } else if old_has_ip && !new_has_ip {
                        self.toast = Some((
                            Toast::warn(format!(
                                "{} {}",
                                m.tag_network, m.toast_network_disconnected
                            )),
                            Instant::now(),
                        ));
                    }
                }
                let ap_count = snap.access_points.len();
                self.network = Some(snap);
                if ap_count > 0 && self.network_selected >= ap_count {
                    self.network_selected = ap_count - 1;
                }
            }
            AppEvent::NetworkScanning(flag) => self.network_scanning = flag,
            AppEvent::Bluetooth(snap) => {
                if let Some(old_snap) = self.bluetooth.as_ref() {
                    let m = self.lang.messages();
                    for new_dev in &snap.devices {
                        if let Some(old_dev) = old_snap.devices.iter().find(|d| d.id == new_dev.id)
                        {
                            if !old_dev.connected && new_dev.connected {
                                let bat_str = match new_dev.battery_percentage {
                                    Some(b) => format!(" ({}: {}%)", m.label_battery, b),
                                    None => "".to_string(),
                                };
                                self.toast = Some((
                                    Toast::success(format!(
                                        "{} {} {}{}",
                                        m.tag_bluetooth,
                                        m.toast_bt_connected_prefix,
                                        new_dev.name,
                                        bat_str
                                    )),
                                    Instant::now(),
                                ));
                            } else if old_dev.connected && !new_dev.connected {
                                self.toast = Some((
                                    Toast::info(format!(
                                        "{} {} {}",
                                        m.tag_bluetooth,
                                        m.toast_bt_disconnected_prefix,
                                        new_dev.name
                                    )),
                                    Instant::now(),
                                ));
                            }
                        }
                    }
                }
                let dev_count = snap.devices.len();
                self.bluetooth = Some(snap);
                if dev_count > 0 && self.bluetooth_selected >= dev_count {
                    self.bluetooth_selected = dev_count - 1;
                }
            }
            AppEvent::BluetoothScanning(flag) => self.bluetooth_scanning = flag,
            AppEvent::Audio(snap) => {
                let cat = match self.audio_category {
                    0 => crate::backend::audio::AudioCategory::Sink,
                    1 => crate::backend::audio::AudioCategory::AppStream,
                    _ => crate::backend::audio::AudioCategory::Source,
                };
                let node_count = snap.nodes_for_category(cat).len();
                self.audio = Some(snap);
                if node_count > 0 && self.audio_selected >= node_count {
                    self.audio_selected = node_count - 1;
                }
            }
            AppEvent::Display(snap) => {
                let count = snap.displays.len();
                self.displays = Some(snap);
                if count > 0 && self.display_selected >= count {
                    self.display_selected = count - 1;
                }
            }
            AppEvent::Toast(toast) => self.toast = Some((toast, Instant::now())),
            AppEvent::ServiceDegraded { name, reason } => {
                self.services.insert(
                    name,
                    ServiceStatus {
                        degraded: Some(reason),
                    },
                );
            }
            AppEvent::StorageChecksumProgress { path, pct } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if s.iso_path == path {
                        if let FlasherStage::Checksumming { pct: p } = &mut s.stage {
                            *p = pct;
                        }
                    }
                }
            }
            AppEvent::StorageChecksumDone { path, sha256 } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if s.iso_path == path {
                        s.stage = FlasherStage::Ready {
                            sha256: Some(sha256),
                        };
                    }
                }
            }
            AppEvent::StorageFlashProgress {
                bytes_written,
                total_bytes,
                speed_mbps,
                eta_secs,
            } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if let FlasherStage::Flashing {
                        bytes_written: bw,
                        total_bytes: tb,
                        speed_mbps: sp,
                        eta_secs: eta,
                    } = &mut s.stage
                    {
                        *bw = bytes_written;
                        *tb = total_bytes;
                        *sp = speed_mbps;
                        *eta = eta_secs;
                    }
                }
            }
            AppEvent::StorageFlashDone { device_id, result } => {
                if let StorageModal::Flasher(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        let m = self.lang.messages();
                        s.stage = match result {
                            Ok(_msg) => {
                                let message =
                                    format!("{} {}", m.tag_flasher, m.storage_flash_success);
                                self.toast =
                                    Some((Toast::success(message.clone()), Instant::now()));
                                FlasherStage::Done { ok: true, message }
                            }
                            Err(err) => {
                                self.toast = Some((
                                    Toast::error(format!(
                                        "{} {}",
                                        m.tag_flasher, m.storage_flash_failed
                                    )),
                                    Instant::now(),
                                ));
                                FlasherStage::Done {
                                    ok: false,
                                    message: err,
                                }
                            }
                        };
                    }
                }
            }
            AppEvent::StorageMultibootIsoList {
                device_id,
                entries,
                free_bytes,
            } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        s.stage = MultibootIsoManagerStage::Listing {
                            entries,
                            selected: 0,
                            free_bytes,
                        };
                    }
                }
            }
            AppEvent::StorageMultibootIsoCopyProgress {
                device_id,
                bytes_written,
                total_bytes,
            } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        if let MultibootIsoManagerStage::Copying {
                            bytes_written: bw,
                            total_bytes: tb,
                            ..
                        } = &mut s.stage
                        {
                            *bw = bytes_written;
                            *tb = total_bytes;
                        }
                    }
                }
            }
            AppEvent::StorageMultibootIsoCopyDone { device_id, result } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        match result {
                            Ok(_) => {
                                s.stage = MultibootIsoManagerStage::Loading;
                                follow_up.push(Action::StorageMultibootListIsos {
                                    device_id: device_id.clone(),
                                });
                            }
                            Err(e) => {
                                s.stage = MultibootIsoManagerStage::Error { message: e };
                            }
                        }
                    }
                }
            }
            AppEvent::StorageMultibootIsoRemoveDone { device_id, result } => {
                if let StorageModal::MultibootIsoManager(s) = &mut self.storage_modal {
                    if s.device_id == device_id {
                        match result {
                            Ok(_) => {
                                s.stage = MultibootIsoManagerStage::Loading;
                                follow_up.push(Action::StorageMultibootListIsos {
                                    device_id: device_id.clone(),
                                });
                            }
                            Err(e) => {
                                s.stage = MultibootIsoManagerStage::Error { message: e };
                            }
                        }
                    }
                }
            }
            AppEvent::StorageAnalyzerSnapshot(snap) => {
                if let Some(state) = &mut self.storage_analyzer {
                    if state.current_path == snap.current_path {
                        state.total_bytes = snap.total_bytes;
                        state.items = snap.items;
                        state.selected = 0;
                        state.is_scanning = false;
                        state.error = None;
                        state.current_scanning_item = None;
                    }
                }
            }
            AppEvent::StorageAnalyzerError { message, .. } => {
                if let Some(state) = &mut self.storage_analyzer {
                    state.is_scanning = false;
                    state.error = Some(message);
                }
            }
            AppEvent::StorageAnalyzerProgress {
                current_item,
                items_scanned,
                total_bytes,
            } => {
                if let Some(state) = &mut self.storage_analyzer {
                    if state.is_scanning {
                        state.current_scanning_item = Some(current_item);
                        state.files_scanned = items_scanned;
                        state.total_bytes = total_bytes;
                    }
                }
            }
        }
        follow_up
    }

    pub fn config_prev_field(&mut self) {
        self.config_cursor = if self.config_cursor == 0 {
            6
        } else {
            self.config_cursor - 1
        };
    }

    pub fn config_next_field(&mut self) {
        self.config_cursor = (self.config_cursor + 1) % 7;
    }

    pub fn config_prev_value(&mut self) {
        self.cycle_config_value(false);
    }

    pub fn config_next_value(&mut self) {
        self.cycle_config_value(true);
    }

    fn cycle_config_value(&mut self, forward: bool) {
        match self.config_cursor {
            0 => {
                let options = ["auto", "pt-BR", "en-US", "es-ES"];
                let cur = options
                    .iter()
                    .position(|&s| s.eq_ignore_ascii_case(&self.config.ui.language))
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.ui.language = options[next].to_string();
                self.lang = self.config.ui.resolved_language();
                self.shared_lang.set(self.lang);
            }
            1 => {
                let options = [
                    "hal",
                    "catppuccin",
                    "tokyo-night",
                    "nord",
                    "gruvbox",
                    "cyberpunk",
                    "dracula",
                    "mono",
                ];
                let cur = options
                    .iter()
                    .position(|&s| s.eq_ignore_ascii_case(&self.config.theme.name))
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.theme.name = options[next].to_string();
            }
            2 => {
                let options = ["auto", "main", "medium", "compact", "none"];
                let cur = options
                    .iter()
                    .position(|&s| s.eq_ignore_ascii_case(&self.config.overview.ascii))
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.overview.ascii = options[next].to_string();
            }
            3 => {
                self.config.ui.icons = !self.config.ui.icons;
            }
            4 => {
                let options = [33u64, 16, 66];
                let cur = options
                    .iter()
                    .position(|&ms| ms == self.config.ui.frame_ms)
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                self.config.ui.frame_ms = options[next];
            }
            5 => {
                self.config.splash.enabled = !self.config.splash.enabled;
            }
            6 => {
                let options = [1500u64, 750, 3000];
                let cur = options
                    .iter()
                    .position(|&ms| ms == self.config.polling.system_ms)
                    .unwrap_or(0);
                let next = if forward {
                    (cur + 1) % options.len()
                } else {
                    (cur + options.len() - 1) % options.len()
                };
                let base = options[next];
                self.config.polling.system_ms = base;
                self.config.polling.audio_ms = base;
                self.config.polling.bluetooth_ms = base * 2;
                self.config.polling.network_ms = base * 3;
                self.config.polling.display_ms = base + 500;
                self.config.polling.storage_ms = base * 4;
            }
            _ => {}
        }
    }

    pub fn storage_drive_index(&self) -> Option<usize> {
        let snap = self.storage.as_ref()?;
        if snap.drives.is_empty() {
            return None;
        }
        Some(self.storage_selected.min(snap.drives.len() - 1))
    }

    pub fn storage_selection(&self) -> Option<(&DriveInfo, Option<&PartitionInfo>)> {
        let snap = self.storage.as_ref()?;
        let idx = self.storage_drive_index()?;
        let drive = snap.drive(idx)?;
        Some((drive, primary_partition(drive)))
    }

    fn storage_mount_toggle(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((_, Some(partition))) = self.storage_selection() else {
            return;
        };
        let action = if partition.is_mounted() {
            Action::StorageUnmount(partition.id.clone())
        } else {
            Action::StorageMount(partition.id.clone())
        };
        let _ = action_tx.send(action);
    }

    fn storage_eject_selected(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((drive, _)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let _ = action_tx.send(Action::StorageEject(drive.id.clone()));
    }

    pub fn storage_modal_open(&self) -> bool {
        !matches!(self.storage_modal, StorageModal::None)
    }

    pub fn sudo_prompt_open(&self) -> bool {
        self.sudo_prompt.is_some()
    }

    pub fn open_sudo_prompt(&mut self, req: crate::events::SudoPasswordRequest) {
        self.sudo_prompt = Some(SudoPromptState {
            label: req.label,
            password: String::new(),
            error: req.retry_error,
        });
        self.sudo_respond = Some(req.respond);
    }

    fn dispatch_sudo_prompt(&mut self, action: Action) {
        let Some(state) = &mut self.sudo_prompt else {
            return;
        };
        match action {
            Action::Quit => self.should_quit = true,
            Action::StorageModalChar(c) => {
                if !c.is_control() {
                    state.password.push(c);
                }
            }
            Action::StorageModalBackspace => {
                state.password.pop();
            }
            Action::Enter => {
                let password = state.password.clone();
                self.sudo_prompt = None;
                if let Some(respond) = self.sudo_respond.take() {
                    let _ = respond.send(Some(password));
                }
            }
            Action::ToggleConfig => {
                self.sudo_prompt = None;
                if let Some(respond) = self.sudo_respond.take() {
                    let _ = respond.send(None);
                }
            }
            _ => {}
        }
    }

    fn lock_tag(&self) -> String {
        if self.config.ui.icons {
            "\u{f023} ".to_string()
        } else {
            "[LOCKED] ".to_string()
        }
    }

    fn toast_system_locked(&mut self) {
        let msg = format!(
            "{}{}",
            self.lock_tag(),
            self.lang.messages().storage_err_system
        );
        self.toast = Some((Toast::error(msg), Instant::now()));
    }

    pub fn wifi_prompt_open(&self) -> bool {
        self.wifi_prompt.is_some()
    }

    pub fn storage_analyzer_open(&self) -> bool {
        self.storage_analyzer.is_some()
    }

    fn dispatch_wifi_prompt(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        let Some(state) = &mut self.wifi_prompt else {
            return;
        };
        match action {
            Action::Quit => self.should_quit = true,
            Action::NetworkModalChar(c) => {
                if !c.is_control() {
                    state.password.push(c);
                }
            }
            Action::NetworkModalBackspace => {
                state.password.pop();
            }
            Action::Enter => {
                let ap_id = state.ap_id.clone();
                let ssid = state.ssid.clone();
                let password = state.password.clone();
                self.wifi_prompt = None;
                let _ = action_tx.send(Action::NetworkConnect {
                    ap_id,
                    ssid,
                    password: Some(password),
                });
            }
            Action::ToggleConfig => {
                self.wifi_prompt = None;
            }
            _ => {}
        }
    }

    pub fn text_input_active(&self) -> bool {
        if self.sudo_prompt.is_some() || self.wifi_prompt.is_some() {
            return true;
        }
        match &self.storage_modal {
            StorageModal::Format(s) => s.field == FormatField::Label,
            StorageModal::Flasher(s) => matches!(
                s.stage,
                FlasherStage::SelectIso { .. } | FlasherStage::Confirm2 { .. }
            ),
            StorageModal::FilePicker(_) => false,
            StorageModal::MultibootIsoManager(_) => false,
            StorageModal::None => false,
        }
    }

    fn storage_format_open(&mut self) {
        let Some((drive, _)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let target_label = drive.friendly_label();
        self.storage_modal = StorageModal::Format(FormatModalState {
            device_id: drive.id.0.clone(),
            target_label,
            fs_idx: 0,
            label: "PENDRIVE".to_string(),
            field: FormatField::Fs,
        });
    }

    fn storage_flasher_open(&mut self) {
        let Some((drive, _)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let target_label = drive.friendly_label();
        self.storage_modal = StorageModal::FilePicker(FilePickerState::open(
            Self::home_dir(),
            FilePickerPurpose::FlasherIso {
                device_id: drive.id.0.clone(),
                target_label,
                target_dev_node: drive.dev_node.clone(),
                target_size: drive.size,
            },
        ));
    }

    fn storage_analyzer_open_selected(&mut self, action_tx: &broadcast::Sender<Action>) {
        let start = self
            .storage_selection()
            .and_then(|(_, part)| part)
            .and_then(|p| p.mount_points.first().cloned())
            .map(PathBuf::from)
            .unwrap_or_else(Self::home_dir);
        self.storage_analyzer = Some(DiskAnalyzerState::opening(start.clone()));
        let _ = action_tx.send(Action::StorageAnalyzerScan(start));
    }

    fn storage_analyzer_drill_down(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some(state) = &self.storage_analyzer else {
            return;
        };
        let Some(item) = state.items.get(state.selected) else {
            return;
        };
        if !item.is_dir {
            return;
        }
        let new_path = state.current_path.join(&item.name);
        if let Some(state) = &mut self.storage_analyzer {
            state.current_path = new_path.clone();
            state.items.clear();
            state.selected = 0;
            state.is_scanning = true;
            state.error = None;
            state.current_scanning_item = None;
            state.files_scanned = 0;
        }
        let _ = action_tx.send(Action::StorageAnalyzerScan(new_path));
    }

    fn storage_analyzer_go_up(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some(state) = &self.storage_analyzer else {
            return;
        };
        let Some(parent) = state.current_path.parent().map(|p| p.to_path_buf()) else {
            return;
        };
        if let Some(state) = &mut self.storage_analyzer {
            state.current_path = parent.clone();
            state.items.clear();
            state.selected = 0;
            state.is_scanning = true;
            state.error = None;
            state.current_scanning_item = None;
            state.files_scanned = 0;
        }
        let _ = action_tx.send(Action::StorageAnalyzerScan(parent));
    }

    fn storage_analyzer_rescan(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some(state) = &mut self.storage_analyzer else {
            return;
        };
        state.is_scanning = true;
        state.error = None;
        state.current_scanning_item = None;
        state.files_scanned = 0;
        let path = state.current_path.clone();
        let _ = action_tx.send(Action::StorageAnalyzerScan(path));
    }

    fn dispatch_storage_analyzer(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        match action {
            Action::Quit => self.should_quit = true,
            Action::StorageAnalyzerClose | Action::ToggleConfig => self.storage_analyzer = None,
            Action::Up => {
                if let Some(s) = &mut self.storage_analyzer {
                    s.move_up();
                }
            }
            Action::Down => {
                if let Some(s) = &mut self.storage_analyzer {
                    s.move_down();
                }
            }
            Action::StorageAnalyzerDrillDown | Action::Enter => {
                self.storage_analyzer_drill_down(action_tx)
            }
            Action::StorageAnalyzerGoUp => self.storage_analyzer_go_up(action_tx),
            Action::StorageAnalyzerRescan => self.storage_analyzer_rescan(action_tx),
            _ => {}
        }
    }

    fn storage_multiboot_prepare_open(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((drive, partition)) = self.storage_selection() else {
            return;
        };
        if drive.is_system {
            self.toast_system_locked();
            return;
        }
        let Some(partition) = partition else {
            let m = self.lang.messages();
            self.toast = Some((
                Toast::error(m.storage_multiboot_no_partition),
                Instant::now(),
            ));
            return;
        };
        let _ = action_tx.send(Action::StorageMultibootPrepare {
            device_id: partition.id.0.clone(),
        });
    }

    fn storage_multiboot_iso_manager_open(&mut self, action_tx: &broadcast::Sender<Action>) {
        let Some((drive, partition)) = self.storage_selection() else {
            return;
        };
        let Some(partition) = partition else {
            let m = self.lang.messages();
            self.toast = Some((
                Toast::error(m.storage_multiboot_no_partition),
                Instant::now(),
            ));
            return;
        };
        let target_label = drive.friendly_label();
        let device_id = partition.id.0.clone();
        self.storage_modal = StorageModal::MultibootIsoManager(MultibootIsoManagerState {
            device_id: device_id.clone(),
            target_label,
            stage: MultibootIsoManagerStage::Loading,
        });
        let _ = action_tx.send(Action::StorageMultibootListIsos { device_id });
    }

    fn dispatch_storage_modal(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        let modal = std::mem::take(&mut self.storage_modal);
        self.storage_modal = match modal {
            StorageModal::None => StorageModal::None,
            StorageModal::Format(s) => self.dispatch_format_modal(s, action, action_tx),
            StorageModal::Flasher(s) => self.dispatch_flasher_modal(s, action, action_tx),
            StorageModal::FilePicker(s) => self.dispatch_file_picker_modal(s, action, action_tx),
            StorageModal::MultibootIsoManager(s) => {
                self.dispatch_multiboot_iso_manager_modal(s, action, action_tx)
            }
        };
    }

    fn home_dir() -> PathBuf {
        directories::UserDirs::new()
            .map(|u| u.home_dir().to_path_buf())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
    }

    fn downloads_dir() -> PathBuf {
        directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(Self::home_dir)
    }

    fn dispatch_format_modal(
        &mut self,
        mut s: FormatModalState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        match action {
            Action::Quit => self.should_quit = true,

            Action::ToggleConfig => return StorageModal::None,
            Action::Up => {
                s.field = match s.field {
                    FormatField::Label => FormatField::Fs,
                    FormatField::Confirm => FormatField::Label,
                    FormatField::Fs => FormatField::Fs,
                };
            }
            Action::Down => {
                s.field = match s.field {
                    FormatField::Fs => FormatField::Label,
                    FormatField::Label => FormatField::Confirm,
                    FormatField::Confirm => FormatField::Confirm,
                };
            }

            Action::NextTab => {
                s.field = match s.field {
                    FormatField::Fs => FormatField::Label,
                    FormatField::Label => FormatField::Confirm,
                    FormatField::Confirm => FormatField::Fs,
                };
            }
            Action::PrevTab => {
                s.field = match s.field {
                    FormatField::Fs => FormatField::Confirm,
                    FormatField::Label => FormatField::Fs,
                    FormatField::Confirm => FormatField::Label,
                };
            }
            Action::Left => {
                if s.field == FormatField::Fs {
                    let n = FsChoice::ALL.len();
                    s.fs_idx = (s.fs_idx + n - 1) % n;
                }
            }
            Action::Right => {
                if s.field == FormatField::Fs {
                    s.fs_idx = (s.fs_idx + 1) % FsChoice::ALL.len();
                }
            }
            Action::StorageModalChar(c) => {
                if s.field == FormatField::Label && !c.is_control() && s.label.chars().count() < 32
                {
                    s.label.push(c);
                }
            }
            Action::StorageModalBackspace => {
                if s.field == FormatField::Label {
                    s.label.pop();
                }
            }

            Action::Enter => {
                let fs = FsChoice::ALL[s.fs_idx];
                let label = if s.label.trim().is_empty() {
                    "PENDRIVE".to_string()
                } else {
                    s.label.clone()
                };
                let m = self.lang.messages();
                self.toast = Some((
                    Toast::info(format!(
                        "{} {} ({})",
                        m.storage_format_started,
                        s.target_label,
                        fs.label()
                    )),
                    Instant::now(),
                ));
                let _ = action_tx.send(Action::StorageFormat {
                    device_id: s.device_id.clone(),
                    fs_type: fs.udisks_type().to_string(),
                    label,
                });
                return StorageModal::None;
            }
            _ => {}
        }
        StorageModal::Format(s)
    }

    fn dispatch_flasher_modal(
        &mut self,
        mut s: FlasherModalState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return StorageModal::Flasher(s);
        }

        if matches!(action, Action::ToggleConfig) {
            if matches!(s.stage, FlasherStage::Flashing { .. }) {
                let _ = action_tx.send(Action::StorageFlashCancel {
                    device_id: s.device_id.clone(),
                });
            }
            return StorageModal::None;
        }

        let m = self.lang.messages();
        match &mut s.stage {
            FlasherStage::SelectIso { input, error } => match action {
                Action::StorageModalOpenPicker => {
                    return StorageModal::FilePicker(FilePickerState::open(
                        Self::home_dir(),
                        FilePickerPurpose::FlasherIso {
                            device_id: s.device_id.clone(),
                            target_label: s.target_label.clone(),
                            target_dev_node: s.target_dev_node.clone(),
                            target_size: s.target_size,
                        },
                    ));
                }
                Action::StorageModalChar(c) if !c.is_control() => input.push(c),
                Action::StorageModalBackspace => {
                    input.pop();
                }
                Action::Enter => match std::fs::metadata(input.trim()) {
                    Ok(meta) if meta.is_file() && meta.len() > 0 => {
                        let size = meta.len();
                        if size > s.target_size {
                            *error = Some(m.storage_flash_err_too_big.to_string());
                        } else {
                            s.iso_path = PathBuf::from(input.trim());
                            s.iso_size = size;
                            s.stage = FlasherStage::Ready { sha256: None };
                        }
                    }
                    Ok(_) => *error = Some(m.storage_flash_err_not_file.to_string()),
                    Err(_) => *error = Some(m.storage_flash_err_not_found.to_string()),
                },
                _ => {}
            },

            FlasherStage::Checksumming { .. } => {}
            FlasherStage::Ready { sha256 } => match action {
                Action::StorageModalChar('c') if sha256.is_none() => {
                    let _ = action_tx.send(Action::StorageChecksumIso(
                        s.iso_path.to_string_lossy().to_string(),
                    ));
                    s.stage = FlasherStage::Checksumming { pct: 0.0 };
                }
                Action::Enter => s.stage = FlasherStage::Confirm1,
                _ => {}
            },
            FlasherStage::Confirm1 => {
                if matches!(action, Action::Enter) {
                    s.stage = FlasherStage::Confirm2 {
                        typed: String::new(),
                    };
                }
            }
            FlasherStage::Confirm2 { typed } => match action {
                Action::StorageModalChar(c) if !c.is_control() => typed.push(c),
                Action::StorageModalBackspace => {
                    typed.pop();
                }
                Action::Enter => {
                    if typed.trim() == s.target_dev_node {
                        let _ = action_tx.send(Action::StorageFlashIso {
                            device_id: s.device_id.clone(),
                            iso_path: s.iso_path.to_string_lossy().to_string(),
                        });
                        s.stage = FlasherStage::Flashing {
                            bytes_written: 0,
                            total_bytes: s.iso_size,
                            speed_mbps: 0.0,
                            eta_secs: 0,
                        };
                    } else {
                        self.toast =
                            Some((Toast::error(m.storage_flash_err_mismatch), Instant::now()));
                    }
                }
                _ => {}
            },

            FlasherStage::Flashing { .. } => {}
            FlasherStage::Done { .. } => {
                if matches!(action, Action::Enter) {
                    return StorageModal::None;
                }
            }
        }
        StorageModal::Flasher(s)
    }

    fn dispatch_file_picker_modal(
        &mut self,
        mut s: FilePickerState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return StorageModal::FilePicker(s);
        }

        if matches!(action, Action::ToggleConfig) {
            return StorageModal::None;
        }

        match action {
            Action::Down | Action::StorageModalChar('j') => s.move_down(),
            Action::Up | Action::StorageModalChar('k') => s.move_up(),
            Action::Left | Action::StorageModalChar('h') | Action::StorageModalBackspace => {
                s.go_up()
            }
            Action::Right | Action::StorageModalChar('l') | Action::Enter => {
                return self.file_picker_enter(s, action_tx);
            }

            Action::StorageModalChar('~') => s.jump_to(Self::home_dir()),
            Action::StorageModalChar('d') | Action::StorageModalChar('D') => {
                s.jump_to(Self::downloads_dir())
            }
            Action::StorageModalChar('M') => s.jump_to(PathBuf::from("/media")),
            Action::StorageModalChar('/') => s.jump_to(PathBuf::from("/")),
            _ => {}
        }
        StorageModal::FilePicker(s)
    }

    fn file_picker_enter(
        &mut self,
        mut s: FilePickerState,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        match s.enter_selected() {
            FilePickerOutcome::None => StorageModal::FilePicker(s),
            FilePickerOutcome::Unsupported => {
                let m = self.lang.messages();
                s.error = Some(m.filepicker_err_unsupported.to_string());
                StorageModal::FilePicker(s)
            }
            FilePickerOutcome::Picked(path) => {
                let size = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
                match s.purpose.clone() {
                    FilePickerPurpose::FlasherIso {
                        device_id,
                        target_label,
                        target_dev_node,
                        target_size,
                    } => {
                        if size > target_size {
                            let m = self.lang.messages();
                            StorageModal::Flasher(FlasherModalState {
                                device_id,
                                target_label,
                                target_dev_node,
                                target_size,
                                iso_path: PathBuf::new(),
                                iso_size: 0,
                                stage: FlasherStage::SelectIso {
                                    input: path.to_string_lossy().to_string(),
                                    error: Some(m.storage_flash_err_too_big.to_string()),
                                },
                            })
                        } else {
                            StorageModal::Flasher(FlasherModalState {
                                device_id,
                                target_label,
                                target_dev_node,
                                target_size,
                                iso_path: path,
                                iso_size: size,
                                stage: FlasherStage::Ready { sha256: None },
                            })
                        }
                    }
                    FilePickerPurpose::MultibootAddIso {
                        device_id,
                        target_label,
                    } => {
                        let file_name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let _ = action_tx.send(Action::StorageMultibootAddIso {
                            device_id: device_id.clone(),
                            src_path: path.to_string_lossy().to_string(),
                        });
                        StorageModal::MultibootIsoManager(MultibootIsoManagerState {
                            device_id,
                            target_label,
                            stage: MultibootIsoManagerStage::Copying {
                                bytes_written: 0,
                                total_bytes: size,
                                file_name,
                            },
                        })
                    }
                }
            }
        }
    }

    fn dispatch_multiboot_iso_manager_modal(
        &mut self,
        mut s: MultibootIsoManagerState,
        action: Action,
        action_tx: &broadcast::Sender<Action>,
    ) -> StorageModal {
        if matches!(action, Action::Quit) {
            self.should_quit = true;
            return StorageModal::MultibootIsoManager(s);
        }

        match &mut s.stage {
            MultibootIsoManagerStage::Loading => {
                if matches!(action, Action::ToggleConfig) {
                    return StorageModal::None;
                }
            }
            MultibootIsoManagerStage::Listing {
                entries, selected, ..
            } => match action {
                Action::ToggleConfig => return StorageModal::None,
                Action::Down | Action::StorageModalChar('j') => {
                    if *selected + 1 < entries.len() {
                        *selected += 1;
                    }
                }
                Action::Up | Action::StorageModalChar('k') => {
                    *selected = selected.saturating_sub(1);
                }
                Action::StorageModalChar('a') | Action::StorageModalChar('A') => {
                    return StorageModal::FilePicker(FilePickerState::open(
                        Self::home_dir(),
                        FilePickerPurpose::MultibootAddIso {
                            device_id: s.device_id.clone(),
                            target_label: s.target_label.clone(),
                        },
                    ));
                }
                Action::StorageModalChar('d')
                | Action::StorageModalChar('x')
                | Action::StorageModalDelete => {
                    if let Some(e) = entries.get(*selected) {
                        let file_name = e.name.clone();
                        s.stage = MultibootIsoManagerStage::ConfirmRemove { file_name };
                    }
                }
                _ => {}
            },
            MultibootIsoManagerStage::ConfirmRemove { file_name } => match action {
                Action::Enter | Action::StorageModalChar('y') | Action::StorageModalChar('Y') => {
                    let _ = action_tx.send(Action::StorageMultibootRemoveIso {
                        device_id: s.device_id.clone(),
                        file_name: file_name.clone(),
                    });
                    s.stage = MultibootIsoManagerStage::Removing {
                        file_name: file_name.clone(),
                    };
                }
                Action::ToggleConfig
                | Action::StorageModalChar('n')
                | Action::StorageModalChar('N') => {
                    let _ = action_tx.send(Action::StorageMultibootListIsos {
                        device_id: s.device_id.clone(),
                    });
                    s.stage = MultibootIsoManagerStage::Loading;
                }
                _ => {}
            },
            MultibootIsoManagerStage::Copying { .. } => {
                if matches!(action, Action::ToggleConfig) {
                    return StorageModal::None;
                }
            }
            MultibootIsoManagerStage::Removing { .. } => {
                if matches!(action, Action::ToggleConfig) {
                    return StorageModal::None;
                }
            }
            MultibootIsoManagerStage::Error { .. } => {
                if matches!(action, Action::ToggleConfig | Action::Enter) {
                    return StorageModal::None;
                }
            }
        }
        StorageModal::MultibootIsoManager(s)
    }

    pub fn save_config(&mut self) {
        match self.config.save() {
            Ok(path) => {
                let m = self.lang.messages();
                let msg = m.toast_settings_saved.replace("{path}", &path.display().to_string());
                self.toast = Some((Toast::info(msg), Instant::now()));
            }
            Err(e) => {
                let m = self.lang.messages();
                self.toast = Some((
                    Toast::error(format!("{}: {e}", m.err_save_config_prefix)),
                    Instant::now(),
                ));
            }
        }
    }

    pub fn dispatch(&mut self, action: Action, action_tx: &broadcast::Sender<Action>) {
        if self.phase == Phase::Splash {
            self.phase = Phase::Running;
        }

        if self.sudo_prompt_open() {
            self.dispatch_sudo_prompt(action);
            return;
        }

        if self.wifi_prompt_open() {
            self.dispatch_wifi_prompt(action, action_tx);
            return;
        }

        if self.storage_modal_open() {
            self.dispatch_storage_modal(action, action_tx);
            return;
        }

        if self.storage_analyzer_open() {
            self.dispatch_storage_analyzer(action, action_tx);
            return;
        }

        if self.show_config {
            match action {
                Action::Quit => self.should_quit = true,
                Action::ToggleConfig => self.show_config = false,
                Action::Up => self.config_prev_field(),
                Action::Down => self.config_next_field(),
                Action::Left => self.config_prev_value(),
                Action::Right | Action::Enter => self.config_next_value(),
                Action::SaveConfig => self.save_config(),
                _ => {}
            }
            return;
        }

        match action {
            Action::Quit => self.should_quit = true,
            Action::KillTopProcess => {
                if let Some(sys) = &self.system {
                    if let Some(top) = sys.detail.top_processes.first() {
                        unsafe {
                            libc::kill(top.pid as libc::pid_t, libc::SIGTERM);
                        }
                        let m = self.lang.messages();
                        self.toast = Some((
                            Toast::info(format!(
                                "{} {} {} ({})",
                                m.tag_process, m.toast_kill_signal_sent, top.pid, top.name
                            )),
                            Instant::now(),
                        ));
                    }
                }
            }
            Action::NextTab => {
                self.active = Tab::from_index((self.active.index() + 1) % Tab::ALL.len());
            }
            Action::PrevTab => {
                let n = Tab::ALL.len();
                self.active = Tab::from_index((self.active.index() + n - 1) % n);
            }
            Action::SelectTab(i) => {
                self.active = Tab::from_index(i);
            }
            Action::Up => {
                if self.active == Tab::Storage {
                    self.storage_selected = self.storage_selected.saturating_sub(1);
                } else if self.active == Tab::Network {
                    self.network_selected = self.network_selected.saturating_sub(1);
                } else if self.active == Tab::Bluetooth {
                    self.bluetooth_selected = self.bluetooth_selected.saturating_sub(1);
                } else if self.active == Tab::Audio {
                    self.audio_selected = self.audio_selected.saturating_sub(1);
                } else if self.active == Tab::Displays {
                    self.display_res_selected = self.display_res_selected.saturating_sub(1);
                } else {
                    let i = self.active.index();
                    self.selection[i] = self.selection[i].saturating_sub(1);
                }
            }
            Action::Down => {
                if self.active == Tab::Storage {
                    self.storage_selected = self.storage_selected.saturating_add(1);
                } else if self.active == Tab::Network {
                    self.network_selected = self.network_selected.saturating_add(1);
                } else if self.active == Tab::Bluetooth {
                    self.bluetooth_selected = self.bluetooth_selected.saturating_add(1);
                } else if self.active == Tab::Audio {
                    self.audio_selected = self.audio_selected.saturating_add(1);
                } else if self.active == Tab::Displays {
                    self.display_res_selected = self.display_res_selected.saturating_add(1);
                } else {
                    let i = self.active.index();
                    self.selection[i] = self.selection[i].saturating_add(1);
                }
            }
            Action::Left => {
                if self.active == Tab::Displays {
                    self.display_selected = self.display_selected.saturating_sub(1);
                    self.display_res_selected = 0;
                }
            }
            Action::Right => {
                if self.active == Tab::Displays {
                    self.display_selected = self.display_selected.saturating_add(1);
                    self.display_res_selected = 0;
                }
            }
            Action::ToggleHelp => {
                self.show_help = !self.show_help;
                if self.show_help {
                    self.show_config = false;
                }
            }
            Action::ToggleConfig => {
                self.show_config = !self.show_config;
                if self.show_config {
                    self.show_help = false;
                }
            }
            Action::SaveConfig => self.save_config(),
            Action::ToggleDetail => self.detailed_overview = !self.detailed_overview,
            Action::Enter => {
                if self.active == Tab::Network {
                    if let Some(net) = &self.network {
                        if let Some(ap) = net.access_points.get(self.network_selected) {
                            if ap.security.needs_password() && !ap.is_saved {
                                self.wifi_prompt = Some(WifiPasswordPromptState {
                                    ap_id: ap.id.0.clone(),
                                    ssid: ap.ssid.clone(),
                                    password: String::new(),
                                    error: None,
                                });
                            } else {
                                let _ = action_tx.send(Action::NetworkConnect {
                                    ap_id: ap.id.0.clone(),
                                    ssid: ap.ssid.clone(),
                                    password: None,
                                });
                            }
                        }
                    }
                } else if self.active == Tab::Bluetooth {
                    if let Some(bt) = &self.bluetooth {
                        if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                            if dev.connected {
                                let _ = action_tx.send(Action::BluetoothDisconnect(dev.id.clone()));
                            } else {
                                let _ = action_tx.send(Action::BluetoothConnect(dev.id.clone()));
                            }
                        }
                    }
                } else if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            if cat == crate::backend::audio::AudioCategory::AppStream {
                                let _ = action_tx.send(Action::AudioToggleMute(node.id));
                            } else {
                                let _ = action_tx.send(Action::AudioSetDefault(node.id));
                            }
                        }
                    }
                } else if self.active == Tab::Displays {
                    if let Some(snap) = &self.displays {
                        if let Some(d) = snap.displays.get(self.display_selected) {
                            if let Some(mode) = d.supported_modes.get(self.display_res_selected) {
                                let _ = action_tx.send(Action::DisplaySetResolution {
                                    display: d.name.clone(),
                                    mode: format!("{}x{}", mode.width, mode.height),
                                    rate: Some(mode.rate),
                                });
                            } else {
                                let _ = action_tx.send(Action::DisplaySetPrimary(d.name.clone()));
                            }
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::NetworkRescan | Action::NetworkToggleRadio | Action::NetworkConnect { .. } => {
                let _ = action_tx.send(action);
            }
            Action::NetworkDisconnect(_) => {
                if let Some(net) = &self.network {
                    if let Some(dev) = &net.wifi_device {
                        let _ = action_tx.send(Action::NetworkDisconnect(dev.id.clone()));
                    }
                }
            }
            Action::NetworkForget(_) => {
                if let Some(net) = &self.network {
                    if let Some(ap) = net.access_points.get(self.network_selected) {
                        if let Some(saved_path) = &ap.saved_conn_path {
                            let _ = action_tx.send(Action::NetworkForget(saved_path.clone()));
                        }
                    }
                }
            }
            Action::NetworkModalChar(_) | Action::NetworkModalBackspace => {}
            Action::BluetoothRescan
            | Action::BluetoothToggleRadio
            | Action::BluetoothConnect(_)
            | Action::BluetoothDisconnect(_) => {
                let _ = action_tx.send(action);
            }
            Action::BluetoothPair(_) => {
                if let Some(bt) = &self.bluetooth {
                    if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                        let _ = action_tx.send(Action::BluetoothPair(dev.id.clone()));
                    }
                }
            }
            Action::BluetoothForget(_) => {
                if let Some(bt) = &self.bluetooth {
                    if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                        let _ = action_tx.send(Action::BluetoothForget(dev.id.clone()));
                    }
                }
            }
            Action::BluetoothToggleBlock(_) => {
                if let Some(bt) = &self.bluetooth {
                    if let Some(dev) = bt.devices.get(self.bluetooth_selected) {
                        let _ = action_tx.send(Action::BluetoothToggleBlock(dev.id.clone()));
                    }
                }
            }
            Action::AudioSelectCategory(cat_idx) => {
                if cat_idx == 99 {
                    self.audio_category = (self.audio_category + 1) % 3;
                } else if cat_idx == 98 {
                    self.audio_category = (self.audio_category + 2) % 3;
                } else {
                    self.audio_category = cat_idx.min(2);
                }
                self.audio_selected = 0;
            }
            Action::AudioSetVolume { .. }
            | Action::AudioVolumeUp(_, _)
            | Action::AudioVolumeDown(_, _)
            | Action::AudioToggleMute(_)
            | Action::AudioSetDefault(_) => {
                let _ = action_tx.send(action);
            }
            Action::VolumeUp => {
                if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            let _ = action_tx.send(Action::AudioVolumeUp(node.id, 0.05));
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::VolumeDown => {
                if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            let _ = action_tx.send(Action::AudioVolumeDown(node.id, 0.05));
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::ToggleMute => {
                if self.active == Tab::Audio {
                    if let Some(audio) = &self.audio {
                        let cat = match self.audio_category {
                            0 => crate::backend::audio::AudioCategory::Sink,
                            1 => crate::backend::audio::AudioCategory::AppStream,
                            _ => crate::backend::audio::AudioCategory::Source,
                        };
                        if let Some(node) = audio.nodes_for_category(cat).get(self.audio_selected) {
                            let _ = action_tx.send(Action::AudioToggleMute(node.id));
                        }
                    }
                } else {
                    let _ = action_tx.send(action);
                }
            }
            Action::DisplaySetLayout(_)
            | Action::DisplaySetResolution { .. }
            | Action::DisplaySetPrimary(_) => {
                let _ = action_tx.send(action);
            }
            Action::Refresh
            | Action::CheckUpdates
            | Action::BrightnessUp
            | Action::BrightnessDown
            | Action::KbdBrightnessUp
            | Action::KbdBrightnessDown
            | Action::ToggleAirplaneMode
            | Action::CyclePowerProfile => {
                let _ = action_tx.send(action);
            }
            Action::Redraw => {}
            Action::StorageMountToggleSelected => self.storage_mount_toggle(action_tx),
            Action::StorageEjectSelected => self.storage_eject_selected(action_tx),
            Action::StorageFormatOpen => self.storage_format_open(),
            Action::StorageFlasherOpen => self.storage_flasher_open(),
            Action::StorageOpenAnalyzer(path) => match path {
                Some(p) => {
                    self.storage_analyzer = Some(DiskAnalyzerState::opening(p.clone()));
                    let _ = action_tx.send(Action::StorageAnalyzerScan(p));
                }
                None => self.storage_analyzer_open_selected(action_tx),
            },
            Action::StorageMultibootPrepareOpen => self.storage_multiboot_prepare_open(action_tx),
            Action::StorageMultibootIsoManagerOpen => {
                self.storage_multiboot_iso_manager_open(action_tx)
            }
            Action::StorageMount(_)
            | Action::StorageUnmount(_)
            | Action::StorageEject(_)
            | Action::StorageRefresh
            | Action::StorageFormat { .. }
            | Action::StorageChecksumIso(_)
            | Action::StorageFlashIso { .. }
            | Action::StorageFlashCancel { .. }
            | Action::StorageMultibootPrepare { .. }
            | Action::StorageMultibootListIsos { .. }
            | Action::StorageMultibootAddIso { .. }
            | Action::StorageMultibootRemoveIso { .. } => {
                let _ = action_tx.send(action);
            }

            Action::StorageModalChar(_)
            | Action::StorageModalBackspace
            | Action::StorageModalDelete
            | Action::StorageModalOpenPicker => {}

            Action::StorageAnalyzerDrillDown
            | Action::StorageAnalyzerGoUp
            | Action::StorageAnalyzerRescan
            | Action::StorageAnalyzerClose => {}
            Action::StorageAnalyzerScan(_) => {
                let _ = action_tx.send(action);
            }
        }
    }
}
