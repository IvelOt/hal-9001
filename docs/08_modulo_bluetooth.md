# 08 — Módulo de Bluetooth (Aba 3 — Bluetooth)

> HAL-9001 — Planejamento arquitetural do Módulo 3 (Bluetooth) sobre o
> **BlueZ** via D-Bus, em Rust 100% puro (`zbus`).
> **Este documento é uma especificação arquitetural e plano de execução.**
> O objetivo é fixar arquitetura, contratos de mensagem, máquinas de estado,
> UX da TUI (Ratatui), telemetria de bateria (`Battery1`), garantias de
> degradação graciosa e a decomposição em tarefas atômicas para o Kanban.

---

## 0. Contexto e Estado Atual

O repositório `hal-9001` possui os seguintes *stubs* para Bluetooth:

- `src/backend/bluetooth.rs` — registra `ServiceDegraded` via
  `pending_stub("bluetooth", "Módulo 3 (bluez)", tx)`.
- `src/ui/bluetooth.rs` — renderiza `draw_pending(...)` com o placeholder da
  Aba 3 anunciando os atalhos previstos.

O fluxo de dados unidirecional da aplicação (ver `docs/01_arquitetura_e_stack.md`):

```
backend workers ──AppEvent(mpsc)──▶ App (estado) ──Action(broadcast)──▶ backend workers
                                        │
                                   ui::draw(&App, Frame)  (função pura, tick-driven)
```

### Regras Inegociáveis Herdadas

1. **100% Pure Rust com `zbus`** — toda a comunicação com o daemon `bluetoothd`
   é feita via D-Bus assíncrono no System Bus (`org.bluez`).
2. **Zero dependências C externas** — proibido o uso de `libbluetooth`, `bluez-libs`,
   `glib`, `dbus-sys` ou wrappers CLI de shell como `bluetoothctl` / `hciconfig`.
3. **UI nunca bloqueia** — todo I/O D-Bus, chamadas a métodos e escuta de sinais
   rodam em background workers Tokio. A thread de render é 100% síncrona e
   apenas lê o estado imutável em `&App`.
4. **Estado Único** — `App` centraliza o `BluetoothSnapshot`; a UI é uma função
   pura `fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect)`.
5. **Sem `Arc<Mutex<...>>` entre UI e backend** — comunicação estritamente por
   canais Tokio (`mpsc::UnboundedSender<AppEvent>` e `broadcast::Receiver<Action>`).
6. **Degradação Graciosa** — na ausência do daemon `org.bluez`, sem adaptadores
   de rádio presentes ou com rádio desligado (soft rfkill / `Powered=false`), o
   módulo entra em modo degradado/desligado com feedback visual claro, sem pânico.
7. **Zero Emojis Policy** — nenhum emoji hardcoded no binário; ícones usam Nerd
   Fonts quando `config.ui.icons == true`, com fallback ASCII textual rígido
   (ex.: `[FONE]`, `[PAD]`, `[TECL]`, `[MOUS]`, `[CEL]`, `[DEV]`).
8. **i18n** — strings da interface usam a política de internacionalização (veja `AGENTS.md`)
   (pt-BR / en-US / es-ES).
9. **Zero Warnings** — `cargo clippy -- -D warnings` limpo como critério de aceite.

> **Grounding em Sistema Real:**
> Esta especificação foi inspecionada e validada diretamente contra o daemon
> **BlueZ 5.7x** no System D-Bus do host Linux, cobrindo o adaptador `/org/bluez/hci0`
> e dispositivos reais pareados e conectados (ex: fones TWS, fones de ouvido Bluetooth
> e periféricos de entrada).

---

## 1. Visão Geral da Arquitetura do Módulo

O Módulo 3 divide-se em três subsistemas essenciais:

| Subsistema | Responsabilidade | Superfície de Risco |
|------------|------------------|---------------------|
| **A. Descoberta & Inventário** | Enumeração de adaptadores (`Adapter1`), escuta do `ObjectManager`, scan de dispositivos BLE e Clássicos (`StartDiscovery`/`StopDiscovery`), monitoramento de RSSI e CoD (*Class of Device*). | Baixa (read-only / temporizado). |
| **B. Ciclo de Vida do Dispositivo** | Pareamento (`Pair`), conexão/desconexão (`Connect`/`Disconnect`), esquecer/remover do inventário (`RemoveDevice`), bloqueio/desbloqueio (`Blocked`), toggle de rádio (`Powered`). | Média (altera estado do subsistema de rádio do kernel e segurança). |
| **C. Telemetria & Periféricos** | Monitoramento de nível de bateria via `org.bluez.Battery1`, perfis de áudio (A2DP / HFP / AVRCP) e codecs ativos. | Baixa (read-only reativo). |

### 1.1 Diagrama de Threads e Canais

