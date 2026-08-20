//! Sistema de internacionalização (i18n) em tempo de compilação para o HAL-9001.
//!
//! Catálogo tipado e estático em Rust puro, garantindo zero overhead de runtime
//! e zero dependências de arquivos externos (.po/.mo), preservando o binário
//! 100% autocontido.

/// Idiomas suportados pelo HAL-9001.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    /// Português do Brasil (Padrão)
    #[default]
    PtBr,
    /// English (US)
    EnUs,
    /// Español
    EsEs,
}

impl Language {
    /// Parseia uma string de idioma (ex: `"pt-BR"`, `"pt"`, `"en-US"`, `"en"`, `"es"`).
    pub fn parse(raw: &str) -> Option<Self> {
        let clean = raw.trim().to_lowercase().replace('_', "-");
        if clean.starts_with("pt") {
            Some(Language::PtBr)
        } else if clean.starts_with("en") {
            Some(Language::EnUs)
        } else if clean.starts_with("es") {
            Some(Language::EsEs)
        } else {
            None
        }
    }

    /// Detecta o idioma preferido a partir das variáveis de ambiente ($LC_ALL, $LC_MESSAGES, $LANG).
    pub fn detect() -> Self {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(val) = std::env::var(var) {
                if let Some(lang) = Self::parse(&val) {
                    return lang;
                }
            }
        }
        Language::PtBr
    }

    /// Código ISO padrão do idioma (`"pt-BR"`, `"en-US"`, `"es-ES"`).
    pub const fn code(self) -> &'static str {
        match self {
            Language::PtBr => "pt-BR",
            Language::EnUs => "en-US",
            Language::EsEs => "es-ES",
        }
    }

    /// Nome amigável do idioma.
    pub const fn name(self) -> &'static str {
        match self {
            Language::PtBr => "Português (Brasil)",
            Language::EnUs => "English (US)",
            Language::EsEs => "Español",
        }
    }

    /// Retorna a tabela de mensagens estáticas traduzidas.
    pub const fn messages(self) -> &'static Messages {
        match self {
            Language::PtBr => &MESSAGES_PT_BR,
            Language::EnUs => &MESSAGES_EN_US,
            Language::EsEs => &MESSAGES_ES_ES,
        }
    }
}

/// Tabela completa de mensagens da interface do HAL-9001.
#[derive(Debug, Clone, Copy)]
pub struct Messages {
    // Cabeçalho e Título
    pub app_title_suffix: &'static str,
    pub splash_title: &'static str,
    pub splash_loading: &'static str,
    pub splash_welcome: &'static str,

    // Abas
    pub tab_overview: &'static str,
    pub tab_network: &'static str,
    pub tab_bluetooth: &'static str,
    pub tab_storage: &'static str,
    pub tab_power: &'static str,
    pub tab_updates: &'static str,
    pub tab_files: &'static str,
    pub tab_terminal: &'static str,

    // Seções do Overview
    pub sec_compute: &'static str,
    pub sec_system: &'static str,
    pub sec_peripherals: &'static str,
    pub sec_palette: &'static str,

    // Rótulos de Métricas
    pub label_host: &'static str,
    pub label_os: &'static str,
    pub label_kernel: &'static str,
    pub label_uptime: &'static str,
    pub label_shell: &'static str,
    pub label_packages: &'static str,
    pub label_battery: &'static str,
    pub label_brightness: &'static str,
    pub label_volume: &'static str,
    pub label_ram: &'static str,
    pub label_swap: &'static str,
    pub label_disk_root: &'static str,
    pub label_cpu_usage: &'static str,
    pub label_power_profile: &'static str,

    // Modo Detalhado
    pub label_board: &'static str,
    pub label_bios: &'static str,
    pub label_gpu: &'static str,
    pub label_cpu_cores: &'static str,
    pub label_cpu_freq: &'static str,
    pub label_temperature: &'static str,
    pub label_desktop: &'static str,

    // Perfis de Energia
    pub profile_power_saver: &'static str,
    pub profile_balanced: &'static str,
    pub profile_performance: &'static str,

    // Bateria
    pub battery_charging: &'static str,
    pub battery_discharging: &'static str,
    pub battery_full: &'static str,
    pub battery_not_charging: &'static str,
    pub battery_unknown: &'static str,

