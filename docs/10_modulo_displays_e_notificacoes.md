# 10 — Módulo de Monitores/Displays & Barramento Global de Notificações (HAL-9001)

> **HAL-9001 — Especificação Arquitetural e Plano de Execução Rigoroso**  
> **Subsistema de Displays / Monitores:** DRM/KMS, Sysfs (`/sys/class/drm`), X11 (`xrandr`), Wayland (`wlr-randr` / D-Bus Mutter / KScreen) e TUI Ratatui.  
> **Subsistema de Notificações:** Barramento Global Reativo de Toasts para Hardware & Sistema (Monitores, Storage/USB, Bluetooth, Rede, Bateria, Áudio e Alertas).  
> **Regra de Ouro:** Hotplug automático com auto-expansão para novos monitores e emissão imediata de toast.

---

## 0. Sumário Executivo & Decisões de Engenharia

O HAL-9001 evolui para gerenciar o ecossistema gráfico de monitores e servir como a central de telemetria e controle de hardware do host Linux. Quando o capitão conecta um monitor externo ao notebook, o sistema deve reagir em milissegundos, configurar a topologia de visualização sem fricção e fornecer feedback visual claro e imediato.

Este documento unifica dois subsistemas interdependentes e de alta prioridade:
1. **Módulo de Displays & Monitores:** Detecção via DRM/KMS (`/sys/class/drm`), leitura pura de EDID, driver multi-servidor (X11 `xrandr` e Wayland), aplicação da **Regra de Ouro de Auto-Expansão**, configuração de resoluções, taxas de atualização (Hz), rotação, espelhamento, monitor primário e renderização espacial com diagrama ASCII 2D na TUI.
2. **Barramento Global de Toasts e Notificações de Hardware:** Barramento unificado com fila prioritária, severidades visuais (`Info`, `Success`, `Warning`, `Error`), temporização configurável e roteamento para todos os eventos de hardware do sistema (monitores, pendrives/USB, dispositivos Bluetooth, conexões Wi-Fi/Ethernet, carregador/bateria, áudio e erros de sistema).

### 0.1 Regras Inegociáveis de Arquitetura

1. **ZERO Novas Dependências no `Cargo.toml`:** A implementação utiliza exclusivamente a stack já homologada: `tokio` (runtime assíncrono, process, fs), `ratatui` (renderização TUI e layout), `serde` / `toml` (configurações), `anyhow` / `thiserror` (erros), `crossterm` (input e terminal), `zbus` (D-Bus para UDisks2, BlueZ, NetworkManager, UPower, Mutter) e `sysinfo`. Nenhuma crate externa de DRM, X11 ou EDID será adicionada.
2. **Zero Emojis Policy:** Proibido o uso de emojis hardcoded na TUI e no código-fonte. Todo feedback visual utiliza glifos Nerd Font quando `config.ui.icons == true`, com fallback rígido para badges ASCII padronizados: `[MONITOR]`, `[EXPAND]`, `[ESPELHO]`, `[INTERNO]`, `[EXTERNO]`, `[DISCO]`, `[USB]`, `[BT]`, `[WIFI]`, `[BAT]`, `[OK]`, `[AVISO]`, `[ERRO]`.
3. **Regra de Ouro de Automação (Hotplug):** Ao detectar a transição de um conector externo para `connected`:
   - Se o monitor não estava ativo, aplica imediatamente a resolução ótima recomendada (preferred mode) posicionada à direita da tela interna: `xrandr --output <ext> --auto --right-of <internal>`.
   - Dispara um toast de sucesso no barramento global: `[MONITOR] [EXPAND] Monitor HDMI-1 conectado — modo Expandir ativado!`.
   - Ao desconectar, desliga a saída (`--off`), reorganiza o canvas virtual e emite toast informativo.
4. **Resiliência e Safety Revert (15 segundos):** Alterações manuais arriscadas de resolução ou layout exibem um modal com contagem regressiva de 15 segundos na TUI. Se o capitão não confirmar com `Enter` (ex.: se a tela externa ficar preta ou fora de frequência), o HAL-9001 reverte automaticamente para a configuração anterior.
5. **Fluxo Unidirecional e Snapshots Imutáveis:** O backend de displays roda em task Tokio dedicada (`src/backend/display.rs`), publicando `AppEvent::Display(Box<DisplaySnapshot>)`. A TUI renderiza o estado de forma pura sem travar o loop de eventos.

---

## 1. Visão Geral da Arquitetura & Fluxo de Dados

### 1.1 Diagrama de Arquitetura de Displays e Notificações