```
                          ┌──────────────────────────────────────────────────────────┐
                          │               backend::bluetooth::run                     │
                          │                                                          │
  BlueZ Signals ─────────▶│  ObjectManager: InterfacesAdded / InterfacesRemoved      │
  (D-Bus Reativo)         │  PropertiesChanged (Connected, RSSI, Battery1, Powered)  │──┐
                          │                                                          │  │
  Action (broadcast) ────▶│  Dispatcher: Connect, Disconnect, Pair, Rescan,          │  │ AppEvent (mpsc)
                          │              ToggleRadio, Forget, ToggleBlock            │  ▼
                          │                                                          │  App.bluetooth
  Interval(scan_timeout) ─▶│  Auto-stop do scan após 30s (economia de energia)        │  (BluetoothSnapshot)
  Interval(poll_fallback)─▶│  Reconciliação periódica de fallback (10s)              │
                          └──────────────────────────────────────────────────────────┘
```

---

## 2. Modelo D-Bus do BlueZ

- **Serviço de Sistema:** `org.bluez`
- **Bus:** System Bus (`Connection::system()`)
- **Raiz do Gerenciador de Objetos:** `/` com interface `org.freedesktop.DBus.ObjectManager`
- **Caminhos de Adaptadores:** `/org/bluez/hci0`, `/org/bluez/hci1`, etc.
- **Caminhos de Dispositivos:** `/org/bluez/hciX/dev_XX_XX_XX_XX_XX_XX`

### 2.1 Tabela de Interfaces, Métodos e Propriedades

| Interface D-Bus | Caminho | Membros Utilizados |
|-----------------|---------|-------------------|
| `org.freedesktop.DBus.ObjectManager` | `/` | Métodos: `GetManagedObjects() -> a{oa{sa{sv}}}`<br>Sinais: `InterfacesAdded(o, a{sa{sv}})`, `InterfacesRemoved(o, as)` |
| `org.bluez.Adapter1` | `/org/bluez/hci0` | **Props:** `Address` (`s`), `Name` (`s`), `Alias` (`s`, writable), `Class` (`u`), `Powered` (`b`, writable), `Discovering` (`b`), `Discoverable` (`b`, writable), `Pairable` (`b`, writable), `UUIDs` (`as`).<br>**Métodos:** `StartDiscovery()`, `StopDiscovery()`, `RemoveDevice(o)`, `SetDiscoveryFilter(a{sv})`. |
| `org.bluez.Device1` | `/org/bluez/hciX/dev_XX...` | **Props:** `Address` (`s`), `Name` (`s`), `Alias` (`s`, writable), `Class` (`u`), `Appearance` (`q`), `Icon` (`s`), `Paired` (`b`), `Bonded` (`b`), `Trusted` (`b`, writable), `Blocked` (`b`, writable), `Connected` (`b`), `RSSI` (`n`), `TxPower` (`n`), `UUIDs` (`as`), `Adapter` (`o`), `ServicesResolved` (`b`).<br>**Métodos:** `Connect()`, `Disconnect()`, `Pair()`, `CancelPairing()`, `ConnectProfile(s)`, `DisconnectProfile(s)`. |
| `org.bluez.Battery1` | `/org/bluez/hciX/dev_XX...` | **Props:** `Percentage` (`y`, byte 0..100), `Source` (`s`, opcional). |
| `org.bluez.MediaControl1` | `/org/bluez/hciX/dev_XX...` | **Props:** `Connected` (`b`), `Player` (`o`). |
| `org.freedesktop.DBus.Properties` | Todos os objetos | **Métodos:** `Get(ss) -> v`, `Set(ssv)`, `GetAll(s) -> a{sv}`<br>**Sinais:** `PropertiesChanged(s, a{sv}, as)` |

### 2.2 Formato das Estruturas D-Bus em Rust (`zvariant`)

O método `GetManagedObjects` retorna o tipo complexo:
```rust
type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
```
Onde:
1. `OwnedObjectPath`: caminho do nó D-Bus (ex: `/org/bluez/hci0/dev_34_09_C9_00_77_CD`).
2. `HashMap<String, ...>`: nome da interface (ex: `org.bluez.Device1`, `org.bluez.Battery1`).
3. `HashMap<String, OwnedValue>`: mapa chave-valor das propriedades do objeto.

---

## 3. Classificação, CoD (Class of Device), Ícones e Modelo de Dados

### 3.1 Categoria de Dispositivo (`BluetoothDeviceType`)

A categoria do dispositivo é determinada por uma função pura que analisa de forma determinística:
1. A propriedade `Icon` do BlueZ (`org.bluez.Device1.Icon`).
2. O campo binário de 24 bits `Class` (*Bluetooth Class of Device* - CoD).
3. O campo `Appearance` de 16 bits para dispositivos BLE.
4. A lista de `UUIDs` de serviços anunciados.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BluetoothDeviceType {
    Headset,   // Fones TWS, Headphones, Headsets, Caixas de Som
    Gamepad,   // Controles de console, Joysticks, Gamepads
    Keyboard,  // Teclados Bluetooth / BLE
    Mouse,     // Mouses, Trackpads, Apontadores
    Phone,     // Smartphones, Celulares
    Computer,  // PCs, Laptops, Servidores
    Other,     // Periféricos diversos, sensores, wearables
}

