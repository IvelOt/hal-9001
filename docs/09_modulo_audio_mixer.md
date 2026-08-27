# 09 — Módulo de Áudio & Mixer de Dispositivos (Aba 5 — Áudio / Mixer)

> HAL-9001 — Planejamento arquitetural do Módulo 5 (Mixer de Áudio & Dispositivos) sobre
> **PipeWire**, **WirePlumber** e **PulseAudio**, em Rust assíncrono com Tokio e Ratatui.
> **Este documento é uma especificação arquitetural e plano de execução rigoroso.**
> O objetivo é fixar arquitetura, contratos de mensagem, modelos de dados, UX da TUI (Ratatui),
> manipulação de streams por app, suporte a overdrive (>100% até 150%), identificação amigável de
> programas, garantias de degradação graciosa e a decomposição em tarefas atômicas para o Kanban (Épicos A a H).

---

## 0. Contexto e Decisão de Engenharia

Na concepção inicial do HAL-9001, a Aba 5 havia sido rascunhada para telemetria de energia/bateria (`UPower`).
Contudo, sob a diretriz do Capitão, os dados essenciais de bateria e consumo já são perfeitamente atendidos
pelo painel de **Overview (Aba 1)** e pelas leituras de periféricos na **Aba 3 (Bluetooth - `Battery1`)**.

A criação de um **Mixer de Áudio Completo (Aba 5 — Áudio / Mixer)** preenche uma lacuna crítica no uso diário
em estações Linux modernas, substituindo a necessidade de abrir ferramentas pesadas de desktop (como `pavucontrol`
ou `alsamixer`) por uma interface TUI ultrarrápida, ergonômica, reativa e integrada ao tema do HAL-9001.

### Fluxo de Dados Unidirecional (HAL-9001 Standard)

Conforme estabelecido em `docs/01_arquitetura_e_stack.md`:

```
backend workers ──AppEvent(mpsc)──▶ App (estado) ──Action(broadcast)──▶ backend workers
                                        │
                                   ui::draw(&App, Frame)  (função pura, tick-driven)
```

### Regras Inegociáveis do Módulo

1. **ZERO Novas Dependências no `Cargo.toml`:** Toda a implementação reutiliza rigorosamente a stack já homologada do projeto (`tokio`, `ratatui`, `serde`, `serde_json`, `anyhow`, `crossterm` e D-Bus com `zbus`). Nenhuma nova crate externa ou binding C será adicionado.
2. **Reatividade e Não-Bloqueio da UI:** A thread de renderização Ratatui nunca invoca chamadas de I/O, comandos de shell ou esperas de sockets. Todo I/O de áudio é executado assincronamente em uma task Tokio dedicada (`src/backend/audio.rs`).
3. **Duplo Suporte (PipeWire/WirePlumber & PulseAudio):** Prioridade total para o ecossistema moderno PipeWire (`wpctl` / `pw-dump`), com fallback automático e transparente para PulseAudio (`pactl`) e ALSA (`amixer`).
4. **Estado Imutável via Snapshots:** O backend publica `AppEvent::Audio(Box<AudioSnapshot>)`. A UI consome `&App.audio` como estado imutável.
5. **Sem `Arc<Mutex<...>>` Compartilhado com a UI:** Toda mutação é disparada via canal `Action` e refletida de volta via `AppEvent`.
6. **Zero Emojis Policy:** Proibido o uso de emojis hardcoded no binário. Ícones usam glifos Nerd Fonts quando `config.ui.icons == true`, com fallback textual rígido em ASCII (ex.: `[SAID]`, `[MIC ]`, `[APP ]`, `[HDMI]`, `[FONE]`, `[USB ]`).
7. **Suporte a Overdrive (0..=150%):** O mixer permite ajuste seguro acima de 100% até 150% (amplificação por software), com indicação visual de cor distinta (alerta/amarelo/magenta).
8. **Internacionalização (i18n):** Todas as strings da interface usam a política de internacionalização (veja `AGENTS.md`).
9. **Zero Warnings:** Compilação limpa com `cargo clippy -- -D warnings` como critério mandatório de aceite.

---

## 1. Visão Geral da Arquitetura do Módulo

O subsistema de áudio opera em 3 domínios funcionais:

| Subsistema | Responsabilidade | Superfície de Risco |
|------------|------------------|---------------------|
| **A. Dispositivos de Saída (Sinks / Outputs)** | Enumeração de placas de áudio, alto-falantes internos, fones P2/Bluetooth, saídas HDMI/DisplayPort; controle de volume Master (0..150%), toggle de Mudo e definição do Sink padrão (*Default Audio Sink*). | Baixa / Média (afeta saída sonora global). |
| **B. Dispositivos de Entrada (Sources / Microfones)** | Enumeração de microfones internos, headsets, interfaces e microfones USB; controle de ganho/sensibilidade de entrada, Mudo e seleção do microfone padrão (*Default Audio Source*). | Baixa / Média (afeta captura de voz). |
| **C. Streams por Aplicativo (Playback Streams)** | Rastreamento dinâmico de aplicações ativas reproduzindo áudio (Spotify, Firefox, Discord, Steam, Games, VLC, Chromium); controle de volume individual e mudo independente por aplicação. | Média (streams são efêmeros e surgem/desaparecem dinamicamente). |

### 1.1 Diagrama de Threads e Canais