```
  ┌────────────────────────────────────────────────────────────────────────┐
  │                    HARDWARE & KERNEL LINUX                             │
  │  /sys/class/drm/card*-*    /dev/input    BlueZ D-Bus    UDisks2 D-Bus  │
  │  DRM uevents / sysfs       xrandr        NM D-Bus       UPower D-Bus   │
  └───────────────┬───────────────────────────────┬────────────────────────┘
                  │                               │
                  ▼                               ▼
  ┌───────────────────────────────┐ ┌──────────────────────────────────────┐
  │     backend::display::run     │ │  Backends de Hardware (Storage, etc) │
  │  - Polling sysfs / DRM (500ms)│ │  - Bluetooth, Network, Audio, Power  │
  │  - Parser EDID (Rust puro)    │ └──────────────────┬───────────────────┘
  │  - Driver xrandr / Wayland    │                    │
  │  - Automação Hotplug (Gold)   │                    │
  └───────────────┬───────────────┘                    │
                  │ AppEvent::Display                  │ AppEvent::Toast
                  │ AppEvent::Toast                    │ AppEvent::*
                  ▼                                    ▼
  ┌────────────────────────────────────────────────────────────────────────┐
  │                             App (src/app.rs)                           │
  │  - app.display: Option<Box<DisplaySnapshot>>                           │
  │  - app.notifications: NotificationManager (Fila de Toasts & Histórico) │
  │  - app.display_safety_timer: Option<SafetyRevertState>                 │
  └───────────────────────────────┬────────────────────────────────────────┘
                                  │
                                  ▼
  ┌────────────────────────────────────────────────────────────────────────┐
  │                          UI (Ratatui - TUI)                            │
  │  - draw_content: Overview (Aba 1) / Painel Displays (Sub-aba / Modal)  │
  │  - draw_display_canvas: Diagrama ASCII 2D do Espaço Virtual            │
  │  - draw_toast_overlay: Stack Flutuante de Notificações com Badges      │
  └────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Comparativo de Drivers por Ambiente de Janelas

| Servidor Gráfico | Mecanismo de Consulta | Mecanismo de Aplicação | Detecção de Hotplug |
|---|---|---|---|
| **X11 (Padrão)** | `xrandr --query --verbose` + Sysfs | `xrandr --output ...` | `/sys/class/drm/*/status` + polling 500ms |
| **Wayland (wlroots/Sway/Hyprland)** | `wlr-randr --json` ou IPC | `wlr-randr --output ...` | Udev / Sysfs DRM |
| **Wayland (GNOME/Mutter)** | D-Bus `org.gnome.Mutter.DisplayConfig` | D-Bus `ApplyMonitorsConfig` | Sinal D-Bus `MonitorsChanged` |
| **Wayland (KDE Plasma)** | `kscreen-doctor -j` | `kscreen-doctor output.<N>...` | Sysfs DRM + D-Bus KScreen |
| **Console Linux / DRM Puro** | Leitura direta `/sys/class/drm` | Somente leitura / telemetria | Inotify em `/sys/class/drm` |

---

## 2. Subsistema de Monitores: Detecção, EDID e Drivers

### 2.1 Inspeção do DRM no Kernel (`/sys/class/drm`)

O Linux expõe conectores de vídeo em `/sys/class/drm/card<N>-<Connector>`. Exemplos típicos:
- `card0-eDP-1`: Display interno integrado (LVDS/eDP de notebooks).
- `card0-HDMI-A-1` ou `card0-HDMI-A-2`: Saídas externas HDMI.
- `card0-DP-1` / `card0-DP-2`: DisplayPort / USB-C DisplayPort Alt-Mode.
- `card0-VGA-1`: Saída analógica legada.

Estrutura de arquivos lidos em cada conector:
- `/sys/class/drm/card0-HDMI-A-1/status`: Retorna `connected` ou `disconnected`.
- `/sys/class/drm/card0-HDMI-A-1/enabled`: Retorna `enabled` ou `disabled`.
- `/sys/class/drm/card0-HDMI-A-1/modes`: Lista de resoluções suportadas pelo conector (ex.: `1920x1080\n1280x720\n...`).
- `/sys/class/drm/card0-HDMI-A-1/edid`: Bloco binário de 128 bytes (ou múltiplos) contendo fabricante, modelo, taxas e timing nativo.

### 2.2 Parser Puro de EDID em Rust (Sem Dependências Externas)

O cabeçalho EDID v1.3/v1.4 é estruturado em blocos de 128 bytes. Implementação em Rust puro sem crates adicionais:

```rust
//! Parser puro de bloco binário EDID (128 bytes) para identificação de monitor.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdidInfo {
    pub manufacturer_code: String,
    pub product_code: u16,
    pub model_name: String,
    pub serial_number: Option<String>,
    pub width_cm: u8,
    pub height_cm: u8,
    pub preferred_width: u32,
    pub preferred_height: u32,
    pub preferred_refresh_hz: u32,
}

pub fn parse_edid(bytes: &[u8]) -> Option<EdidInfo> {
    if bytes.len() < 128 {
        return None;
    }
    // Verifica cabeçalho fixo EDID: 00 FF FF FF FF FF FF 00
    const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
    if bytes[0..8] != HEADER {
        return None;
    }

    // Fabricante: 3 letras comprimidas em 2 bytes (bytes 8 e 9)
    let mfg_raw = u16::from_be_bytes([bytes[8], bytes[9]]);
    let c1 = (((mfg_raw >> 10) & 0x1F) as u8 + b'@') as char;
    let c2 = (((mfg_raw >> 5) & 0x1F) as u8 + b'@') as char;
    let c3 = ((mfg_raw & 0x1F) as u8 + b'@') as char;
    let manufacturer_code = format!("{c1}{c2}{c3}");

    let product_code = u16::from_le_bytes([bytes[10], bytes[11]]);
    let width_cm = bytes[21];
    let height_cm = bytes[22];

    // Detalhamento de Timings Descritores (4 blocos de 18 bytes a partir do offset 54)
    let mut model_name = String::new();
    let mut serial_number = None;
    let mut preferred_width = 0;
    let mut preferred_height = 0;
    let mut preferred_refresh_hz = 60;

    for i in 0..4 {
        let offset = 54 + i * 18;
        let desc = &bytes[offset..offset + 18];

        if desc[0] == 0 && desc[1] == 0 {
            // Descritor de Display
            let tag = desc[3];
            if tag == 0xFC {
                // Monitor Name (ASCII)
                let text = String::from_utf8_lossy(&desc[5..18]);
                model_name = text.trim_matches(|c: char| c == '\n' || c == '\r' || c == '\0' || c.is_whitespace()).to_string();
            } else if tag == 0xFF {
                // Monitor Serial Number
                let text = String::from_utf8_lossy(&desc[5..18]);
                serial_number = Some(text.trim().to_string());
            }
        } else if i == 0 {
            // Primeiro bloco: Timing Detalhado Preferido (Preferred Timing Descriptor)
            let pixel_clock = u16::from_le_bytes([desc[0], desc[1]]) as u32 * 10_000;
            let hactive = desc[2] as u32 | (((desc[4] >> 4) as u32) << 8);
            let hblank = desc[3] as u32 | (((desc[4] & 0x0F) as u32) << 8);
            let vactive = desc[5] as u32 | (((desc[7] >> 4) as u32) << 8);
            let vblank = desc[6] as u32 | (((desc[7] & 0x0F) as u32) << 8);

            preferred_width = hactive;
            preferred_height = vactive;

            let htotal = hactive + hblank;
            let vtotal = vactive + vblank;
            if htotal > 0 && vtotal > 0 && pixel_clock > 0 {
                preferred_refresh_hz = (pixel_clock as f64 / (htotal as f64 * vtotal as f64)).round() as u32;
            }
        }
    }

    if model_name.is_empty() {
        model_name = format!("{manufacturer_code}-{product_code:04X}");
    }

    Some(EdidInfo {
        manufacturer_code,
        product_code,
        model_name,
        serial_number,
        width_cm,
        height_cm,
        preferred_width,
        preferred_height,
        preferred_refresh_hz,
    })
}
```

---

## 3. Modelos de Dados em Rust (`src/backend/display.rs`)

### 3.1 Entidades do Subsistema de Vídeo

```rust
use serde::{Deserialize, Serialize};

/// Servidor gráfico identificado no host
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayServerKind {
    X11,
    WaylandGnome,
    WaylandWlroots,
    WaylandKde,
    DrmSysfsOnly,
    Unknown,
}

impl DisplayServerKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::X11 => "[X11/RandR]",
            Self::WaylandGnome => "[WAYLAND/GNOME]",
            Self::WaylandWlroots => "[WAYLAND/WLR]",
            Self::WaylandKde => "[WAYLAND/KDE]",
            Self::DrmSysfsOnly => "[DRM/KMS]",
            Self::Unknown => "[DISPLAY/UNK]",
        }
    }
}

/// Modos de disposição de múltiplos monitores
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DisplayLayoutPreset {
    #[default]
    ExtendRight,   // Monitor externo à direita do notebook
    ExtendLeft,    // Monitor externo à esquerda do notebook
    Mirror,        // Espelhar mesma imagem em todas as telas
    ExternalOnly,  // Desliga tela do notebook, usa apenas monitor externo
    InternalOnly,  // Desliga monitor externo, usa apenas notebook
    Custom,        // Disposição manual de coordenadas (X, Y)
}

impl DisplayLayoutPreset {
    pub const ALL: [DisplayLayoutPreset; 5] = [
        DisplayLayoutPreset::ExtendRight,
        DisplayLayoutPreset::ExtendLeft,
        DisplayLayoutPreset::Mirror,
        DisplayLayoutPreset::ExternalOnly,
        DisplayLayoutPreset::InternalOnly,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ExtendRight => "Expandir à Direita",
            Self::ExtendLeft => "Expandir à Esquerda",
            Self::Mirror => "Espelhar (Duplicar)",
            Self::ExternalOnly => "Apenas Monitor Externo",
            Self::InternalOnly => "Apenas Tela do Notebook",
            Self::Custom => "Personalizado",
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::ExtendRight => "[EXPAND-DIR]",
            Self::ExtendLeft => "[EXPAND-ESQ]",
            Self::Mirror => "[ESPELHO]",
            Self::ExternalOnly => "[APENAS-EXT]",
            Self::InternalOnly => "[APENAS-INT]",
            Self::Custom => "[CUSTOM]",
        }
    }
}

/// Rotação da saída de vídeo
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DisplayRotation {
    #[default]
    Normal,     // 0 graus
    Left90,     // 90 graus anti-horário (retrato)
    Right270,   // 90 graus horário (retrato invertido)
    Inverted180,// 180 graus (cabeça para baixo)
}

impl DisplayRotation {
    pub const ALL: [DisplayRotation; 4] = [
        DisplayRotation::Normal,
        DisplayRotation::Left90,
        DisplayRotation::Right270,
        DisplayRotation::Inverted180,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal (0°)",
            Self::Left90 => "Girar Esquerda (90°)",
            Self::Right270 => "Girar Direita (270°)",
            Self::Inverted180 => "Invertido (180°)",
        }
    }

    pub fn xrandr_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Left90 => "left",
            Self::Right270 => "right",
            Self::Inverted180 => "inverted",
        }
    }
}

/// Resolução e taxa de atualização suportada
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub refresh_rate: f32, // Hz (ex.: 59.94, 60.00, 144.00)
    pub is_current: bool,
    pub is_preferred: bool,
}

impl DisplayMode {
    pub fn label(&self) -> String {
        format!("{}x{} @ {:.1}Hz", self.width, self.height, self.refresh_rate)
    }
}

/// Informações completas de um monitor/conector
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonitorDevice {
    pub connector: String,          // Ex.: "eDP-1", "HDMI-1", "DP-1"
    pub is_internal: bool,          // True se eDP, LVDS ou DSI
    pub is_connected: bool,         // Cabo plugado no conector
    pub is_active: bool,            // Exibindo imagem no momento
    pub is_primary: bool,           // Monitor primário do sistema
    pub model_name: String,         // Obtido via EDID ou driver (ex: "LG 29UM69G")
    pub manufacturer: String,      // Ex: "GSM", "DEL", "SEC"
    pub serial_number: Option<String>,
    pub current_mode: Option<DisplayMode>,
    pub supported_modes: Vec<DisplayMode>,
    pub position: (i32, i32),       // Posição X, Y no espaço virtual
    pub rotation: DisplayRotation,
    pub scale: f32,                 // Escala (1.0 = 100%, 1.25, 1.5, 2.0)
    pub brightness_pct: Option<u8>, // Brilho lido via sysfs/ddcutil (0..100)
}

impl MonitorDevice {
    pub fn display_title(&self) -> String {
        if self.model_name.is_empty() {
            self.connector.clone()
        } else {
            format!("{} ({})", self.model_name, self.connector)
        }
    }

    pub fn type_badge(&self) -> &'static str {
        if self.is_internal {
            "[INTERNO]"
        } else {
            "[EXTERNO]"
        }
    }
}

/// Snapshot consolidado do estado de vídeo
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplaySnapshot {
    pub server: DisplayServerKind,
    pub monitors: Vec<MonitorDevice>,
    pub virtual_canvas_width: u32,
    pub virtual_canvas_height: u32,
    pub current_preset: DisplayLayoutPreset,
    pub last_hotplug_event: Option<String>,
}

impl DisplaySnapshot {
    pub fn empty() -> Self {
        Self {
            server: DisplayServerKind::Unknown,
            monitors: Vec::new(),
            virtual_canvas_width: 1920,
            virtual_canvas_height: 1080,
            current_preset: DisplayLayoutPreset::ExtendRight,
            last_hotplug_event: None,
        }
    }

    pub fn internal_monitor(&self) -> Option<&MonitorDevice> {
        self.monitors.iter().find(|m| m.is_internal && m.is_connected)
    }

    pub fn external_monitors(&self) -> Vec<&MonitorDevice> {
        self.monitors.iter().filter(|m| !m.is_internal && m.is_connected).collect()
    }
}
```

---

## 4. Regra de Ouro da Automação de Hotplug

### 4.1 Máquina de Estados da Detecção e Auto-Aplicação

```
           [Monitor Desconectado]
                     │
                     │ Usuário pluga o cabo HDMI / DP
                     ▼
           [Kernel DRM emite UEvent / Sysfs status muda para 'connected']
                     │
                     ▼
           [Backend detecta novo conector conectado]
                     │
                     ├── O monitor já estava ativo?
                     │     ├── SIM ──▶ Nenhuma alteração de layout necessária
                     │     │
                     │     └── NÃO ──▶ [APLICA REGRA DE OURO]
                     │                   │
                     │                   ├─ 1. Identifica tela interna (eDP-1)
                     │                   ├─ 2. Identifica modo preferido do monitor externo
                     │                   ├─ 3. Executa: xrandr --output <EXT> --auto --right-of <INT>
                     │                   ├─ 4. Emite AppEvent::Toast (Sucesso)
                     │                   └─ 5. Atualiza DisplaySnapshot
                     ▼
           [UI renderiza novo diagrama com 2 telas no espaço virtual]
```

### 4.2 Lógica Pura de Execução do Hotplug (`handle_hotplug_transition`)

```rust
/// Avalia a transição entre dois snapshots de conectores e decide se deve disparar a Regra de Ouro.
pub fn evaluate_hotplug_transition(
    previous: &[MonitorDevice],
    current: &[MonitorDevice],
) -> Vec<HotplugAction> {
    let mut actions = Vec::new();

    for cur in current {
        let prev = previous.iter().find(|p| p.connector == cur.connector);
        let was_connected = prev.map(|p| p.is_connected).unwrap_or(false);

        // Caso 1: Monitor recém-conectado
        if cur.is_connected && !was_connected {
            if !cur.is_internal {
                actions.push(HotplugAction::AutoExtend {
                    connector: cur.connector.clone(),
                    model: cur.model_name.clone(),
                });
            }
        }
        // Caso 2: Monitor desconectado
        else if !cur.is_connected && was_connected {
            actions.push(HotplugAction::HandleDisconnect {
                connector: cur.connector.clone(),
                model: prev.map(|p| p.model_name.clone()).unwrap_or_default(),
            });
        }
    }

    actions
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotplugAction {
    AutoExtend { connector: String, model: String },
    HandleDisconnect { connector: String, model: String },
}
```

### 4.3 Comandos Gerados para `xrandr`

1. **Auto-Expansão à Direita (Regra de Ouro):**
   ```bash
   xrandr --output eDP-1 --auto --primary --output HDMI-1 --auto --right-of eDP-1
   ```
2. **Auto-Expansão à Esquerda:**
   ```bash
   xrandr --output eDP-1 --auto --primary --output HDMI-1 --auto --left-of eDP-1
   ```
3. **Espelhar / Duplicar (Clone):**
   ```bash
   xrandr --output eDP-1 --auto --output HDMI-1 --auto --same-as eDP-1
   ```
4. **Apenas Monitor Externo:**
   ```bash
   xrandr --output eDP-1 --off --output HDMI-1 --auto --primary
   ```
5. **Apenas Tela do Notebook (Desconectar/Desligar Externo):**
   ```bash
   xrandr --output HDMI-1 --off --output eDP-1 --auto --primary
   ```

---

## 5. Barramento Global de Notificações & Fila de Toasts

### 5.1 Arquitetura do Barramento Centralizado de Hardware

O HAL-9001 unifica a emissão de notificações efêmeras de todos os subsistemas em um barramento reativo de alta fidelidade:

```rust
use std::time::{Duration, Instant};

/// Fonte originadora do evento de hardware
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NotificationSource {
    Display,
    Storage,
    Bluetooth,
    Network,
    Battery,
    Audio,
    System,
}

impl NotificationSource {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Display => "[MONITOR]",
            Self::Storage => "[STORAGE]",
            Self::Bluetooth => "[BT     ]",
            Self::Network => "[REDE   ]",
            Self::Battery => "[ENERGIA]",
            Self::Audio => "[AUDIO  ]",
            Self::System => "[SISTEMA]",
        }
    }
}

/// Nível de severidade visual
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum NotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

/// Item de notificação no barramento global
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationToast {
    pub id: u64,
    pub source: NotificationSource,
    pub severity: NotificationSeverity,
    pub title: String,
    pub message: String,
    pub created_at: Instant,
    pub timeout: Duration,
}

impl NotificationToast {
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() >= self.timeout
    }

    /// Formata a linha visual completa para a TUI
    pub fn format_line(&self) -> String {
        format!("{} {} — {}", self.source.badge(), self.title, self.message)
    }
}
```

### 5.2 Gerenciador de Notificações (`NotificationManager`)

Para evitar que mensagens críticas sejam sobrescritas instantaneamente quando múltiplos eventos ocorrem em cascata (ex.: ao plugar uma dock USB-C com monitor, pendrive e rede simultaneamente), o `NotificationManager` mantém uma fila ordenada e um histórico consultável:

```rust
#[derive(Debug, Clone)]
pub struct NotificationManager {
    pub active_toasts: Vec<NotificationToast>,
    pub history: Vec<NotificationToast>,
    pub max_active: usize,
    pub max_history: usize,
    next_id: u64,
}

impl NotificationManager {
    pub fn new() -> Self {
        Self {
            active_toasts: Vec::new(),
            history: Vec::new(),
            max_active: 3,
            max_history: 50,
            next_id: 1,
        }
    }

    pub fn push(
        &mut self,
        source: NotificationSource,
        severity: NotificationSeverity,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        let timeout = match severity {
            NotificationSeverity::Info => Duration::from_millis(3500),
            NotificationSeverity::Success => Duration::from_millis(4500),
            NotificationSeverity::Warning => Duration::from_millis(6000),
            NotificationSeverity::Error => Duration::from_millis(8000),
        };

        let toast = NotificationToast {
            id: self.next_id,
            source,
            severity,
            title: title.into(),
            message: message.into(),
            created_at: Instant::now(),
            timeout,
        };

        self.next_id += 1;
        self.active_toasts.push(toast.clone());
        if self.active_toasts.len() > self.max_active {
            self.active_toasts.remove(0);
        }

        self.history.push(toast);
        if self.history.len() > self.max_history {
            self.history.remove(0);
        }
    }

    /// Remove notificações expiradas no tick da UI
    pub fn prune_expired(&mut self) {
        self.active_toasts.retain(|t| !t.is_expired());
    }

    /// Retorna a notificação mais recente para a statusline
    pub fn current(&self) -> Option<&NotificationToast> {
        self.active_toasts.last()
    }
}
```

### 5.3 Mapeamento de Eventos de Hardware para Toasts

| Subsistema | Gatilho / Evento | Severidade | Título | Mensagem / Exemplo |
|---|---|---|---|---|
| **Monitores** | Novo monitor externo plugado | `Success` | `[MONITOR] [EXPAND]` | `Monitor HDMI-1 conectado — modo Expandir ativado!` |
| **Monitores** | Monitor externo desconectado | `Info` | `[MONITOR]` | `Monitor HDMI-1 desconectado` |
| **Monitores** | Erro ao aplicar modo de tela | `Error` | `[MONITOR] [ERRO]` | `Falha no xrandr: resolução não suportada pelo display` |
| **Storage/USB** | Pendrive ou disco USB inserido | `Success` | `[STORAGE] [USB]` | `Pendrive USB conectado (/dev/sdb1 — 32GB FAT32)` |
| **Storage/USB** | Dispositivo ejetado com segurança | `Info` | `[STORAGE] [EJECT]` | `Dispositivo /dev/sdb removido com segurança` |
| **Storage/USB** | Gravação de ISO concluída | `Success` | `[FLASHER] [OK]` | `Gravação de 'archlinux.iso' concluída com sucesso!` |
| **Bluetooth** | Dispositivo pareado conectado | `Success` | `[BT] [CONECTADO]` | `soundcore P20i conectado (Bateria: 85%)` |
| **Bluetooth** | Dispositivo desconectado | `Info` | `[BT] [DESCONECT]` | `soundcore P20i desconectado` |
| **Rede/Wi-Fi** | Associação Wi-Fi bem-sucedida | `Success` | `[WIFI] [ONLINE]` | `Conectado em 'Starlink-5G' (IP: 192.168.1.42)` |
| **Rede/Wi-Fi** | Cabo Ethernet plugado / link ativo | `Success` | `[ETH] [ONLINE]` | `Cabo de rede conectado (1000 Mbps Full Duplex)` |
| **Rede/Wi-Fi** | Falha de senha no Wi-Fi | `Error` | `[WIFI] [ERRO]` | `Falha de autenticação WPA na rede 'Office-Secure'` |
| **Bateria/Power**| Carregador AC conectado | `Success` | `[ENERGIA] [AC]` | `Carregador conectado — Bateria carregando (78%)` |
| **Bateria/Power**| Bateria baixa (<15%) | `Warning` | `[ENERGIA] [BAIXA]` | `Bateria em 14% (restam aproximadamente 22 min)` |
| **Bateria/Power**| Bateria crítica (<5%) | `Error` | `[ENERGIA] [CRIT]` | `Bateria crítica em 4%! Conecte o carregador agora.` |

---

## 6. Interface TUI: Diagrama Espacial ASCII & Painel de Displays

### 6.1 Layout do Painel de Monitores (Aba / Modal Interativo)

```
┌─ Displays & Monitores ────────────────────────────────────────── [X11/RandR] ─┐
│ Disposição Espacial Virtual (Canvas: 3840x1080)                                │
│                                                                                │
│   ┌─── [1] eDP-1 ★ ───┐   ┌─────── [2] HDMI-1 ────────┐                        │
│   │   TELA DO NOTEBOOK │   │   LG UltraWide 29UM69G    │                        │
│   │   1920x1080 @ 60Hz │   │   1920x1080 @ 75Hz        │                        │
│   │   Posição: (0, 0)  │   │   Posição: (1920, 0)      │                        │
│   └────────────────────┘   └───────────────────────────┘                        │
│                                                                                │
├────────────────────────────────────────────────────────────────────────────────┤
│ Configurações do Monitor Selecionado: [2] HDMI-1 (LG UltraWide 29UM69G)       │
│                                                                                │
│  Presets Rápidos:                                                              │
│  [1] Expandir Dir. [2] Expandir Esq. [3] Espelhar [4] Só Externo [5] Só Note   │
│                                                                                │
│  Resolução: [ 1920x1080 ] (Suportadas: 1920x1080*, 1600x900, 1366x768, 1280x720)│
│  Taxa (Hz): [ 75.0 Hz   ] (Opções: 75.0Hz*, 60.0Hz, 50.0Hz)                    │
│  Rotação:   [ Normal (0°)] (Opções: Normal, Girar 90°, Girar 270°, Invertido)  │
│  Primário:  [ ] Não (Pressione 'p' para definir como monitor primário ★)      │
├────────────────────────────────────────────────────────────────────────────────┤
│ [1..5] Aplicar Preset  [+/-] Resolução  [h/v] Frequência  [r] Rotação  [p] Prim│
│ [Enter] Aplicar Mudanças  [Esc/q] Voltar  [i] Identificar Telas (OSD)          │
└────────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 Renderizador Puro do Diagrama ASCII Espacial (`render_ascii_display_canvas`)

O algoritmo mapeia as posições relativas $(x, y)$ e dimensões $(w, h)$ dos monitores para a grade de caracteres do terminal:

```rust
/// Gera linhas de texto representando a disposição dos monitores no espaço 2D
pub fn generate_ascii_canvas(monitors: &[MonitorDevice], canvas_width_chars: usize) -> Vec<String> {
    if monitors.is_empty() {
        return vec!["  [ Nenhum monitor ativo no espaço virtual ]".to_string()];
    }

    let mut lines = Vec::new();
    let mut top_line = String::from("  ");
    let mut title_line = String::from("  ");
    let mut res_line = String::from("  ");
    let mut pos_line = String::from("  ");
    let mut bot_line = String::from("  ");

    for (idx, m) in monitors.iter().filter(|m| m.is_active).enumerate() {
        let star = if m.is_primary { " ★" } else { "" };
        let name = if m.model_name.is_empty() { &m.connector } else { &m.model_name };
        let res = m.current_mode.as_ref().map(|c| c.label()).unwrap_or_else(|| "Desligado".into());
        let pos = format!("({}, {})", m.position.0, m.position.1);

        let box_w = 26;
        let header = format!("┌─ [{}] {}{} ", idx + 1, m.connector, star);
        let header_padded = format!("{:<width$}┐", header, width = box_w - 1);
        let t_padded = format!("│ {:<width$} │", truncate_str(name, box_w - 4), width = box_w - 4);
        let r_padded = format!("│ {:<width$} │", truncate_str(&res, box_w - 4), width = box_w - 4);
        let p_padded = format!("│ Pos: {:<width$} │", pos, width = box_w - 9);
        let b_padded = format!("└{}┘", "─".repeat(box_w - 2));

        top_line.push_str(&header_padded);
        top_line.push_str("   ");
        title_line.push_str(&t_padded);
        title_line.push_str("   ");
        res_line.push_str(&r_padded);
        res_line.push_str("   ");
        pos_line.push_str(&p_padded);
        pos_line.push_str("   ");
        bot_line.push_str(&b_padded);
        bot_line.push_str("   ");
    }

    lines.push(top_line);
    lines.push(title_line);
    lines.push(res_line);
    lines.push(pos_line);
    lines.push(bot_line);
    lines
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    } else {
        s.to_string()
    }
}
```

### 6.3 Overlay de Toasts na TUI (`draw_toast_overlay`)

As notificações ativas são renderizadas no canto superior/inferior direito com destaque de cores e badges ASCII:

```rust
// Renderização do Toast na Statusline ou Stack Flutuante
pub fn render_toast_badge(toast: &NotificationToast, pal: &Palette) -> Line<'static> {
    let (color, badge_text) = match toast.severity {
        NotificationSeverity::Info => (pal.accent, "[INFO]"),
        NotificationSeverity::Success => (pal.ok, "[OK  ]"),
        NotificationSeverity::Warning => (pal.warn, "[AVIS]"),
        NotificationSeverity::Error => (pal.err, "[ERRO]"),
    };

    Line::from(vec![
        Span::styled(format!(" {} ", toast.source.badge()), Style::default().fg(pal.bg).bg(color).bold()),
        Span::styled(format!(" {} ", badge_text), Style::default().fg(color).bold()),
        Span::styled(format!("{} — {}", toast.title, toast.message), Style::default().fg(pal.fg)),
    ])
}
```

---

## 7. Contratos de Mensagens e Modificações de Estado

### 7.1 Novos Variantes em `src/events/mod.rs`

```rust
// Em AppEvent:
/// Snapshot do estado dos monitores e displays (Módulo Displays). Boxed para evitar inflar o enum.
Display(Box<crate::backend::display::DisplaySnapshot>),

/// Notificação unificada de hardware/sistema para o barramento global de toasts.
Notification(crate::backend::display::NotificationToast),

// Em Action:
/// Aplica um preset rápido de disposição de monitores.
DisplayApplyPreset(crate::backend::display::DisplayLayoutPreset),

/// Ajusta a resolução e taxa de atualização de um monitor específico.
DisplaySetMode {
    connector: String,
    width: u32,
    height: u32,
    rate: f32,
},

/// Ajusta a rotação de um monitor.
DisplaySetRotation {
    connector: String,
    rotation: crate::backend::display::DisplayRotation,
},

/// Define o monitor especificado como primário.
DisplaySetPrimary(String),

/// Confirma a alteração visual antes do término do Safety Revert Timer.
DisplayConfirmSafety,

/// Reverte imediatamente a alteração de vídeo para a configuração anterior.
DisplayRevertSafety,

/// Força re-escaneamento imediato dos conectores via sysfs e xrandr.
DisplayRescan,
```

### 7.2 Safety Revert Modal (Proteção contra Telas Pretas)

Quando o usuário altera resoluções ou layouts manualmente, o `App` entra em modo de confirmação:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct DisplaySafetyRevertState {
    pub original_command: String,
    pub revert_command: String,
    pub expires_at: Instant,
    pub seconds_remaining: u8,
}
```

- A TUI renderiza um modal flutuante:  
  `[ ATENÇÃO: Configuração de vídeo alterada. Pressione ENTER para confirmar ou ESC para reverter (15s)... ]`
- Se o timer atingir 0 sem confirmação, o worker executa `revert_command` e restaura a tela original.

---

## 8. Plano de Execução Decomposto em Épicos (Kanban A a H)

A implementação segue uma ordem estrita de dependências sem quebrar compilações intermediárias:

### Épico A — Modelos de Dados, EDID Parser e Algoritmos Puros
- **A1.** Criar `src/backend/display.rs` contendo os enums `DisplayServerKind`, `DisplayLayoutPreset`, `DisplayRotation`, as structs `DisplayMode`, `MonitorDevice`, `DisplaySnapshot` e `EdidInfo`.
- **A2.** Implementar `parse_edid` em Rust puro com suporte a extração de nome de fabricante, modelo, número de série e modo preferido.
- **A3.** Implementar testes unitários para `parse_edid` usando dumps binários reais de EDID capturados de monitores LG, Dell, Samsung e telas de notebook.
- **A4.** Implementar funções puras de cálculo de canvas virtual e gerador de diagrama ASCII 2D (`generate_ascii_canvas`).

### Épico B — Sysfs DRM Monitor & Leitor de Conectores
- **B1.** Implementar varredura assíncrona de `/sys/class/drm/card*` para detectar conectores internos (`eDP`, `LVDS`) e externos (`HDMI`, `DP`, `VGA`).
- **B2.** Implementar leitura não-bloqueante de `/sys/class/drm/*/status`, `modes` e `edid`.
- **B3.** Adicionar detecção automática do servidor gráfico ativo (`DisplayServerKind::detect()`) testando variáveis `$WAYLAND_DISPLAY`, `$DISPLAY` e daemons de D-Bus.

### Épico C — Driver de Execução de Comandos de Vídeo (`xrandr` / Wayland)
- **C1.** Implementar executor assíncrono para comandos `xrandr` com parsing de `--query --verbose`.
- **C2.** Implementar gerador de comandos para os 5 presets: `ExtendRight`, `ExtendLeft`, `Mirror`, `ExternalOnly` e `InternalOnly`.
- **C3.** Implementar gerador de comandos para alteração de resolução, taxa em Hz, rotação e monitor primário.
- **C4.** Implementar abstração inicial para Wayland (`wlr-randr` e D-Bus Mutter `org.gnome.Mutter.DisplayConfig`).

### Épico D — Regra de Ouro de Automação do Hotplug
- **D1.** Implementar o loop do worker Tokio `backend::display::run` com monitoramento contínuo (polling de 500ms no sysfs).
- **D2.** Implementar a máquina de estados `evaluate_hotplug_transition`: ao detectar novo monitor externo em estado `connected`, invocar imediatamente o preset `ExtendRight`.
- **D3.** Emitir `AppEvent::Toast` de sucesso com badge `[MONITOR] [EXPAND]` ao aplicar auto-expansão.
- **D4.** Tratar o evento de desconexão com desligamento gracioso da saída e notificação informativa.

### Épico E — Barramento Global Unificado de Notificações
- **E1.** Implementar `NotificationManager`, `NotificationToast`, `NotificationSource` e `NotificationSeverity`.
- **E2.** Substituir o campo único `app.toast` por `app.notifications: NotificationManager`.
- **E3.** Integrar emissores de toast em todos os backends existentes:
  - `backend::storage.rs`: Toasts para inserção de pendrives, ejeção segura e término do Flasher ISO.
  - `backend::bluetooth.rs`: Toasts para conexão/desconexão e nível de bateria de fones/headsets.
  - `backend::network.rs`: Toasts para conexão Wi-Fi/Ethernet e erros de autenticação.
  - `backend::power.rs` / `system.rs`: Toasts para carregador conectado e bateria fraca (<15%).
  - `backend::audio.rs`: Toasts para mudança de saída padrão e alertas.
- **E4.** Escrever testes unitários para rotação, expiração e limites de memória da fila de toasts.

### Épico F — Interface TUI Ratatui (Painel de Displays & Toasts)
- **F1.** Criar `src/ui/display.rs` com renderização do diagrama espacial ASCII e cards de propriedades dos monitores.
- **F2.** Implementar o seletor de presets rápidos `[1] Expandir Dir. [2] Expandir Esq. [3] Espelhar [4] Só Ext. [5] Só Int.`.
- **F3.** Implementar modal de segurança (Safety Revert) com contagem regressiva de 15s.
- **F4.** Implementar a renderização da stack de toasts no topo/rodapé da TUI com paleta temática e badges ASCII.

### Épico G — Integração de Teclado, Roteamento no App e i18n
- **G1.** Adicionar variantes em `src/events/mod.rs` (`AppEvent::Display`, `Action::Display*`).
- **G2.** Mapear atalhos de teclado em `src/events/input.rs`: teclas `1..5` para presets, `+`/`-` para resolução, `r` para rotação, `p` para primário e `Enter` para aplicar.
- **G3.** Integrar campos no `App` (`src/app.rs`) e tratar o dispatch de ações.
- **G4.** Adicionar strings traduzidas no catálogo `src/i18n.rs` (pt-BR, en-US, es-ES).

### Épico H — Validação, Testes de Integração e Garantia de Qualidade
- **H1.** Criar fixtures de testes de integração em `tests/display_hotplug.rs` simulando eventos de plug/unplug no sysfs.
- **H2.** Validar compilação limpa com `cargo clippy -- -D warnings`.
- **H3.** Validar execução limpa em modo headless (`cargo test` e `TERM=dumb cargo run`).
- **H4.** Teste prático em hardware real com monitor externo HDMI/DisplayPort.

---

## 9. Matriz de Riscos & Mitigações

| Risco | Impacto | Mitigação |
|---|---|---|
| **Tela preta após aplicar resolução incompatível** | Usuário perde visibilidade do sistema. | **Safety Revert Modal:** contagem regressiva de 15 segundos reverte automaticamente o comando se não houver confirmação com `Enter`. |
| **Flapping / Repetidos eventos ao plugar conector com mau contato** | Disparo de múltiplos comandos `xrandr` simultâneos. | Debounce de 300ms na transição de status antes de disparar o executor. |
| **Travamento do `xrandr` em placas híbridas (NVIDIA Optimus / Prime)** | Bloqueio da thread principal da TUI. | Execução assíncrona com `tokio::time::timeout` de 3 segundos para todos os processos externos. |
| **Sessão Wayland sem suporte ao `xrandr`** | Falha silenciosa de configuração de vídeo. | Identificação prévia do `DisplayServerKind`; se Wayland, usa `wlr-randr` ou D-Bus Mutter/KScreen, ou degrada para telemetria DRM com aviso visual. |
| **Inundação de Toasts simultâneos** | Poluição visual e sobreposição na interface. | Fila `NotificationManager` limita o display a no máximo 3 toasts simultâneos, priorizando alertas de severidade `Error` e `Warning`. |

---

## 10. Conclusão & Próximos Passos

A presente especificação estabelece as bases definitivas para que o HAL-9001 forneça uma experiência de nível de sistema operacional comercial para gerenciamento de monitores e notificações de hardware em Linux, mantendo a filosofia de alta velocidade, elegância em terminal e zero inchaço de dependências.