    // Rodapé / Dicas de Teclado
    pub hint_mode_detailed: &'static str,
    pub hint_mode_normal: &'static str,
    pub hint_profile: &'static str,
    pub hint_brightness: &'static str,
    pub hint_volume: &'static str,
    pub hint_mute: &'static str,
    pub hint_config: &'static str,
    pub hint_quit: &'static str,
    pub hint_help: &'static str,

    // Notificações (Toasts)
    pub toast_profile_prefix: &'static str,
    pub toast_brightness_prefix: &'static str,
    pub toast_volume_prefix: &'static str,
    pub toast_muted: &'static str,
    pub toast_unmuted: &'static str,
    pub toast_unavailable: &'static str,
}

// ---------------------------------------------------------------------------
// 🇧🇷 Português do Brasil (Padrão)
// ---------------------------------------------------------------------------
pub static MESSAGES_PT_BR: Messages = Messages {
    app_title_suffix: "Assistente de Sistema",
    splash_title: "HAL-9001 · Assistente de Sistema",
    splash_loading: "CARREGANDO",
    splash_welcome: "Bem-vindo",

    tab_overview: "Overview",
    tab_network: "Rede",
    tab_bluetooth: "Bluetooth",
    tab_storage: "Discos",
    tab_power: "Energia",
    tab_updates: "Atualizações",
    tab_files: "Arquivos",
    tab_terminal: "Terminal",

    sec_compute: "Available Compute / Hardware",
    sec_system: "System & Platform",
    sec_peripherals: "Peripherals & Power",
    sec_palette: "Color Palette",

    label_host: "Host",
    label_os: "SO",
    label_kernel: "Kernel",
    label_uptime: "Uptime",
    label_shell: "Shell",
    label_packages: "Pacotes",
    label_battery: "Bateria",
    label_brightness: "Brilho",
    label_volume: "Volume",
    label_ram: "RAM",
    label_swap: "Swap",
    label_disk_root: "Disco (/)",
    label_cpu_usage: "Uso CPU",
    label_power_profile: "Perfil",

    label_board: "Placa-Mãe",
    label_bios: "BIOS",
    label_gpu: "GPU",
    label_cpu_cores: "CPU Núcleos",
    label_cpu_freq: "Frequência",
    label_temperature: "Temperatura",
    label_desktop: "Desktop/WM",

    profile_power_saver: "Economia",
    profile_balanced: "Equilibrado",
    profile_performance: "Desempenho",

    battery_charging: "Carregando",
    battery_discharging: "Descarregando",
    battery_full: "Completa",
    battery_not_charging: "Sem carga",
    battery_unknown: "Desconhecido",

    hint_mode_detailed: "[.] Detalhes",
    hint_mode_normal: "[.] Normal",
    hint_profile: "[p] Perfil",
    hint_brightness: "[b/B] Brilho",
    hint_volume: "[v/V] Volume",
    hint_mute: "[m] Mudo",
    hint_config: "[c] Config",
    hint_quit: "[q] Sair",
    hint_help: "[?] Ajuda",

    toast_profile_prefix: "Perfil de Energia",
    toast_brightness_prefix: "Brilho",
    toast_volume_prefix: "Volume",
    toast_muted: "Áudio mudo",
    toast_unmuted: "Áudio ativo",
    toast_unavailable: "indisponível",
};