```
                          ┌──────────────────────────────────────────────────────────┐
                          │                 backend::audio::run                      │
                          │                                                          │
   PipeWire / WirePlumber │  Detector de Backend: PipeWire (wpctl) / Pulse (pactl)   │
   PulseAudio Sockets     │  Polling reativo + Watcher de eventos de áudio           │──┐
                          │                                                          │  │
   Action (broadcast) ───▶│  Dispatcher: SetVolume, VolumeUp, VolumeDown,           │  │ AppEvent::Audio
                          │              ToggleMute, SetDefault, Refresh             │  ▼ (Box<AudioSnapshot>)
                          │                                                          │  App.audio
   Debounce Timer (50ms) ─▶│  Coalescência de comandos rápidos de volume             │  (AudioSnapshot)
                          └──────────────────────────────────────────────────────────┘
```

---

## 2. Domínio de Áudio Linux & Estratégia de Extração/Controle

### 2.1 Detecção do Servidor de Áudio Ativo

O backend determina em tempo de execução o mecanismo primário disponível:

```
[Início] ──▶ Existe `wpctl` e daemon PipeWire ativo?
                 │
                 ├── SIM ──▶ Backend: AudioBackendKind::PipeWire (wpctl / pw-dump)
                 │
                 └── NÃO ──▶ Existe `pactl` e PulseAudio / PipeWire-Pulse ativo?
                                 │
                                 ├── SIM ──▶ Backend: AudioBackendKind::PulseAudio (pactl)
                                 │
                                 └── NÃO ──▶ Fallback: AudioBackendKind::Alsa (amixer) ou Degraded
```

### 2.2 Tabela de Comandos por Servidor de Som

| Operação | PipeWire / WirePlumber (`wpctl`) | PulseAudio (`pactl`) |
|----------|----------------------------------|----------------------|
| **Listar Estrutura** | `wpctl status` ou `pw-dump` | `pactl --format=json list sinks`, `sources`, `sink-inputs` |
| **Ajustar Volume (Absoluto)** | `wpctl set-volume --limit 1.5 <ID> <VAL>` (ex: `0.85`) | `pactl set-sink-volume <NAME> <VAL>%` (ex: `85%`) |
| **Aumentar Volume (Delta)** | `wpctl set-volume --limit 1.5 <ID> 5%+` | `pactl set-sink-volume <NAME> +5%` |
| **Diminuir Volume (Delta)** | `wpctl set-volume --limit 1.5 <ID> 5%-` | `pactl set-sink-volume <NAME> -5%` |
| **Alternar Mudo** | `wpctl set-mute <ID> toggle` | `pactl set-sink-mute <NAME> toggle` |
| **Definir Dispositivo Padrão** | `wpctl set-default <ID>` | `pactl set-default-sink <NAME>` / `set-default-source <NAME>` |
| **Volume de App (Stream)** | `wpctl set-volume --limit 1.5 <STREAM_ID> <VAL>` | `pactl set-sink-input-volume <INDEX> <VAL>%` |
| **Mudo de App (Stream)** | `wpctl set-mute <STREAM_ID> toggle` | `pactl set-sink-input-mute <INDEX> toggle` |

### 2.3 Parsing Resiliente da Saída de `wpctl status`

A saída textual padrão do WirePlumber estrutura nós de áudio em hierarquia legível:

```text
Audio
 ├─ Devices:
 │      48. Áudio interno                      [alsa]
 │  
 ├─ Sinks:
 │  *   57. Áudio interno Estéreo analógico  [vol: 0.80 MUTED]
 │      65. Fone de Ouvido Bluetooth (P20i)  [vol: 1.15]
 │  
 ├─ Sources:
 │  *   58. Áudio interno Estéreo analógico  [vol: 0.38]
 │      70. Microfone USB Condensador        [vol: 0.75 MUTED]
 │  
 ├─ Filters:
 │  
 └─ Streams:
        82. Spotify                            
             83. output_FL       > ALC257 Analog:playback_FL	[vol: 0.60]
        95. Firefox                            
             96. output_FL       > ALC257 Analog:playback_FL	[vol: 1.00]
        104. Discord                           
             105. output_FL      > ALC257 Analog:playback_FL	[vol: 0.80 MUTED]
```

**Regras de Parsing:**
1. O marcador `*` antes do ID indica o dispositivo **Default / Padrão** do sistema.
2. Dispositivos sob `Sinks` mapeiam para `AudioDevice` de saída.
3. Dispositivos sob `Sources` mapeiam para `AudioDevice` de entrada (microfone).
4. Nós sob `Streams` mapeiam para `AudioStream` (aplicações em reprodução).
5. O token `[vol: X.XX]` fornece o nível linear de volume (0.00 a 1.50).
6. A presença de `MUTED` indica estado de mudo ativo.

---

## 3. Modelos de Dados em Rust (`src/backend/audio.rs`)

### 3.1 Identidade e Tipos de Dispositivos