impl BluetoothDeviceType {
    pub fn label(self) -> &'static str {
        match self {
            Self::Headset => "Áudio/Fone",
            Self::Gamepad => "Controle/Gamepad",
            Self::Keyboard => "Teclado",
            Self::Mouse => "Mouse/Trackpad",
            Self::Phone => "Smartphone",
            Self::Computer => "Computador",
            Self::Other => "Dispositivo",
        }
    }

    /// Retorna o ícone Nerd Font ou o fallback ASCII (Zero Emojis Policy)
    pub fn icon_badge(self, use_nerd_icons: bool) -> &'static str {
        if use_nerd_icons {
            match self {
                Self::Headset => "\u{f025}",   // 󰋋 nf-fa-headphones
                Self::Gamepad => "\u{f11b}",   // 󰊴 nf-fa-gamepad
                Self::Keyboard => "\u{f11c}",  // 󰌌 nf-fa-keyboard_o
                Self::Mouse => "\u{f87c}",     // 󰍽 nf-md-mouse
                Self::Phone => "\u{f10b}",     // 󰄜 nf-fa-mobile
                Self::Computer => "\u{f108}",  // 󰌢 nf-fa-desktop
                Self::Other => "\u{f293}",     // 󰂯 nf-fa-bluetooth
            }
        } else {
            match self {
                Self::Headset => "[FONE]",
                Self::Gamepad => "[PAD]",
                Self::Keyboard => "[TECL]",
                Self::Mouse => "[MOUS]",
                Self::Phone => "[CEL]",
                Self::Computer => "[PC]",
                Self::Other => "[DEV]",
            }
        }
    }
}
```

### 3.2 Algoritmo de Derivação de Tipo (Função Pura e Testável)

```rust
// UUIDs conhecidos de serviços Bluetooth
pub const UUID_A2DP_SINK: &str     = "0000110b-0000-1000-8000-00805f9b34fb";
pub const UUID_A2DP_SOURCE: &str   = "0000110a-0000-1000-8000-00805f9b34fb";
pub const UUID_AVRCP_TARGET: &str  = "0000110c-0000-1000-8000-00805f9b34fb";
pub const UUID_HEADSET: &str       = "00001108-0000-1000-8000-00805f9b34fb";
pub const UUID_HANDSFREE: &str     = "0000111e-0000-1000-8000-00805f9b34fb";
pub const UUID_HID: &str           = "00001124-0000-1000-8000-00805f9b34fb";
pub const UUID_HOGP: &str          = "00001812-0000-1000-8000-00805f9b34fb"; // HID over GATT

pub fn derive_device_type(
    icon: Option<&str>,
    class: Option<u32>,
    appearance: Option<u16>,
    uuids: &[String],
) -> BluetoothDeviceType {
    // 1. Prioridade para a propriedade explícita de ícone do BlueZ
    if let Some(ic) = icon {
        match ic {
            "audio-headset" | "audio-headphones" | "audio-card" | "audio-speaker" => {
                return BluetoothDeviceType::Headset;
            }
            "input-gaming" => return BluetoothDeviceType::Gamepad,
            "input-keyboard" => return BluetoothDeviceType::Keyboard,
            "input-mouse" | "input-tablet" => return BluetoothDeviceType::Mouse,
            "phone" => return BluetoothDeviceType::Phone,
            "computer" => return BluetoothDeviceType::Computer,
            _ => {}
        }
    }

    // 2. Análise do Bluetooth Class of Device (CoD - bits 8..12 = Major Device Class)
    if let Some(cod) = class {
        let major = (cod >> 8) & 0x1F;
        let minor = (cod >> 2) & 0x3F;

        match major {
            0x01 => return BluetoothDeviceType::Computer,
            0x02 => return BluetoothDeviceType::Phone,
            0x04 => return BluetoothDeviceType::Headset, // Audio/Video
            0x05 => { // Peripheral
                if minor & 0x10 != 0 || minor == 0x01 || minor == 0x02 {
                    return BluetoothDeviceType::Gamepad; // Joystick / Gamepad
                }
                if minor & 0x04 != 0 {
                    return BluetoothDeviceType::Keyboard;
                }
                if minor & 0x08 != 0 {
                    return BluetoothDeviceType::Mouse;
                }
            }
            _ => {}
        }
    }

    // 3. Análise do BLE Appearance
    if let Some(app) = appearance {
        let category = app >> 6;
        match category {
            15 => return BluetoothDeviceType::Gamepad, // Category 15: HID Gamepad/Joystick
            16 => return BluetoothDeviceType::Keyboard, // Category 16: Keyboard
            17 => return BluetoothDeviceType::Mouse,    // Category 17: Mouse
            33 => return BluetoothDeviceType::Headset,  // Category 33: Media Player / Audio
            _ => {}
        }
    }

    // 4. Fallback por UUIDs de serviços anunciados
    if uuids.iter().any(|u| {
        let s = u.to_ascii_lowercase();
        s == UUID_A2DP_SINK || s == UUID_HEADSET || s == UUID_HANDSFREE
    }) {
        return BluetoothDeviceType::Headset;
    }
    if uuids.iter().any(|u| {
        let s = u.to_ascii_lowercase();
        s == UUID_HID || s == UUID_HOGP
    }) {
        return BluetoothDeviceType::Keyboard;
    }

    BluetoothDeviceType::Other
}
```

### 3.3 Modelo de Dados do Snapshot (`BluetoothSnapshot`)

O backend publica instâncias imutáveis de `BluetoothSnapshot` envelopadas em `AppEvent::Bluetooth(Box<BluetoothSnapshot>)`:

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct BluetoothSnapshot {
    pub bluez_available: bool,
    pub adapter: Option<BluetoothAdapter>,
    pub devices: Vec<BluetoothDevice>,
    pub is_scanning: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BluetoothAdapter {
    pub id: DeviceId,            // Object path, ex: "/org/bluez/hci0"
    pub name: String,            // "IvelPC"
    pub address: String,         // "18:93:41:63:82:BE"
    pub powered: bool,           // Rádio ligado/desligado
    pub pairable: bool,
    pub discoverable: bool,
    pub is_discovering: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BluetoothDevice {
    pub id: DeviceId,            // Object path, ex: "/org/bluez/hci0/dev_34_09_C9_00_77_CD"
    pub address: String,         // "34:09:C9:00:77:CD"
    pub name: String,            // Nome anunciado ou fallback para Alias/Endereço
    pub alias: String,           // Alias local configurado
    pub device_type: BluetoothDeviceType,
    pub paired: bool,
    pub connected: bool,
    pub trusted: bool,
    pub blocked: bool,
    pub rssi: Option<i16>,       // Sinal em dBm (-100..0)
    pub battery_pct: Option<u8>, // Percentual de bateria (0..100 via Battery1)
    pub audio_connected: bool,   // Perfil de mídia ativo
    pub uuids: Vec<String>,
}

impl BluetoothDevice {
    /// Nome amigável para exibição na UI
    pub fn display_name(&self) -> &str {
        if !self.alias.trim().is_empty() {
            &self.alias
        } else if !self.name.trim().is_empty() {
            &self.name
        } else {
            &self.address
        }
    }

    /// Cálculo percentual aproximado da qualidade de sinal baseado no RSSI (dBm)
    pub fn signal_quality(&self) -> Option<u8> {
        let rssi = self.rssi?;
        // Mapeia faixa típica de -100 dBm (0%) a -40 dBm (100%)
        let clamped = rssi.clamp(-100, -40);
        let pct = ((clamped + 100) as f32 / 60.0 * 100.0).round() as u8;
        Some(pct.min(100))
    }
}
```