// ---------------------------------------------------------------------------
// 🇺🇸 English (US)
// ---------------------------------------------------------------------------
pub static MESSAGES_EN_US: Messages = Messages {
    app_title_suffix: "System Assistant",
    splash_title: "HAL-9001 · System Assistant",
    splash_loading: "LOADING",
    splash_welcome: "Welcome",

    tab_overview: "Overview",
    tab_network: "Network",
    tab_bluetooth: "Bluetooth",
    tab_storage: "Storage",
    tab_power: "Power",
    tab_updates: "Updates",
    tab_files: "Files",
    tab_terminal: "Terminal",

    sec_compute: "Available Compute / Hardware",
    sec_system: "System & Platform",
    sec_peripherals: "Peripherals & Power",
    sec_palette: "Color Palette",

    label_host: "Host",
    label_os: "OS",
    label_kernel: "Kernel",
    label_uptime: "Uptime",
    label_shell: "Shell",
    label_packages: "Packages",
    label_battery: "Battery",
    label_brightness: "Brightness",
    label_volume: "Volume",
    label_ram: "RAM",
    label_swap: "Swap",
    label_disk_root: "Disk (/)",
    label_cpu_usage: "CPU Load",
    label_power_profile: "Profile",

    label_board: "Motherboard",
    label_bios: "BIOS",
    label_gpu: "GPU",
    label_cpu_cores: "CPU Cores",
    label_cpu_freq: "Frequency",
    label_temperature: "Temperature",
    label_desktop: "Desktop/WM",

    profile_power_saver: "Power Saver",
    profile_balanced: "Balanced",
    profile_performance: "Performance",

    battery_charging: "Charging",
    battery_discharging: "Discharging",
    battery_full: "Full",
    battery_not_charging: "Not Charging",
    battery_unknown: "Unknown",

    hint_mode_detailed: "[.] Details",
    hint_mode_normal: "[.] Normal",
    hint_profile: "[p] Profile",
    hint_brightness: "[b/B] Brightness",
    hint_volume: "[v/V] Volume",
    hint_mute: "[m] Mute",
    hint_config: "[c] Config",
    hint_quit: "[q] Quit",
    hint_help: "[?] Help",

    toast_profile_prefix: "Power Profile",
    toast_brightness_prefix: "Brightness",
    toast_volume_prefix: "Volume",
    toast_muted: "Audio muted",
    toast_unmuted: "Audio unmuted",
    toast_unavailable: "unavailable",
};

// ---------------------------------------------------------------------------
// 🇪🇸 Español
// ---------------------------------------------------------------------------
pub static MESSAGES_ES_ES: Messages = Messages {
    app_title_suffix: "Asistente de Sistema",
    splash_title: "HAL-9001 · Asistente de Sistema",
    splash_loading: "CARGANDO",
    splash_welcome: "Bienvenido",

    tab_overview: "Visión General",
    tab_network: "Red",
    tab_bluetooth: "Bluetooth",
    tab_storage: "Discos",
    tab_power: "Energía",
    tab_updates: "Actualizaciones",
    tab_files: "Archivos",
    tab_terminal: "Terminal",

    sec_compute: "Disponibilidade de Hardware",
    sec_system: "Sistema y Plataforma",
    sec_peripherals: "Periféricos y Energía",
    sec_palette: "Paleta de Colores",

    label_host: "Host",
    label_os: "SO",
    label_kernel: "Kernel",
    label_uptime: "Tiempo activo",
    label_shell: "Shell",
    label_packages: "Paquetes",
    label_battery: "Batería",
    label_brightness: "Brillo",
    label_volume: "Volumen",
    label_ram: "RAM",
    label_swap: "Swap",
    label_disk_root: "Disco (/)",
    label_cpu_usage: "Uso CPU",
    label_power_profile: "Perfil",

    label_board: "Placa Base",
    label_bios: "BIOS",
    label_gpu: "GPU",
    label_cpu_cores: "Núcleos CPU",
    label_cpu_freq: "Frecuencia",
    label_temperature: "Temperatura",
    label_desktop: "Entorno/WM",

    profile_power_saver: "Ahorro",
    profile_balanced: "Equilibrado",
    profile_performance: "Rendimiento",

    battery_charging: "Cargando",
    battery_discharging: "Descargando",
    battery_full: "Completa",
    battery_not_charging: "Sin carga",
    battery_unknown: "Desconocido",

    hint_mode_detailed: "[.] Detalles",
    hint_mode_normal: "[.] Normal",
    hint_profile: "[p] Perfil",
    hint_brightness: "[b/B] Brillo",
    hint_volume: "[v/V] Volumen",
    hint_mute: "[m] Silenciar",
    hint_config: "[c] Config",
    hint_quit: "[q] Salir",
    hint_help: "[?] Ayuda",

    toast_profile_prefix: "Perfil de Energía",
    toast_brightness_prefix: "Brillo",
    toast_volume_prefix: "Volumen",
    toast_muted: "Audio silenciado",
    toast_unmuted: "Audio activo",
    toast_unavailable: "no disponible",
};