```rust
use serde::{Deserialize, Serialize};

/// Backend de áudio ativo no host
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioBackendKind {
    PipeWire,
    PulseAudio,
    Alsa,
    Degraded,
}

impl AudioBackendKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::PipeWire => "PipeWire (WirePlumber)",
            Self::PulseAudio => "PulseAudio",
            Self::Alsa => "ALSA Direct",
            Self::Degraded => "Indisponível",
        }
    }
}

/// Categorias / Sub-abas do Mixer de Áudio
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AudioCategory {
    #[default]
    Sinks,       // Saídas (Alto-falantes, Fones, HDMI)
    Sources,     // Entradas (Microfones)
    Streams,     // Aplicativos (Spotify, Discord, etc.)
}

impl AudioCategory {
    pub const ALL: [AudioCategory; 3] = [
        AudioCategory::Sinks,
        AudioCategory::Sources,
        AudioCategory::Streams,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::Sinks => 0,
            Self::Sources => 1,
            Self::Streams => 2,
        }
    }

    pub fn from_index(i: usize) -> Self {
        match i {
            0 => Self::Sinks,
            1 => Self::Sources,
            _ => Self::Streams,
        }
    }

    pub fn title_badge(self) -> &'static str {
        match self {
            Self::Sinks => "1. Saídas (Sinks)",
            Self::Sources => "2. Entradas (Mic)",
            Self::Streams => "3. Aplicativos (Apps)",
        }
    }
}

/// Classificação semântica do endpoint de áudio
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioDeviceType {
    Speaker,          // Alto-falantes embutidos
    Headphones,       // Fones de ouvido P2/P3
    HeadsetBluetooth, // Fone/Headset sem fio Bluetooth
    Hdmi,             // Saída de Áudio Digital HDMI / DisplayPort
    Microphone,       // Microfone interno / integrado
    MicrophoneUsb,    // Microfone ou Interface USB
    VirtualStream,    // Stream virtual / Loopback
    Generic,          // Dispositivo genérico
}

impl AudioDeviceType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Speaker => "Alto-falante",
            Self::Headphones => "Fone de Ouvido",
            Self::HeadsetBluetooth => "Fone Bluetooth",
            Self::Hdmi => "HDMI / DisplayPort",
            Self::Microphone => "Microfone Interno",
            Self::MicrophoneUsb => "Microfone USB",
            Self::VirtualStream => "Stream de Áudio",
            Self::Generic => "Dispositivo de Áudio",
        }
    }

    /// Retorna o ícone Nerd Font ou badge ASCII (Zero Emojis Policy)
    pub fn icon_badge(self, use_nerd_icons: bool) -> &'static str {
        if use_nerd_icons {
            match self {
                Self::Speaker => "\u{f028}",          //  nf-fa-volume_up
                Self::Headphones => "\u{f025}",       // 󰋋 nf-fa-headphones
                Self::HeadsetBluetooth => "\u{f293}", // 󰂯 nf-fa-bluetooth
                Self::Hdmi => "\u{f008}",             // 󰤽 nf-fa-film / video
                Self::Microphone => "\u{f130}",       // 󰍬 nf-fa-microphone
                Self::MicrophoneUsb => "\u{f87b}",    // 󰍹 nf-md-usb
                Self::VirtualStream => "\u{f001}",    // 󰝚 nf-fa-music
                Self::Generic => "\u{f026}",          // 󰕿 nf-fa-volume_off
            }
        } else {
            match self {
                Self::Speaker => "[ALTO]",
                Self::Headphones => "[FONE]",
                Self::HeadsetBluetooth => "[BT  ]",
                Self::Hdmi => "[HDMI]",
                Self::Microphone => "[MIC ]",
                Self::MicrophoneUsb => "[USB ]",
                Self::VirtualStream => "[STRM]",
                Self::Generic => "[DEV ]",
            }
        }
    }
}
```

### 3.2 Estruturas de Dispositivos e Streams

```rust
/// Dispositivo físico ou nó de áudio (Sink ou Source)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioDevice {
    pub id: String,                  // ID estável ou número do nó (ex: "57")
    pub node_id: u32,                // ID numérico PipeWire/Pulse
    pub name: String,                // Nome técnico do nó (ex: "alsa_output.pci-0000_00_1f.3.analog-stereo")
    pub description: String,         // Rótulo amigável (ex: "Áudio interno Estéreo analógico")
    pub device_type: AudioDeviceType,
    pub volume: f32,                 // Nível linear 0.00..=1.50 (0% a 150%)
    pub is_muted: bool,
    pub is_default: bool,
    pub active_port: Option<String>, // Ex: "analog-output-speaker", "analog-output-headphones"
    pub ports: Vec<String>,
}

impl AudioDevice {
    /// Percentual de volume arredondado (0..=150)
    pub fn volume_pct(&self) -> u16 {
        (self.volume * 100.0).round().clamp(0.0, 150.0) as u16
    }

    /// Nome exibido na interface com fallback limpo
    pub fn display_name(&self) -> &str {
        if !self.description.trim().is_empty() {
            &self.description
        } else {
            &self.name
        }
    }
}

/// Fluxo de reprodução por aplicativo (Playback Stream / Sink Input)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStream {
    pub id: String,                  // ID numérico do stream (ex: "82")
    pub node_id: u32,
    pub app_name: String,            // Nome do aplicativo (ex: "Spotify", "Firefox", "Discord")
    pub binary_name: Option<String>, // Nome do executável (ex: "spotify", "firefox-bin")
    pub media_title: Option<String>, // Título da mídia atual (se fornecido pelo cliente)
    pub volume: f32,                 // Nível linear 0.00..=1.50
    pub is_muted: bool,
    pub target_sink: Option<String>, // ID do Sink para onde está roteado
}

impl AudioStream {
    pub fn volume_pct(&self) -> u16 {
        (self.volume * 100.0).round().clamp(0.0, 150.0) as u16
    }
}

/// Snapshot completo de áudio publicado pelo backend
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSnapshot {
    pub backend: AudioBackendKind,
    pub sinks: Vec<AudioDevice>,
    pub sources: Vec<AudioDevice>,
    pub streams: Vec<AudioStream>,
    pub default_sink_id: Option<String>,
    pub default_source_id: Option<String>,
}

impl AudioSnapshot {
    pub fn empty() -> Self {
        Self {
            backend: AudioBackendKind::Degraded,
            sinks: Vec::new(),
            sources: Vec::new(),
            streams: Vec::new(),
            default_sink_id: None,
            default_source_id: None,
        }
    }
}
```