---

## 4. Telemetria de Bateria e Periféricos (`org.bluez.Battery1`)

Quando um dispositivo periférico (especialmente fones TWS, headsets e mouses) se conecta, o daemon BlueZ expõe dinamicamente a interface `org.bluez.Battery1` no mesmo caminho D-Bus do dispositivo.

- **Propriedade Principal:** `Percentage` (`y`, u8 de 0 a 100).
- **Sinais:** `PropertiesChanged` sobre a interface `org.bluez.Battery1` dispara atualizações em tempo real quando o nível de carga se altera.
- **Detecção de desconexão:** Quando o dispositivo é desconectado, o BlueZ emite o sinal `InterfacesRemoved` contendo `org.bluez.Battery1`, limpando o campo `battery_pct = None` no modelo.

### Formatação Visual de Bateria (Função Pura)

```rust
pub fn format_battery_badge(battery: Option<u8>) -> String {
    match battery {
        Some(pct) => {
            let bar = match pct {
                0..=20 => " ",
                21..=40 => "▃",
                41..=60 => "▅",
                61..=80 => "▆",
                _ => "█",
            };
            format!("[{bar}] {pct:>3}%")
        }
        None => "—".to_string(),
    }
}
```

---

## 5. Máquinas de Estado e Ciclos de Vida

### 5.1 Descoberta / Scan de Dispositivos (`StartDiscovery` / `StopDiscovery`)

```
                  ┌────────────────────────────────────────────────────────┐
                  │                                                        │
                  ▼                                                        │
┌──────────────────┐           Action::BluetoothRescan           ┌─────────────────┐
│     IDLE         │ ───────────────────────────────────────────▶│    SCANNING     │
│ (Discovering: F) │                                             │ (Discovering: T)│
└──────────────────┘◀────────────────────────────────────────────└─────────────────┘
                            Auto-timeout (30s) / Action::Rescan
```

1. Ao receber `Action::BluetoothRescan`:
   - Se o adaptador estiver com `Discovering == true`, chama `StopDiscovery()` imediatamente.
   - Se `Discovering == false`, chama `StartDiscovery()`, emite `AppEvent::BluetoothScanning(true)` e agenda um timer de segurança de 30 segundos no Tokio para chamar `StopDiscovery()` automaticamente, evitando drenar a bateria do host e do rádio.
2. Durante o scan, sinais `InterfacesAdded` populam novos `BluetoothDevice`s descobertos. Sinais `PropertiesChanged` atualizam o `RSSI` em tempo real.

### 5.2 Conexão e Desconexão (`Connect` / `Disconnect`)

```
                  ┌───────────────── Action::BluetoothConnect(id) ───────────────┐
                  ▼                                                              │
┌──────────────────┐               Connect() OK                 ┌─────────────────┐
│  DESCONECTADO    │ ──────────────────────────────────────────▶│    CONECTADO    │
│ (Connected: F)   │                                             │ (Connected: T)  │
└──────────────────┘◀───────────────────────────────────────────└─────────────────┘
                      Disconnect() / Disconnected Signal / Timeout
```

1. **Conectar (`Enter` no item desconectado):** invoca `org.bluez.Device1.Connect()`. Se o dispositivo não estiver pareado, o BlueZ automaticamente inicia o pareamento ("Just Works" ou agente padrão).
2. **Desconectar (`Enter` no item conectado):** invoca `org.bluez.Device1.Disconnect()`.
3. **Tratamento de Timeout:** chamadas de conexão D-Bus possuem timeout configurado de 15 segundos para evitar que falhas de RF travem a task.

### 5.3 Pareamento Explícito (`Action::BluetoothPair`)

- Acionado pela tecla `[p]`.
- Invoca `org.bluez.Device1.Pair()`.
- Emite toast informativo `"Pareando com <dispositivo>..."`.
- Ao concluir com sucesso, altera a propriedade `Trusted = true` via D-Bus para permitir reconexão automática futura sem atrito.

### 5.4 Esquecer / Remover Dispositivo (`Action::BluetoothForget`)

- Acionado pela tecla `[f]`.
- Invoca `org.bluez.Adapter1.RemoveDevice(ObjectPath)`.
- O dispositivo é imediatamente purgado do cache do BlueZ e a lista de dispositivos é atualizada na UI.

### 5.5 Bloqueio / Desbloqueio (`Action::BluetoothToggleBlock`)

- Acionado pela tecla `[b]`.
- Lê o estado atual `Blocked` (`b`) e escreve o inverso em `org.bluez.Device1.Blocked`.
- Dispositivos bloqueados são impedidos pelo kernel de estabelecer conexão.

### 5.6 Toggle de Rádio do Adaptador (`Action::BluetoothToggleRadio`)

- Acionado pela tecla `[t]`.
- Escreve `Powered = !Powered` na interface `org.bluez.Adapter1`.
- Ao desligar, todos os dispositivos conectados passam para o estado desconectado.

---

## 6. Interface TUI (Ratatui — Aba 3 `src/ui/bluetooth.rs`)

### 6.1 Layout Responsivo em 3 Blocos Verticais

```
┌─ Bluetooth & Dispositivos ───────────────────────────────────────────────────┐
│ Adaptador: hci0 [18:93:41:63:82:BE]  Rádio: [● LIGADO]   [BUSCANDO...] (12s) │
├──────────────────────────────────────────────────────────────────────────────┤
│  St  Tipo    Nome / Dispositivo           Endereço MAC       Sinal    Bateria│
│ ──────────────────────────────────────────────────────────────────────────── │
│ ● ▶  [FONE]  soundcore P20i               34:09:C9:00:77:CD  [████]   [████] │
│      [PAD]   Xbox Wireless Controller     5C:BA:37:44:11:02  [███░]    —     │
│      [TECL]  Keychron K2                  DC:2C:26:A1:88:99  [████]   [███░] │
│      [CEL]   Galaxy S24                   A4:75:B3:90:12:34  [██░░]    —     │
│      [DEV]   Dispositivo Próximo          78:23:44:12:99:AA  [█░░░]    —     │
├──────────────────────────────────────────────────────────────────────────────┤
│ Telemetria: soundcore P20i | Bateria: 80% | Áudio A2DP Ativo (SBC/AAC)       │
│ [Enter] Conectar/Desconectar [p] Parear [r] Escanear [f] Esquecer [b] Bloq  │
└──────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 Estrutura da Tabela de Dispositivos

1. **Ordenação das Linhas:**
   - Dispositivos **Conectados** primeiro (`Connected == true`, marcador `●`).
   - Dispositivos **Pareados** em seguida (`Paired == true`).
   - Dispositivos **Descobertos / Não-pareados** por último, ordenados por intensidade de `RSSI` decrescente.
2. **Colunas:**
   - **Status (St):** Marcador `●` em verde para conectado, `○` para desconectado, `🔒` ou `[B]` para bloqueado. Indicador de seleção `▶`.
   - **Tipo:** Ícone Nerd Font (`󰋋`, `󰊴`, `󰌌`, etc.) ou badge ASCII (`[FONE]`, `[PAD]`, `[TECL]`).
   - **Nome:** Nome anunciado ou alias amigável.
   - **Endereço MAC:** Endereço físico do rádio.
   - **Sinal:** Barra gráfica de RSSI (`[████] 85%`) com cores dinâmicas (Verde >= 70%, Amarelo >= 40%, Vermelho < 40%).
   - **Bateria:** Porcentagem vinda de `Battery1` (`[████] 80%` ou `—`).

### 6.3 Atalhos de Teclado e Mapeamento de Contexto

| Tecla | Ação | Contexto |
|-------|------|----------|
| `j`/`k` ou `↑`/`↓` | Navegar pela tabela de dispositivos | Aba Bluetooth ativa |
| `Enter` | Alternar Conexão (Conecta se desconectado / Desconecta se conectado) | Dispositivo selecionado |
| `p` | Parear dispositivo selecionado | Dispositivo selecionado |
| `r` | Iniciar / Parar Scan de Descoberta | Adaptador ligado |
| `f` | Esquecer / Remover dispositivo do inventário | Dispositivo selecionado |
| `t` | Alternar Rádio do Adaptador (Ligar/Desligar) | Sempre |
| `b` | Alternar Bloqueio do dispositivo | Dispositivo selecionado |

### 6.4 Estados Degradados na Interface

| Condição | Exibição na Interface |
|----------|----------------------|
| **BlueZ não está rodando** | Renderiza painel `ServiceDegraded`: *"Serviço org.bluez não encontrado no D-Bus do sistema. Verifique se bluetoothd está ativo."* |
| **Nenhum adaptador encontrado** | Header exibe `Adaptador: Nenhum rádio encontrado` e a tabela mostra *"Conecte um dongle ou ative o controlador Bluetooth na BIOS/firmware."* |
| **Rádio desligado (`Powered=false`)** | Header exibe `Rádio: [○ DESLIGADO]` e a tabela exibe aviso *"O rádio Bluetooth está desligado. Pressione [t] para ligar."* |
| **Scan ativo sem dispositivos** | Exibe spinner animado e mensagem *"Procurando dispositivos Bluetooth próximos..."* |

---

## 7. Contratos de Mensagem

### 7.1 Novos Variantes em `src/events/mod.rs`

```rust
// Em AppEvent:
/// Snapshot completo do estado do Bluetooth (Módulo 3). Boxed para evitar inflar o enum.
Bluetooth(Box<crate::backend::bluetooth::BluetoothSnapshot>),
/// Indicador de estado de escaneamento Bluetooth ativo.
BluetoothScanning(bool),