---

## 4. Algoritmos Puros: Derivação de Ícones, Nomes Amigáveis e Parsing

### 4.1 Identificação de Aplicações e Badges Nerd Font / ASCII

Uma função pura inspeciona o nome da aplicação (`app_name`) ou o binário (`binary_name`) para atribuir o ícone de marca correspondente:

```rust
pub fn derive_app_badge(app_name: &str, binary: Option<&str>, use_nerd_icons: bool) -> (&'static str, &'static str) {
    let name_low = app_name.to_ascii_lowercase();
    let bin_low = binary.unwrap_or("").to_ascii_lowercase();

    if name_low.contains("spotify") || bin_low.contains("spotify") {
        if use_nerd_icons { ("\u{f1bc}", "Spotify") } else { ("[SPOT]", "Spotify") }
    } else if name_low.contains("firefox") || bin_low.contains("firefox") {
        if use_nerd_icons { ("\u{f269}", "Firefox") } else { ("[FIRE]", "Firefox") }
    } else if name_low.contains("discord") || bin_low.contains("discord") || bin_low.contains("vesktop") {
        if use_nerd_icons { ("\u{f392}", "Discord") } else { ("[DISC]", "Discord") }
    } else if name_low.contains("steam") || bin_low.contains("steam") {
        if use_nerd_icons { ("\u{f1b6}", "Steam") } else { ("[STEM]", "Steam") }
    } else if name_low.contains("vlc") || bin_low.contains("vlc") {
        if use_nerd_icons { ("\u{f008}", "VLC Player") } else { ("[VLC ]", "VLC Player") }
    } else if name_low.contains("chrome") || name_low.contains("chromium") || bin_low.contains("chrome") {
        if use_nerd_icons { ("\u{f268}", "Chrome/Chromium") } else { ("[CHRM]", "Chromium") }
    } else if name_low.contains("telegram") || bin_low.contains("telegram") {
        if use_nerd_icons { ("\u{f2c6}", "Telegram") } else { ("[TELE]", "Telegram") }
    } else if name_low.contains("obs") || bin_low.contains("obs64") {
        if use_nerd_icons { ("\u{f03d}", "OBS Studio") } else { ("[OBS ]", "OBS Studio") }
    } else {
        if use_nerd_icons { ("\u{f001}", app_name) } else { ("[APP ]", app_name) }
    }
}
```

### 4.2 Classificação de Tipo de Dispositivo (`AudioDeviceType`)

```rust
pub fn derive_device_type(name: &str, desc: &str, active_port: Option<&str>) -> AudioDeviceType {
    let combined = format!("{} {} {}", name, desc, active_port.unwrap_or("")).to_ascii_lowercase();

    if combined.contains("bluez") || combined.contains("bluetooth") || combined.contains("p20i") {
        AudioDeviceType::HeadsetBluetooth
    } else if combined.contains("hdmi") || combined.contains("displayport") || combined.contains("dp") {
        AudioDeviceType::Hdmi
    } else if combined.contains("headphone") || combined.contains("headset") || combined.contains("fone") {
        AudioDeviceType::Headphones
    } else if combined.contains("usb") {
        if combined.contains("mic") || combined.contains("input") || combined.contains("capture") {
            AudioDeviceType::MicrophoneUsb
        } else {
            AudioDeviceType::Speaker
        }
    } else if combined.contains("mic") || combined.contains("input") || combined.contains("capture") {
        AudioDeviceType::Microphone
    } else if combined.contains("speaker") || combined.contains("alto-falante") || combined.contains("estéreo analógico") {
        AudioDeviceType::Speaker
    } else {
        AudioDeviceType::Generic
    }
}
```

### 4.3 Formatação de Barras de Volume com Suporte a Overdrive (0..150%)

As barras visuais do HAL-9001 possuem 20 caracteres de resolução gráfica:
- Faixa 0..100%: Preenchimento em blocos `█` graduados.
- Faixa 101..150%: Extensão gráfica com marcador de amplificação `▲` ou cor diferenciada.
- Mudo: Barra atenuada em estilo monocromático e indicação textual `[MUTED]`.

```rust
pub fn render_volume_slider(volume_pct: u16, is_muted: bool, width: usize) -> (String, &'static str) {
    if is_muted {
        let empty = "░".repeat(width);
        return (format!("[{empty}] (MUDO)"), "muted");
    }

    let clamped = volume_pct.min(150);
    // Base de cálculo para largura normal (100% ocupa a barra total; overdrive adiciona marcador visual)
    let normal_pct = clamped.min(100) as usize;
    let filled_normal = (normal_pct * width) / 100;
    let empty_normal = width.saturating_sub(filled_normal);

    if clamped <= 100 {
        let bar = format!("{}{}", "█".repeat(filled_normal), "░".repeat(empty_normal));
        (format!("[{bar}] {clamped:>3}%"), "normal")
    } else {
        let boost = clamped - 100;
        let bar = "█".repeat(width);
        (format!("[{bar}] {clamped:>3}% (+{boost}%)"), "overdrive")
    }
}
```

---

## 5. Máquinas de Estado e Ciclos de Vida

### 5.1 Ciclo de Polling e Reatividade do Backend

```
┌─────────────────┐           Tick (250ms) / Action::Audio*           ┌──────────────────────┐
│  IDLE / SLEEP   │ ─────────────────────────────────────────────────▶│ Executa wpctl / pactl│
│                 │                                                   │ Coleta Snapshot      │
└─────────────────┘◀──────────────────────────────────────────────────└──────────────────────┘
                           Emite AppEvent::Audio se houve mudança
```