// Em Action:
BluetoothRescan,
BluetoothToggleRadio,
BluetoothConnect(DeviceId),
BluetoothDisconnect(DeviceId),
BluetoothPair(DeviceId),
BluetoothForget(DeviceId),
BluetoothToggleBlock(DeviceId),
```

### 7.2 Mapeamento de Teclas em `src/events/input.rs`

```rust
// Dentro do mapeador de teclas quando active == Tab::Bluetooth:
if active == Tab::Bluetooth {
    match key.code {
        KeyCode::Char('r') => return Some(Action::BluetoothRescan),
        KeyCode::Char('t') => return Some(Action::BluetoothToggleRadio),
        KeyCode::Char('p') => return Some(Action::BluetoothPair(DeviceId(String::new()))),
        KeyCode::Char('f') => return Some(Action::BluetoothForget(DeviceId(String::new()))),
        KeyCode::Char('b') => return Some(Action::BluetoothToggleBlock(DeviceId(String::new()))),
        _ => {}
    }
}
```

### 7.3 Extensão de `App` em `src/app.rs`

```rust
// Campos adicionados à struct App:
pub bluetooth: Option<Box<crate::backend::bluetooth::BluetoothSnapshot>>,
pub bluetooth_selected: usize,
pub bluetooth_scanning: bool,
```

No método `handle_event`:
```rust
AppEvent::Bluetooth(snap) => {
    let dev_count = snap.devices.len();
    self.bluetooth = Some(snap);
    if dev_count > 0 && self.bluetooth_selected >= dev_count {
        self.bluetooth_selected = dev_count - 1;
    }
}
AppEvent::BluetoothScanning(flag) => {
    self.bluetooth_scanning = flag;
}
```

No método `dispatch`:
```rust
// Resolução da seleção atual para ações contextuais:
Action::BluetoothPair(_) => {
    if let Some(snap) = &self.bluetooth {
        if let Some(dev) = snap.devices.get(self.bluetooth_selected) {
            let _ = action_tx.send(Action::BluetoothPair(dev.id.clone()));
        }
    }
}
Action::BluetoothForget(_) => {
    if let Some(snap) = &self.bluetooth {
        if let Some(dev) = snap.devices.get(self.bluetooth_selected) {
            let _ = action_tx.send(Action::BluetoothForget(dev.id.clone()));
        }
    }
}
Action::BluetoothToggleBlock(_) => {
    if let Some(snap) = &self.bluetooth {
        if let Some(dev) = snap.devices.get(self.bluetooth_selected) {
            let _ = action_tx.send(Action::BluetoothToggleBlock(dev.id.clone()));
        }
    }
}
Action::Enter if self.active == Tab::Bluetooth => {
    if let Some(snap) = &self.bluetooth {
        if let Some(dev) = snap.devices.get(self.bluetooth_selected) {
            if dev.connected {
                let _ = action_tx.send(Action::BluetoothDisconnect(dev.id.clone()));
            } else {
                let _ = action_tx.send(Action::BluetoothConnect(dev.id.clone()));
            }
        }
    }
}
```

---

## 8. Limites de Escopo v1 (Documentados)

| Fora do Escopo v1 | Justificativa / Roadmap |
|-------------------|-------------------------|
| **Agente Interativo de PIN com Código Customizado** | A v1 suporta "Just Works", pareamento com confirmação simples e dispositivos que usam PIN padrão (`0000`/`1234`). Entrada de PIN manual na TUI é roadmap. |
| **Múltiplos Adaptadores Concorrentes** | A v1 seleciona o primeiro adaptador funcional (`hci0`). Seletor de adaptador é roadmap. |
| **GATT Custom Explorer / Leitura de Características Arbitrárias** | A v1 foca em áudio, periféricos HID, telemetria de bateria e conectividade padrão. Leitura de árvores GATT brutas fica para versão avançada. |
| **Transferência de Arquivos via OBEX** | Fora do escopo do HAL-9001 (utilizar cliente dedicado). |

---

## 9. Dependências e Ferramentas

| Componente | Tipo | Papel | Status |
|------------|------|-------|--------|
| `zbus` | Crate Rust | Comunicação assíncrona D-Bus com `org.bluez` e `ObjectManager` | **Já presente no `Cargo.toml`** |
| `tokio` | Crate Rust | Runtime assíncrono, timers de scan e canais de mensagens | **Já presente no `Cargo.toml`** |
| `ratatui` | Crate Rust | Renderização da TUI, tabelas e headers | **Já presente no `Cargo.toml`** |
| `bluetoothd` | Daemon Host | Daemon do BlueZ no Linux (`bluetooth.service`) | Runtime do sistema operacional |

> **Garantia Arquitetural:** Zero adições de crates C, FFI ou invocação de processos externos.

---

## 10. Estratégia de Testes

| Nível de Teste | Alvo / Escopo |
|----------------|---------------|
| **Unitário (Parsers Puros)** | - `derive_device_type`: classificação precisa para todas as classes CoD (Audio, Gamepad, Teclado, Mouse, Celular), BLE Appearances e UUIDs.<br>- `BluetoothDevice::signal_quality`: conversão exata de RSSI (dBm) para percentual (0..100).<br>- `format_battery_badge`: formatação de baterias normais, baixas e ausentes.<br>- Deduplicação e ordenação da lista de dispositivos (conectados no topo, seguidos por pareados e depois por sinal). |
| **Unitário (Máquinas de Estado)** | - Transições de `Action` para comandos D-Bus.<br>- Tratamento do auto-timeout de 30s para `StopDiscovery`.<br>- Alternância de conexão e bloqueio. |
| **Integração (Mock D-Bus)** | - Testes com mock publicando payloads sintéticos de `GetManagedObjects`, simulando `InterfacesAdded` (novo dispositivo detectado) e `PropertiesChanged` (atualização de bateria ou RSSI). |
| **Smoke Test** | - Execução headless com `TERM=dumb` em ambiente sem rádio Bluetooth ou sem D-Bus: verificação de que o app degrada graciosamente e não causa *panic*. |
| **E2E / Manual** | - Conexão e desconexão real de fone TWS (ex: Soundcore / Thinkplus).<br>- Validação da exibição da bateria em tempo real.<br>- Rescan e pareamento de novos dispositivos. |

---

## 11. Plano de Implementação Modular & Decomposição em Tasks (Kanban)

### Épico A — Modelos e Parsers Puros (Sem risco, 100% testável)
- **A1.** Definir `BluetoothSnapshot`, `BluetoothAdapter`, `BluetoothDevice` e `BluetoothDeviceType` em `src/backend/bluetooth.rs`.
- **A2.** Implementar `derive_device_type`, `signal_quality`, `format_battery_badge` e testes unitários exaustivos.
- **A3.** Adicionar variantes `AppEvent::Bluetooth` e `AppEvent::BluetoothScanning` em `src/events/mod.rs` e plugar no `App::handle_event`.

### Épico B — Conexão D-Bus & Introspecção via `ObjectManager`
- **B1.** Implementar `collect_snapshot` com `Connection::system()` lendo `org.freedesktop.DBus.ObjectManager.GetManagedObjects` em `/`.
- **B2.** Parser para extrair o adaptador primário (`org.bluez.Adapter1`), propriedades `Powered`, `Discovering`, `Address` e `Name`.
- **B3.** Parser para extrair a lista de dispositivos (`org.bluez.Device1`) e telemetria de bateria (`org.bluez.Battery1`).

### Épico C — Loop do Worker Tokio & Sinais Reativos
- **C1.** Configurar listener de sinais `InterfacesAdded` e `InterfacesRemoved` do `ObjectManager`.
- **C2.** Configurar listener de `PropertiesChanged` para atualizações dinâmicas de `RSSI`, `Connected`, `Powered` e `Battery1.Percentage`.
- **C3.** Montar o loop principal com `tokio::select!` integrando intervalos periódicos, sinais D-Bus e canal `broadcast<Action>`.

### Épico D — Ações de Controle do Adaptador e Scan
- **D1.** Implementar `Action::BluetoothToggleRadio` (`Adapter1.Powered = !Powered`).
- **D2.** Implementar `Action::BluetoothRescan` (`Adapter1.StartDiscovery` / `StopDiscovery`) com auto-stop de 30s.
- **D3.** Tratamento de erros e toasts na statusline para falhas de operação no adaptador.

### Épico E — Ações de Dispositivo (Conectar, Parear, Remover, Bloquear)
- **E1.** Implementar `Action::BluetoothConnect` e `Action::BluetoothDisconnect` com timeouts de proteção.
- **E2.** Implementar `Action::BluetoothPair` e configuração automática de `Trusted = true`.
- **E3.** Implementar `Action::BluetoothForget` (`Adapter1.RemoveDevice`) e `Action::BluetoothToggleBlock` (`Device1.Blocked`).

### Épico F — Interface de Usuário (Aba 3 Ratatui em `src/ui/bluetooth.rs`)
- **F1.** Implementar `draw_header` exibindo nome do adaptador, MAC, status do rádio `[● LIGADO]` e badge de scan `[BUSCANDO...]`.
- **F2.** Implementar `draw_device_table` com colunas (Status, Tipo, Nome/MAC, Sinal, Bateria) e destaque da linha selecionada.
- **F3.** Implementar `draw_footer` com telemetria do dispositivo selecionado (Bateria, Codec/Áudio) e legenda de atalhos.
- **F4.** Implementar tratamento de telas compactas/estreitas e renderização de estados degradados.

### Épico G — Integração de Teclado e Roteamento no App
- **G1.** Configurar `map_key` em `src/events/input.rs` para capturar `[Enter]`, `[p]`, `[r]`, `[f]`, `[t]`, `[b]` na Aba Bluetooth.
- **G2.** Implementar roteamento das ações em `App::dispatch` resolvendo a linha selecionada para o `DeviceId` concreto.

### Épico H — Validação, Testes e Fechamento
- **H1.** Criar suíte de testes de integração com mock do D-Bus para sinais e comandos.
- **H2.** Validação com `cargo clippy -- -D warnings` e testes headless.
- **H3.** Testes manuais em hardware real com dispositivos Bluetooth reais.
- **H4.** Atualização de `docs/02_especificacao_das_abas.md` e `docs/03_plano_de_execucao_modular.md`.

### Grafo de Dependências

```
Épico A (Modelos) ──▶ Épico B (ObjectManager) ──▶ Épico C (Worker & Sinais)
       │                                                   │
       ├──▶ Épico F (UI Ratatui) ◀─────────────────────────┼──▶ Épico D (Ações Rádio/Scan)
       │          │                                        │          │
       │          ▼                                        ▼          ▼
       └──▶ Épico G (Keymap & Dispatch) ◀────────────── Épico E (Ações Dispositivo)
                  │
                  ▼
            Épico H (Validação & Fechamento)