1. **Watcher Reativo:** O backend monitora o servidor de som via polling curto (250ms quando a aba está em foco, 1000ms em segundo plano).
2. **Execução Imediata:** Ações disparadas pelo usuário (`Action::Audio*`) executam a mutação imediatamente via `tokio::process::Command` e solicitam coleta instantânea de snapshot, garantindo latência de resposta perceptual nula (<16ms).
3. **Debounce de Volume Contínuo:** Quando o usuário segura `+` ou `-`, múltiplos eventos são coalescidos em passos de 5% sem sobrecarregar o daemon do PipeWire.

### 5.2 Transições de Estado por Ação

| Ação | Efeito no PipeWire / WirePlumber | Resposta da UI |
|------|-----------------------------------|----------------|
| `Action::AudioVolumeUp(id, 0.05)` | `wpctl set-volume --limit 1.5 <id> 5%+` | Slider incrementa +5% até 150%. |
| `Action::AudioVolumeDown(id, 0.05)` | `wpctl set-volume --limit 1.5 <id> 5%-` | Slider decrementa -5% até 0%. |
| `Action::AudioToggleMute(id)` | `wpctl set-mute <id> toggle` | Alterna flag `MUTED` e altera cor do card. |
| `Action::AudioSetDefault(id)` | `wpctl set-default <id>` | Move badge `* [PADRÃO]` para o dispositivo escolhido. |
| `Action::AudioSwitchCategory(cat)` | Altera estado local `app.audio_category` | Redesenha lista da categoria correspondente instantaneamente. |

---

## 6. Interface TUI (Ratatui — Aba 5 `src/ui/audio.rs`)

### 6.1 Layout Responsivo e Sub-Abas

A Aba 5 organiza-se em 3 blocos visuais bem definidos:

```
┌─ Áudio & Mixer de Dispositivos ────────────────────────────────── [PipeWire] ─┐
│ [1] Saídas (Sinks)  │ * [2] Aplicativos (Streams) * │  [3] Entradas (Mic)      │
├────────────────────────────────────────────────────────────────────────────────┤
│  St  Ícone   Nome / Dispositivo                  Volume / Slider         Nível │
│ ────────────────────────────────────────────────────────────────────────────── │
│  ●   [SPOT]  Spotify (Playing: Bohemian Rapsody) [████████████░░░░░░░░]   60%  │
│  ▶   [DISC]  Discord (Voice Connected)           [████████████████░░░░]   80%  │
│  M   [FIRE]  Firefox (YouTube Video)             [░░░░░░░░░░░░░░░░░░░░] (MUDO) │
│      [STEM]  Steam (Game: Cyberpunk 2077)        [████████████████████]  115%▲ │
├────────────────────────────────────────────────────────────────────────────────┤
│ Seleção: Discord | Saída: Áudio interno Estéreo analógico | Volume: 80%        │
│ [1/2/3] Sub-aba [↑/↓] Navegar [+/-] Volume [m] Mudo [*] Padrão [r] Refresh     │
└────────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 Visualização da Sub-Aba 1: Saídas de Áudio (Sinks)

```
┌─ Áudio & Mixer de Dispositivos ────────────────────────────────── [PipeWire] ─┐
│ * [1] Saídas (Sinks) * │  [2] Aplicativos (Streams)  │  [3] Entradas (Mic)     │
├────────────────────────────────────────────────────────────────────────────────┤
│  St  Ícone   Dispositivo de Saída                Volume / Slider         Nível │
│ ────────────────────────────────────────────────────────────────────────────── │
│★ ● ▶ [ALTO]  Áudio interno Estéreo analógico     [████████████████░░░░]   80%  │
│      [FONE]  soundcore P20i (Bluetooth A2DP)     [████████████░░░░░░░░]   60%  │
│      [HDMI]  Áudio Digital HDMI/DisplayPort      [████████████████████]  100%  │
├────────────────────────────────────────────────────────────────────────────────┤
│ Dispositivo: Áudio interno | Porta: Alto-falantes | Status: Padrão do Sistema  │
│ [Enter/*] Definir Padrão [m] Mudo [+/-] Volume (0..150%) [p] Alternar Porta   │
└────────────────────────────────────────────────────────────────────────────────┘
```

### 6.3 Visualização da Sub-Aba 3: Microfones & Entradas (Sources)

```
┌─ Áudio & Mixer de Dispositivos ────────────────────────────────── [PipeWire] ─┐
│   [1] Saídas (Sinks)   │  [2] Aplicativos (Streams)  │ * [3] Entradas (Mic) *  │
├────────────────────────────────────────────────────────────────────────────────┤
│  St  Ícone   Microfone / Entrada                 Ganho / Sensibilidade   Nível │
│ ────────────────────────────────────────────────────────────────────────────── │
│★ ● ▶ [MIC ]  Microfone Interno Analógico         [███████░░░░░░░░░░░░░]   38%  │
│  M   [USB ]  HyperX SoloCast USB Condenser       [░░░░░░░░░░░░░░░░░░░░] (MUDO) │
│      [BT  ]  soundcore P20i (Handsfree HFP)      [████████████░░░░░░░░]   60%  │
├────────────────────────────────────────────────────────────────────────────────┤
│ Microfone: HyperX SoloCast | Estado: MUDO | Ganho anterior: 75%               │
│ [Enter/*] Definir Padrão [m] Alternar Mudo [+/-] Ajustar Ganho de Entrada     │
└────────────────────────────────────────────────────────────────────────────────┘
```

### 6.4 Mapeamento de Atalhos e Teclado

| Tecla | Ação | Contexto |
|-------|------|----------|
| `1` / `2` / `3` | Alternar diretamente para Saídas (1), Apps (2) ou Microfones (3) | Aba Áudio ativa |
| `Tab` / `Shift-Tab` | Ciclar entre as 3 sub-abas | Aba Áudio ativa |
| `h` / `l` ou `←` / `→` | Navegar entre sub-abas ou ajustar volume | Aba Áudio ativa |
| `j` / `k` ou `↑` / `↓` | Navegar pela lista de dispositivos/streams | Sub-aba ativa |
| `+` / `=` ou `]` | Aumentar volume em +5% (até 150%) | Item selecionado |
| `-` / `_` ou `[` | Diminuir volume em -5% (até 0%) | Item selecionado |
| `m` | Alternar Mudo (Mute / Unmute) | Item selecionado |
| `Enter` / `*` / `d` | Definir como Dispositivo Padrão (Sinks/Sources) | Dispositivo selecionado |
| `r` | Forçar atualização imediata do snapshot | Sempre |

### 6.5 Estados Degradados na Interface

| Condição | Exibição na Interface |
|----------|----------------------|
| **Nenhum servidor de som detectado** | Painel `ServiceDegraded`: *"Servidores PipeWire e PulseAudio não encontrados. Verifique se o daemon de áudio do usuário está ativo."* |
| **Nenhum aplicativo tocando som** | Na sub-aba de Aplicativos: exibe banner centralizado *"Nenhum aplicativo reproduzindo áudio no momento. Inicie o Spotify, navegador ou player."* |
| **Nenhum microfone conectado** | Na sub-aba de Entradas: exibe aviso *"Nenhum dispositivo de captura de áudio detectado."* |

---

## 7. Contratos de Mensagem

### 7.1 Novos Variantes em `src/events/mod.rs`

```rust
// Em AppEvent:
/// Snapshot completo do subsistema de áudio (Módulo 5). Boxed para evitar inflar o enum.
Audio(Box<crate::backend::audio::AudioSnapshot>),

// Em Action:
/// Ajusta o volume absoluto de um nó de áudio (0.0..=1.5).
AudioSetVolume {
    target_id: String,
    volume: f32,
},
/// Incrementa o volume de um nó em um delta percentual (ex: +0.05).
AudioVolumeUp {
    target_id: String,
    delta: f32,
},
/// Decrementa o volume de um nó em um delta percentual (ex: -0.05).
AudioVolumeDown {
    target_id: String,
    delta: f32,
},
/// Alterna o mudo de um nó de áudio.
AudioToggleMute(String),
/// Define o dispositivo especificado como padrão (Sink ou Source).
AudioSetDefault {
    target_id: String,
    is_source: bool,
},
/// Alterna a categoria/sub-aba ativa no mixer.
AudioSwitchCategory(crate::backend::audio::AudioCategory),
/// Solicita refresh forçado do subsistema de áudio.
AudioRefresh,
```

### 7.2 Mapeamento de Teclas em `src/events/input.rs`

```rust
// Dentro de map_key quando active == Tab::Audio:
if active == Tab::Audio {
    match key.code {
        KeyCode::Char('1') => return Some(Action::AudioSwitchCategory(AudioCategory::Sinks)),
        KeyCode::Char('2') => return Some(Action::AudioSwitchCategory(AudioCategory::Streams)),
        KeyCode::Char('3') => return Some(Action::AudioSwitchCategory(AudioCategory::Sources)),
        KeyCode::Tab => return Some(Action::Right), // Cicla sub-aba para a direita
        KeyCode::BackTab => return Some(Action::Left), // Cicla sub-aba para a esquerda
        KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char(']') => {
            return Some(Action::AudioVolumeUp { target_id: String::new(), delta: 0.05 });
        }
        KeyCode::Char('-') | KeyCode::Char('_') | KeyCode::Char('[') => {
            return Some(Action::AudioVolumeDown { target_id: String::new(), delta: 0.05 });
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            return Some(Action::AudioToggleMute(String::new()));
        }
        KeyCode::Char('*') | KeyCode::Char('d') => {
            return Some(Action::AudioSetDefault { target_id: String::new(), is_source: false });
        }
        KeyCode::Char('r') => return Some(Action::AudioRefresh),
        _ => {}
    }
}
```

### 7.3 Extensões no `App` (`src/app.rs`)

Campos adicionados à struct `App`:

```rust
pub audio: Option<Box<crate::backend::audio::AudioSnapshot>>,
pub audio_category: crate::backend::audio::AudioCategory,
pub audio_selected_sink: usize,
pub audio_selected_source: usize,
pub audio_selected_stream: usize,
```

No método `handle_event`:
```rust
AppEvent::Audio(snap) => {
    // Clampeia seleções para evitar out-of-bounds após remoção de streams/dispositivos
    if !snap.sinks.is_empty() && self.audio_selected_sink >= snap.sinks.len() {
        self.audio_selected_sink = snap.sinks.len() - 1;
    }
    if !snap.sources.is_empty() && self.audio_selected_source >= snap.sources.len() {
        self.audio_selected_source = snap.sources.len() - 1;
    }
    if !snap.streams.is_empty() && self.audio_selected_stream >= snap.streams.len() {
        self.audio_selected_stream = snap.streams.len() - 1;
    }
    self.audio = Some(snap);
}
```

No método `dispatch`:
```rust
// Resolução dinâmica do ID selecionado com base na categoria ativa:
Action::AudioVolumeUp { target_id, delta } => {
    let resolved_id = if target_id.is_empty() {
        self.resolve_current_audio_id()
    } else {
        Some(target_id)
    };
    if let Some(id) = resolved_id {
        let _ = action_tx.send(Action::AudioVolumeUp { target_id: id, delta });
    }
}
Action::AudioToggleMute(target_id) => {
    let resolved_id = if target_id.is_empty() {
        self.resolve_current_audio_id()
    } else {
        Some(target_id)
    };
    if let Some(id) = resolved_id {
        let _ = action_tx.send(Action::AudioToggleMute(id));
    }
}
Action::Enter if self.active == Tab::Audio => {
    // Se estiver em Sinks ou Sources, Enter define como Padrão. Se em Streams, Enter alterna Mudo.
    if let Some(snap) = &self.audio {
        match self.audio_category {
            AudioCategory::Sinks => {
                if let Some(dev) = snap.sinks.get(self.audio_selected_sink) {
                    let _ = action_tx.send(Action::AudioSetDefault { target_id: dev.id.clone(), is_source: false });
                }
            }
            AudioCategory::Sources => {
                if let Some(dev) = snap.sources.get(self.audio_selected_source) {
                    let _ = action_tx.send(Action::AudioSetDefault { target_id: dev.id.clone(), is_source: true });
                }
            }
            AudioCategory::Streams => {
                if let Some(st) = snap.streams.get(self.audio_selected_stream) {
                    let _ = action_tx.send(Action::AudioToggleMute(st.id.clone()));
                }
            }
        }
    }
}
```

---

## 8. Limites de Escopo v1 (Documentados)

| Fora do Escopo v1 | Justificativa / Roadmap Futuro |
|-------------------|--------------------------------|
| **Matriz de Roteamento JACK Arbitrária** | Conexões ponta-a-ponta de nós complexos de estúdio ficam para plugin pro-audio dedicado. A v1 foca em mixagem diária e seleção de endpoints. |
| **Equalizador Paramétrico DSP Embutido** | Configuração de curvas de EQ por nó requer integração profunda com filtros PipeWire/EasyEffects. Previsto para v2. |
| **Gravação de Áudio / Loopback para Arquivo** | Fora do escopo do mixer (utilizar ferramentas dedicadas de gravação). |
| **Balanço Estéreo por Canal L/R Individual** | Ajuste independente de canal esquerdo/direito será suportado na v1.1. Na v1, o volume ajusta o master unificado do nó. |

---

## 9. Dependências e Ferramentas (ZERO Novas Crates)

| Componente | Tipo | Papel no Módulo | Status |
|------------|------|-----------------|--------|
| `tokio` | Crate Rust | Execução assíncrona, timers de polling e canais mpsc/broadcast | **Já presente no `Cargo.toml`** |
| `ratatui` | Crate Rust | Renderização TUI das barras de volume, badges e sub-abas | **Já presente no `Cargo.toml`** |
| `serde` / `serde_json` | Crate Rust | Desserialização de payloads JSON de áudio | **Já presente no `Cargo.toml`** |
| `wpctl` / `pw-dump` | CLI PipeWire | Extração de topologia e controle de nós de som | Nativo no host Linux moderno |
| `pactl` | CLI PulseAudio | Fallback em distribuições legadas sem WirePlumber | Nativo no host Linux |

> **Garantia Arquitetural:** Zero novas dependências de pacotes no `Cargo.toml`. Reutilização de 100% da stack existente.

---

## 10. Estratégia de Testes

| Nível de Teste | Alvo / Escopo |
|----------------|---------------|
| **Unitário (Parsers Puros)** | - `parse_wpctl_status`: parsing de blocos Sinks, Sources e Streams a partir de strings estáticas reais capturadas de sistemas reais.<br>- `derive_app_badge`: mapeamento correto de Spotify, Firefox, Discord, Steam, VLC, OBS, etc., para ícones Nerd Font e ASCII.<br>- `derive_device_type`: classificação de Headphone, Bluetooth, HDMI, USB Mic e Speakers.<br>- `render_volume_slider`: formatação de volumes em 0%, 50%, 100%, 150% (overdrive) e estado Muted. |
| **Unitário (Máquinas de Estado)** | - Clampeamento de volumes entre 0.0 e 1.50.<br>- Alternância de categorias e preservação dos índices de seleção.<br>- Resolução de IDs vazios a partir do cursor ativo. |
| **Integração (Backend Assíncrono)** | - Mock de subprocessos emitindo saídas de `wpctl` e `pactl` para validar a publicação de `AppEvent::Audio`.<br>- Validação de não-travamento caso o comando CLI retorne erro ou timeout. |
| **Smoke Test** | - Execução headless (`cargo test` e `TERM=dumb cargo run`) em ambiente de CI sem hardware de áudio, validando degradação limpa sem panic. |
| **E2E / Manual** | - Teste em hardware real com fone Bluetooth e alto-falante integrado alternando saídas, silenciando Spotify e ajustando ganho do microfone. |

---

## 11. Plano de Implementação Modular & Decomposição em Tasks (Kanban)

### Épico A — Modelos e Parsers Puros (100% testável e sem I/O)
- **A1.** Definir `AudioSnapshot`, `AudioDevice`, `AudioStream`, `AudioCategory`, `AudioDeviceType` e `AudioBackendKind` em `src/backend/audio.rs`.
- **A2.** Implementar `derive_app_badge`, `derive_device_type`, `render_volume_slider` e seus testes unitários completos.
- **A3.** Adicionar variantes `AppEvent::Audio` e `Action::Audio*` em `src/events/mod.rs` e registrar campos em `src/app.rs`.

### Épico B — Parser e Driver PipeWire / WirePlumber (`wpctl`)
- **B1.** Implementar parser robusto da saída de `wpctl status` separando Sinks, Sources e Streams.
- **B2.** Implementar extração de volume e mudo via `wpctl get-volume <ID>` e `wpctl inspect <ID>`.
- **B3.** Escrever testes unitários com fixtures de saída real de `wpctl`.

### Épico C — Driver de Fallback PulseAudio (`pactl`)
- **C1.** Implementar parser de JSON para `pactl --format=json list sinks`, `sources` e `sink-inputs`.
- **C2.** Mapear entidades do PulseAudio para os modelos unificados `AudioDevice` e `AudioStream`.
- **C3.** Adicionar testes unitários com mocks de payloads JSON do PulseAudio.

### Épico D — Loop do Worker Tokio e Mutadores de Estado
- **D1.** Criar a task principal `backend::audio::run` com detecção automática do backend (PipeWire vs PulseAudio).
- **D2.** Implementar executores de `Action::AudioVolumeUp`, `Action::AudioVolumeDown`, `Action::AudioSetVolume`, `Action::AudioToggleMute` e `Action::AudioSetDefault`.
- **D3.** Adicionar proteção com timeout de 3 segundos para execuções de comandos de áudio.

### Épico E — Interface Ratatui (Tab 5 — `src/ui/audio.rs`)
- **E1.** Implementar seletor de sub-abas no topo (`[1] Saídas | [2] Aplicativos | [3] Entradas`).
- **E2.** Implementar renderização de cards e sliders visuais com barras graduadas, indicação de Mudo e destaque de Overdrive (>100%).
- **E3.** Implementar renderização do rodapé com telemetria do item focado e legenda de atalhos.
- **E4.** Implementar telas de estados degradados e lista vazia de streams.

### Épico F — Integração de Teclado e Roteamento no App
- **F1.** Mapear atalhos `1`, `2`, `3`, `Tab`, `+`, `-`, `m`, `*`, `d`, `r` em `src/events/input.rs` quando na Aba 5.
- **F2.** Implementar resolução de seleção contextual em `App::dispatch`.
- **F3.** Atualizar `Tab::title_in` em `src/app.rs` e o catálogo de mensagens em `src/i18n.rs` para refletir "Áudio / Mixer".

### Épico G — Integração Geral e Limpeza de Dependências Antigas
- **G1.** Substituir o stub de energia em `src/backend/mod.rs` e `src/ui/mod.rs` pelo novo módulo de áudio.
- **G2.** Atualizar documentações de referência (`docs/02_especificacao_das_abas.md` e `docs/03_plano_de_execucao_modular.md`).

### Épico H — Validação, Testes Automatizados e Fechamento
- **H1.** Criar suíte completa de testes de integração em `tests/audio_mixer.rs`.
- **H2.** Validação com `cargo clippy -- -D warnings` e `cargo test`.
- **H3.** Testes manuais em hardware Linux com múltiplos streams simultâneos.

### Grafo de Dependências

```
Épico A (Modelos) ──▶ Épico B (Driver PipeWire) ──▶ Épico D (Worker Tokio)
       │                      │                             │
       │                      └──▶ Épico C (Fallback Pulse) ┘
       │                                                    │
       ├──▶ Épico E (UI Ratatui) ◀──────────────────────────┤
       │          │                                         │
       │          ▼                                         ▼
       └──▶ Épico F (Keymap & Dispatch) ◀───────────── Épico G (Integração App)
                  │
                  ▼
            Épico H (Validação & Testes)
```

---

## 12. Riscos & Mitigações

| Risco | Impacto | Mitigação |
|-------|---------|-----------|
| **Streams de aplicativos fechando subitamente (TOCTOU)** | Erro ao tentar ajustar volume de um app que acabou de fechar. | Comandos CLI ignoram código de saída não-zero para nós destruídos e forçam coleta de novo snapshot sem alertar erro falso. |
| **Inundação de comandos ao pressionar teclas de volume repetidamente** | Alto consumo de CPU e atraso no áudio. | O backend coalesces passos rápidos em deltas acumulados e despacha chamadas com debounce leve (~50ms). |
| **Volumes acima de 100% distorcendo o áudio** | Desconforto acústico no usuário. | O overdrive é limitado rigidamente a 150% e a interface exibe aviso visual colorido (`▲ +X%`). |
| **Ambientes heterogêneos (alguns com WirePlumber, outros com PulseAudio puro)** | Falha ao inicializar áudio. | Detecção dinâmica em tempo de boot com chaveamento automático de driver (`PipeWire` ➔ `PulseAudio` ➔ `Alsa`). |

---

## 13. Definição de Pronto (Módulo 5 / Aba 5)

1. **`cargo clippy -- -D warnings` limpo sem nenhum alerta.**
2. **ZERO novas dependências externas adicionadas ao `Cargo.toml`.**
3. **Aba 5 totalmente operacional** exibindo Saídas, Microfones e Streams de aplicativos em tempo real.
4. **Controle completo de volume (0..150%) e Mudo** para dispositivos físicos e por aplicativo.
5. **Definição de dispositivo padrão** (Default Sink e Default Source) funcionando via `[Enter]` ou `[*]`.
6. **Identificação de aplicativos com ícones de marca** (Spotify, Discord, Firefox, Steam, etc.) com fallback ASCII.
7. **Degradação graciosa** com avisos claros quando nenhum servidor de som estiver disponível ou quando nenhum app estiver tocando.
8. **Suíte de testes automatizados unitários e de integração** validando parsing, renderização e fluxo de eventos.