```

---

## 12. Riscos & Mitigações

| Risco | Impacto | Mitigação |
|-------|---------|-----------|
| **Scan infinito esgotando bateria/CPU do host** | Alto consumo de energia e ruído de rádio. | O backend impõe auto-stop obrigatório de 30 segundos com timer assíncrono no Tokio. |
| **Dispositivo fantasma / TOCTOU** | Falha ao tentar conectar a dispositivo que saiu do alcance de RF. | Chamadas de conexão usam timeout de 15 segundos; falhas emitem Toast claro e não bloqueiam a UI. |
| **Propriedade `Battery1` ausente em dispositivos antigos** | Exibição de dados nulos. | Fallback gracioso exibindo `—` sem quebrar o layout da tabela. |
| **Ausência de agente Polkit em sessões headless** | Erro de autorização ao parear. | Captura de `NotAuthorized` com toast explicativo, mantendo o app responsivo. |
| **Múltiplos adaptadores Bluetooth no host** | Ambiguidade de controle. | Seleção determinística do primeiro adaptador ativo (`hci0`), com logging de advertência se houver outros. |

---

## 13. Definição de Pronto (Módulo 3)

1. **`cargo clippy -- -D warnings` limpo sem nenhum alerta.**
2. **100% Pure Rust sobre `zbus`** — zero bibliotecas C externas ou wrappers de CLI adicionados.
3. **Aba 3 funcional e responsiva** exibindo o estado do adaptador, tabela de dispositivos com ícones (Nerd Font / ASCII), RSSI e nível de bateria via `org.bluez.Battery1`.
4. **Todos os atalhos de ação operacionais:**
   - `[Enter]`: Conectar / Desconectar
   - `[p]`: Parear dispositivo
   - `[r]`: Iniciar / Interromper Scan
   - `[f]`: Esquecer / Remover dispositivo
   - `[t]`: Ligar / Desligar rádio
   - `[b]`: Bloquear / Desbloquear dispositivo
5. **Degradação graciosa completa** em cenários sem BlueZ, sem adaptadores ou com rádio desligado.
6. **Todas as strings visíveis internacionalizadas** através da política definida em `AGENTS.md`.
7. **Suíte de testes automatizados** cobrindo derivação de tipos, formatação de telemetria, máquinas de estado e manuseio de sinais.
