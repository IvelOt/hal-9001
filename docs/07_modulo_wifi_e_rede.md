# 07 — Módulo de Wi-Fi & Rede (Aba 2 — Network)

> HAL-9001 — Planejamento arquitetural do Módulo 2 (Wi-Fi & Rede) sobre o
> **NetworkManager** via D-Bus, em Rust 100% puro (`zbus`).
> **Este documento é somente projeto.** Nenhum código de produção é escrito aqui;
> o objetivo é fixar arquitetura, contratos de mensagem, máquinas de estado,
> UX da TUI (inclusive o modal de senha Wi-Fi), telemetria de rede, garantias de
> degradação graciosa e a decomposição em tarefas atômicas para o Kanban.

---

## 0. Contexto e Estado Atual

O repositório já possui os *stubs* do Módulo 0:

- `src/backend/network.rs` — hoje apenas registra `ServiceDegraded` via
  `pending_stub("network", "Módulo 2 (NetworkManager)", tx)`.
- `src/ui/network.rs` — hoje renderiza `draw_pending(...)` com o placeholder da
  aba, já anunciando os atalhos previstos (`Enter` conectar, `d` desconectar,
  `f` esquecer, `r` rescan, `t` rádio).

O fluxo unidirecional existente (ver `docs/01_arquitetura_e_stack.md`) é:

```
backend workers ──AppEvent(mpsc)──▶ App (estado) ──Action(broadcast)──▶ backend workers
                                        │
                                   ui::draw(&App, Frame)  (função pura, tick-driven)
```

Regras herdadas que este módulo **deve** respeitar (idênticas às do Módulo 4):

1. **UI nunca bloqueia** — todo I/O de D-Bus, leitura de `/sys`, e qualquer
   espera por sinal roda em *tasks* Tokio; a thread de render só lê `App`.
2. **Estado único** — `App` é a fonte da verdade; a UI de rede é
   `fn(&App, &Palette, &mut Frame, Rect)`.
3. **Sem `Arc<Mutex<...>>` entre UI e backend** — comunicação exclusivamente por
   canais (`mpsc<AppEvent>`, `broadcast<Action>`, e um canal dedicado de segredo
   — ver §5).
4. **Degradação graciosa** — sem NetworkManager no host, ou com o rádio Wi-Fi
   desligado (soft/rfkill), a aba entra em modo "indisponível/rádio off", nunca
   derruba o app.
5. **i18n** — todas as strings visíveis passam por `Language::messages()`
   (pt-BR / en-US / es-ES). A entrada `tab_network` já existe no catálogo.
6. **Zero Emojis Policy** — nenhum emoji na base; ícones são Nerd Font quando
   `config.ui.icons == true`, com *fallback* ASCII (ex.: `[WPA2]`, `[LOCKED]`,
   barras de sinal `▂▄▆█`) caso contrário.
7. **Sem warnings** — `cargo clippy -- -D warnings` limpo (Definição de Pronto).

### 0.1 Diretriz mestra: zero dependências C / zero wrappers pesados

Todo o módulo fala **direto** com `org.freedesktop.NetworkManager` via `zbus`
(já presente no `Cargo.toml`). **Proibido**: `libnm`, `glib`, `nm-*` FFI, ou
qualquer wrapper de shell (`nmcli`, `iw`, `wpa_cli`) como dependência de
runtime. `nmcli` é usado **apenas** como ferramenta de diagnóstico/validação
neste documento — nunca invocado pelo binário.

> **Grounding:** este projeto foi validado contra um host real rodando
> **NetworkManager 1.58.1** com um `wlan0` conectado. Todas as assinaturas de
> método, tipos de propriedade e caminhos de objeto citados abaixo foram
> confirmados por introspeção D-Bus nesse host (ver `data/scout-hall9001-wifi-spec/report.md`).

---

## 1. Visão Geral da Arquitetura do Módulo

O Módulo 2 tem **três responsabilidades** que compartilham o mesmo backend e a
mesma aba:

| Subsistema | Descrição | Superfície de risco |
|------------|-----------|---------------------|
| **A. Estado & Scan** | Enumerar dispositivos, achar o Wi-Fi, escanear APs (SSID, sinal, segurança), refletir rádio on/off. | Baixa (read-only). |
| **B. Conexão & Auth** | Conectar (aberta / WPA2 / WPA3-SAE), autenticar via modal de senha, desconectar, esquecer perfil, toggle de rádio. | Média (muda o estado de rede da máquina; senha é dado sensível). |
| **C. Telemetria** | IP/gateway/DNS, taxa de bits do enlace, throughput RX/TX em tempo real. | Baixa (read-only). |

Tudo é orquestrado por uma única task de backend `network::run`, que:

- Mantém uma **conexão `zbus` com o system bus** (`org.freedesktop.NetworkManager`)
  para leitura reativa (sinais) e ações (métodos D-Bus tipados por `call_method`).
- Mantém um **canal dedicado de segredo** (`NetworkSecretTx`, gêmeo do
  `SudoPasswordTx` já existente) para solicitar a senha Wi-Fi ao modal nativo da
  TUI sem trafegá-la pelo `broadcast<Action>` (ver §5.2).
- Consome `Action`s filtrando apenas as de rede.

```
                          ┌──────────────────────────────────────────────────┐
                          │              network::run (task raiz)              │
                          │                                                    │
  NM signals ────────────▶│  StateChanged / AccessPointAdded/Removed /        │
  (D-Bus reativo)         │  PropertiesChanged (WirelessEnabled, ActiveConn)  │──┐
                          │                                                    │  │
  Action (broadcast) ────▶│  dispatcher → método D-Bus (Activate/Deactivate/  │  │ AppEvent (mpsc)
                          │  RequestScan/Enable/Delete)                        │  ▼
                          │                                                    │  App.network
  interval(net_ms)  ─────▶│  rebuild NetworkSnapshot (devices→APs→telemetria)  │  (NetworkSnapshot)
  interval(1000ms)  ─────▶│  telemetria leve: deltas RX/TX (taxa instantânea)  │
                          │                                                    │
  NetworkSecret (oneshot)◀┤  ao precisar de PSK: pede ao modal, recebe Secret  │
                          └──────────────────────────────────────────────────┘
```

### 1.1 Por que NetworkManager (e não `iw`/`wpa_supplicant` direto)

- **Não-root:** o NM já roda como `root` e autoriza ações da sessão ativa via
  Polkit; o HAL-9001 permanece um binário de usuário. Conectar/desconectar de
  uma sessão de desktop tipicamente não pede senha.
- **Reatividade:** sinais D-Bus (`StateChanged`, `AccessPointAdded/Removed`,
  `PropertiesChanged`) eliminam *polling* de `iw scan`/`wpa_cli`.
- **Cobertura:** listar APs, ler segurança (WPA/WPA2/WPA3/OWE), conectar com
  perfil persistido, desconectar, esquecer, toggle de rádio, telemetria IP —
  tudo por métodos e propriedades tipadas.
- **Consistência:** é o mesmo daemon que GNOME/KDE usam; o estado que a TUI
  mostra é sempre coerente com o resto do desktop.

`/sys/class/net/<iface>/statistics/{rx,tx}_bytes` permanece como **leitura de
throughput** barata e 100% Rust puro (sem D-Bus), com a interface
`Device.Statistics` do próprio NM como alternativa pura-D-Bus (§4.3).

---

## 2. Modelo D-Bus do NetworkManager

**Serviço:** `org.freedesktop.NetworkManager`  ·  **Bus:** system  ·
**Objeto raiz:** `/org/freedesktop/NetworkManager`

### 2.1 Interfaces, métodos e propriedades relevantes

| Interface D-Bus | Uso no módulo |
|-----------------|---------------|
| `org.freedesktop.NetworkManager` (raiz) | Props: `Devices` (`ao`), `ActiveConnections` (`ao`), `NetworkingEnabled` (`b`), `WirelessEnabled` (`b`, **writable**), `WirelessHardwareEnabled` (`b`, rfkill), `State` (`u`), `Connectivity` (`u`), `PrimaryConnection` (`o`), `Version` (`s`). Métodos: `GetDevices → ao`, `ActivateConnection(conn:o, dev:o, spec:o) → o`, `AddAndActivateConnection(conn:a{sa{sv}}, dev:o, spec:o) → (o,o)`, `DeactivateConnection(active:o)`, `Enable(b)`. Sinais: `DeviceAdded(o)`, `DeviceRemoved(o)`, `StateChanged(u)`. |
| `org.freedesktop.NetworkManager.Device` | Props: `Interface` (`s`, ex.: `wlan0`), `DeviceType` (`u`, **2 = WIFI**, 1 = ETHERNET), `State` (`u`, `NMDeviceState`), `Managed` (`b`), `ActiveConnection` (`o`), `Ip4Config` (`o`), `HwAddress` (`s`, MAC), `Driver` (`s`). Métodos: `Disconnect()`, `Delete()`. Sinal: `StateChanged(new:u, old:u, reason:u)`. |
| `org.freedesktop.NetworkManager.Device.Wireless` | Props: `ActiveAccessPoint` (`o`), `Bitrate` (`u`, kb/s), `LastScan` (`x`, ms `CLOCK_BOOTTIME`, `-1` se nunca), `Mode` (`u`), `WirelessCapabilities` (`u`). Métodos: `GetAllAccessPoints() → ao`, `RequestScan(options:a{sv})`. Sinais: `AccessPointAdded(o)`, `AccessPointRemoved(o)`. |
| `org.freedesktop.NetworkManager.Device.Statistics` | Props: `RxBytes` (`t`), `TxBytes` (`t`), `RefreshRateMs` (`u`, **writable**; escrever `>0` habilita atualização periódica). Alternativa pura-D-Bus ao sysfs para throughput (§4.3). |
| `org.freedesktop.NetworkManager.AccessPoint` | Props: `Ssid` (`ay`, bytes), `Strength` (`y`, 0..100), `Frequency` (`u`, MHz), `HwAddress` (`s`, BSSID), `Flags` (`u`, `NM80211ApFlags`), `WpaFlags` (`u`), `RsnFlags` (`u`, `NM80211ApSecurityFlags`), `MaxBitrate` (`u`, kb/s), `Mode` (`u`). Sinal: `PropertiesChanged` (`Strength`). |
| `org.freedesktop.NetworkManager.Settings` | Props: `Connections` (`ao`). Métodos: `ListConnections() → ao`, `AddConnection(conn:a{sa{sv}}) → o`, `GetConnectionByUuid(s) → o`. |
| `org.freedesktop.NetworkManager.Settings.Connection` | Métodos: `GetSettings() → a{sa{sv}}`, `Delete()`, `Update(a{sa{sv}})`, `GetSecrets(name:s) → a{sa{sv}}`. Usado para casar SSID↔perfil salvo (`is_saved`) e para **esquecer** (`Delete`). |
| `org.freedesktop.NetworkManager.Connection.Active` | Props: `Connection` (`o`, → `Settings.Connection`), `Id` (`s`), `Uuid` (`s`), `Type` (`s`), `State` (`u`, `NMActiveConnectionState`), `Devices` (`ao`), `Ip4Config` (`o`). Sinal: `StateChanged(state:u, reason:u)`. |
| `org.freedesktop.NetworkManager.IP4Config` | Props: `AddressData` (`aa{sv}`: `{address:s, prefix:u}`), `Gateway` (`s`), `NameserverData` (`aa{sv}`: `{address:s}`), `Domains` (`as`). |

### 2.2 Exemplos reais (host de validação, NM 1.58.1)

```
Device        /org/freedesktop/NetworkManager/Devices/62   Interface=wlan0  DeviceType=2
 .Wireless    ActiveAccessPoint=/…/AccessPoint/16746  Bitrate=866700 (≈866.7 Mbps)  LastScan=665274323
 .Statistics  RxBytes=16595163720  TxBytes=3666492209  RefreshRateMs=0

AccessPoint   /org/freedesktop/NetworkManager/AccessPoint/16746
 Ssid=ay["ROSANE_VIACONNECT"]  Strength=86  Frequency=5745 (→ 5 GHz)  HwAddress=48:EF:61:5A:DF:E4
 Flags=3 (PRIVACY|WPS)  WpaFlags=0  RsnFlags=392 (0x188)  MaxBitrate=1170000  Mode=2 (infra)

IP4Config     /org/freedesktop/NetworkManager/IP4Config/50
 AddressData=[{address:"192.168.3.6", prefix:24}]  Gateway="192.168.3.1"  NameserverData=[{address:"192.168.3.1"}]

Connection.Active /org/freedesktop/NetworkManager/ActiveConnection/34
 Id="ROSANE_VIACONNECT"  State=2 (activated)  Uuid="eadc43b8-…"  Connection=/…/Settings/15
```

### 2.3 Enums do NetworkManager (constantes a fixar em Rust)

```rust
// NMDeviceType (Device.DeviceType) — só WIFI nos interessa na v1.
const NM_DEVICE_TYPE_ETHERNET: u32 = 1;
const NM_DEVICE_TYPE_WIFI: u32     = 2;

// NMDeviceState (Device.State / StateChanged.new)
const NM_DEVICE_STATE_UNKNOWN: u32       = 0;
const NM_DEVICE_STATE_UNMANAGED: u32     = 10;
const NM_DEVICE_STATE_UNAVAILABLE: u32   = 20;  // rádio off / sem portadora
const NM_DEVICE_STATE_DISCONNECTED: u32  = 30;
const NM_DEVICE_STATE_PREPARE: u32       = 40;
const NM_DEVICE_STATE_CONFIG: u32        = 50;
const NM_DEVICE_STATE_NEED_AUTH: u32     = 60;  // precisa de segredo
const NM_DEVICE_STATE_IP_CONFIG: u32     = 70;
const NM_DEVICE_STATE_IP_CHECK: u32      = 80;
const NM_DEVICE_STATE_SECONDARIES: u32   = 90;
const NM_DEVICE_STATE_ACTIVATED: u32     = 100;
const NM_DEVICE_STATE_DEACTIVATING: u32  = 110;
const NM_DEVICE_STATE_FAILED: u32        = 120;

// NMDeviceStateReason (StateChanged.reason) — gatilhos de auth/erro.
const NM_DEVICE_STATE_REASON_NO_SECRETS: u32          = 7;   // ← senha errada/ausente
const NM_DEVICE_STATE_REASON_SUPPLICANT_DISCONNECT: u32 = 8;
const NM_DEVICE_STATE_REASON_SSID_NOT_FOUND: u32      = 53;

// NMActiveConnectionState (Connection.Active.State)
const NM_ACTIVE_CONNECTION_STATE_UNKNOWN: u32     = 0;
const NM_ACTIVE_CONNECTION_STATE_ACTIVATING: u32  = 1;
const NM_ACTIVE_CONNECTION_STATE_ACTIVATED: u32   = 2;
const NM_ACTIVE_CONNECTION_STATE_DEACTIVATING: u32 = 3;
const NM_ACTIVE_CONNECTION_STATE_DEACTIVATED: u32 = 4;

// NM80211ApFlags (AccessPoint.Flags)
const NM_802_11_AP_FLAGS_PRIVACY: u32 = 0x1;  // rede protegida (WEP+ ou WPA+)
const NM_802_11_AP_FLAGS_WPS: u32     = 0x2;

// NM80211ApSecurityFlags (WpaFlags / RsnFlags) — bitmask.
const NM_SEC_PAIR_CCMP: u32     = 0x008;
const NM_SEC_GROUP_CCMP: u32    = 0x080;
const NM_SEC_KEY_MGMT_PSK: u32  = 0x100;  // WPA/WPA2 pessoal
const NM_SEC_KEY_MGMT_8021X: u32 = 0x200; // Enterprise (fora do escopo v1)
const NM_SEC_KEY_MGMT_SAE: u32  = 0x400;  // WPA3 pessoal
const NM_SEC_KEY_MGMT_OWE: u32  = 0x800;  // Opportunistic Wireless Encryption
```

> **Validação da derivação de segurança:** o AP de exemplo tem `RsnFlags=392`
> (`0x188` = `PSK | GROUP_CCMP | PAIR_CCMP`) e `WpaFlags=0` → **WPA2-PSK**.
> A regra da §3.3 produz exatamente isso.

---

## 3. Detecção, Scan e Modelo de Dados

### 3.1 Descoberta do dispositivo Wi-Fi

Na inicialização de `network::run`:

1. `Connection::system().await` → conexão zbus (mesmo padrão de `storage::run`).
2. `GetDevices()` → `Vec<OwnedObjectPath>`; para cada um, ler
   `Device.DeviceType`; selecionar o **primeiro** com `DeviceType == 2` (WIFI).
   *(v1 assume um adaptador Wi-Fi; multi-adaptador é roadmap.)*
3. Ler o estado global do rádio na raiz: `WirelessEnabled` (soft),
   `WirelessHardwareEnabled` (rfkill físico), `NetworkingEnabled`.
4. Se não houver dispositivo WIFI → snapshot com `wifi_device: None`
   (a UI mostra "nenhum adaptador Wi-Fi"; telemetria de enlace cabeado pode ser
   exibida como cortesia, mas não é objetivo do Módulo 2).

### 3.2 Enumeração de Access Points

`Device.Wireless.GetAllAccessPoints() → ao`. Para cada AP, ler as props via
`org.freedesktop.DBus.Properties.GetAll` (uma chamada por AP) ou `Get` por
propriedade. Preferir `GetAll` para reduzir *round-trips*.

- `Ssid` (`ay`) → decodificar por `String::from_utf8_lossy` (SSID pode conter
  bytes não-UTF8; guardar também os bytes crus para reuso em
  `AddAndActivateConnection`). SSID vazio → rede **oculta** (§8).
- `Strength` (`y`, 0..100) → barras de sinal (§6.3).
- `Frequency` (`u`, MHz) → banda derivada (§3.4).
- `HwAddress` (`s`) → BSSID.
- `Flags`/`WpaFlags`/`RsnFlags` → segurança (§3.3).
- `is_active` = (`AP.path == Device.Wireless.ActiveAccessPoint`).
- `is_saved` = existe perfil salvo cujo SSID casa (§3.5).

**Deduplicação/ordenação:** vários APs podem anunciar o mesmo SSID (roaming,
2.4+5 GHz). A UI agrega por SSID mostrando o **melhor sinal**; a conexão usa o
`specific_object` do melhor AP daquele SSID. Ordenar: AP ativo primeiro, depois
por `Strength` desc, depois SSID alfabético.

### 3.3 Derivação de segurança (função pura, testável)

```rust
pub enum Security { Open, Wep, Wpa, Wpa2, Wpa3, Wpa2Enterprise, Owe }

pub fn derive_security(flags: u32, wpa: u32, rsn: u32) -> Security {
    let has = |bits: u32, m: u32| bits & m != 0;
    if has(rsn, NM_SEC_KEY_MGMT_SAE)            { return Security::Wpa3; }
    if has(rsn, NM_SEC_KEY_MGMT_OWE)            { return Security::Owe; }
    if has(rsn, NM_SEC_KEY_MGMT_8021X)
        || has(wpa, NM_SEC_KEY_MGMT_8021X)      { return Security::Wpa2Enterprise; }
    if has(rsn, NM_SEC_KEY_MGMT_PSK)            { return Security::Wpa2; } // RSN=WPA2/WPA3-PSK
    if wpa != 0                                  { return Security::Wpa; }  // só WPA1
    if has(flags, NM_802_11_AP_FLAGS_PRIVACY)   { return Security::Wep; }  // PRIVACY sem RSN/WPA
    Security::Open
}

impl Security {
    /// `true` quando conectar exige uma PSK do usuário (modal de senha).
    pub fn needs_psk(self) -> bool {
        matches!(self, Security::Wep | Security::Wpa | Security::Wpa2 | Security::Wpa3)
    }
    /// `key-mgmt` do `802-11-wireless-security` do NM.
    pub fn key_mgmt(self) -> Option<&'static str> {
        match self {
            Security::Wpa | Security::Wpa2 => Some("wpa-psk"),
            Security::Wpa3 => Some("sae"),
            Security::Wep  => Some("none"),   // WEP usa `wep-key0`, não `psk`
            Security::Owe  => Some("owe"),
            _ => None,
        }
    }
}
```

### 3.4 Banda por frequência (função pura)

```rust
pub enum WifiBand { Ghz24, Ghz5, Ghz6, Unknown }
pub fn band_of(freq_mhz: u32) -> WifiBand {
    match freq_mhz {
        2400..=2500 => WifiBand::Ghz24,
        4900..=5900 => WifiBand::Ghz5,   // ex.: 5745 → 5 GHz
        5925..=7125 => WifiBand::Ghz6,
        _ => WifiBand::Unknown,
    }
}
```

### 3.5 Perfis salvos (`is_saved`) e SSID↔conexão

`Settings.ListConnections() → ao`; para cada, `GetSettings() → a{sa{sv}}` e
ler `802-11-wireless.ssid` (`ay`) ou `connection.id`. Casar com o SSID do AP
(bytes crus preferencialmente). O caminho da `Settings.Connection` casada é o
que **esquecer** (`Delete`) usa. Cachear esse mapeamento no snapshot para a UI
marcar redes conhecidas e permitir reconexão sem digitar senha.

### 3.6 Modelo de dados (novo `backend/network.rs`)

```rust
pub struct NetworkSnapshot {
    pub nm_available: bool,
    pub networking_enabled: bool,
    pub wireless_enabled: bool,       // WirelessEnabled (soft)
    pub wireless_hw_enabled: bool,    // WirelessHardwareEnabled (rfkill)
    pub wifi_device: Option<WifiDevice>,
    pub access_points: Vec<AccessPoint>,   // já ordenados (ativo→sinal→SSID)
    pub active: Option<ActiveConnectionInfo>,
    pub telemetry: NetTelemetry,
}

pub struct WifiDevice {
    pub id: DeviceId,                 // object path do Device (identidade estável)
    pub iface: String,                // "wlan0"
    pub hw_address: String,           // MAC
    pub state: u32,                   // NMDeviceState cru (mapeado na UI)
    pub bitrate_kbps: u32,            // Device.Wireless.Bitrate
    pub active_ap: Option<DeviceId>,  // ActiveAccessPoint
    pub last_scan_ms: i64,            // LastScan (-1 = nunca)
}

pub struct AccessPoint {
    pub id: DeviceId,                 // object path do AP
    pub ssid: String,                 // decodificado (lossy) p/ exibição
    pub ssid_raw: Vec<u8>,            // bytes originais p/ AddAndActivate
    pub bssid: String,
    pub strength: u8,                 // 0..100
    pub frequency: u32,               // MHz
    pub band: WifiBand,
    pub max_bitrate_kbps: u32,
    pub security: Security,
    pub is_active: bool,
    pub is_saved: bool,
    pub saved_conn_path: Option<String>, // Settings.Connection (p/ forget)
}

pub struct ActiveConnectionInfo {
    pub id: DeviceId,                 // Connection.Active object path
    pub ssid: String,
    pub state: u32,                   // NMActiveConnectionState
    pub connection_path: Option<String>, // Settings.Connection
}

pub struct NetTelemetry {
    pub ip4: Option<String>,          // "192.168.3.6/24"
    pub gateway: Option<String>,      // "192.168.3.1"
    pub dns: Vec<String>,             // ["192.168.3.1"]
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub rx_rate_bps: f64,             // taxa instantânea (janela ~1s)
    pub tx_rate_bps: f64,
}
```

`DeviceId(pub String)` já existe em `events/mod.rs` (é o caminho do objeto
D-Bus) e é **reusado** aqui como identidade estável de devices/APs/conexões —
nomes de AP (`…/AccessPoint/N`) mudam entre replug/rescan, então a UI referencia
sempre por esse path e o backend revalida antes de agir (TOCTOU, §9).

---

## 4. Telemetria de Rede

### 4.1 IP, gateway e DNS

Da `Device.Ip4Config` (object path) → interface `IP4Config`:

- `AddressData` (`aa{sv}`) → primeiro item: `{address:s, prefix:u}` →
  `"192.168.3.6/24"`.
- `Gateway` (`s`) → `"192.168.3.1"` (pode ser vazio quando sem rota default).
- `NameserverData` (`aa{sv}`) → coletar cada `{address:s}`.

IPv6 (`Ip6Config`) é **roadmap**; a v1 exibe apenas IPv4 (o campo IPv6 pode ser
adicionado ao `NetTelemetry` depois sem quebrar contratos).

### 4.2 Taxa de bits do enlace

`Device.Wireless.Bitrate` (`u`, kb/s) → exibir em Mbps
(`866700 → "866.7 Mbps"`). É a taxa negociada do enlace, não o throughput real.

### 4.3 Throughput RX/TX em tempo real (duas estratégias)

1. **Preferencial (Rust puro, sem D-Bus):** ler
   `/sys/class/net/<iface>/statistics/rx_bytes` e `tx_bytes` (contadores
   monotônicos do kernel), computar delta entre dois *ticks* de telemetria.
   Barato, sempre disponível, zero dependência. Um parser puro trivial
   (`u64::from_str` do conteúdo trimado).
2. **Alternativa pura-D-Bus:** habilitar `Device.Statistics.RefreshRateMs`
   (escrever `1000`) e ler `RxBytes`/`TxBytes` (`t`). Útil se `/sys` estiver
   indisponível (contêiner restrito). Custa uma escrita de propriedade e mantém
   o NM atualizando o contador.

**Cálculo da taxa** (função pura, reusa a lógica de `compute_speed_eta` do
storage adaptada):

```rust
pub fn rate_bps(prev: u64, curr: u64, secs: f64) -> f64 {
    if secs <= 0.0 { return 0.0; }
    curr.saturating_sub(prev) as f64 / secs
}
```

Exibir com `human_bytes(rate as u64) + "/s"` (helper já existente em
`ui/widgets.rs`), ex.: `"1.2 MiB/s ↓ / 84 KiB/s ↑"`.

### 4.4 Cadência dupla de atualização

- `interval(config.polling.network_ms)` (default **5000 ms**, já em
  `config.toml`) → **rebuild completo** do snapshot (rescan de props de APs,
  perfis salvos, segurança). Custa vários round-trips D-Bus.
- `interval(1000 ms)` → **telemetria leve** apenas: deltas RX/TX + `Bitrate` +
  estado do device, para uma taxa fluida sem reconstruir a lista inteira de APs.

Os dois emitem `AppEvent::Network(Box<NetworkSnapshot>)`; o snapshot leve
reaproveita a última lista de APs conhecida (mantida no backend) atualizando só
`telemetry` + `wifi_device`.

---

## 5. Conexão, Autenticação e Segurança da Senha

### 5.1 Máquinas de estado

#### 5.1.1 Scan

```
RadioOff ──(t: liga)──▶ Idle
Idle ──(r / usuário)──▶ Scanning ──(LastScan avança / AccessPointAdded)──▶ Idle
Idle ──(t: desliga / rfkill)──▶ RadioOff
```

`RequestScan({})` dispara; o fim do scan é observado por `LastScan` mudando e por
sinais `AccessPointAdded/Removed`. Nunca bloquear esperando resultado — a UI
mostra "escaneando…" e atualiza quando os eventos chegarem.

#### 5.1.2 Conexão (espelha `NMDeviceState`)

```
Disconnected
  │ (Enter sobre AP)
  ▼
Preparing (40) ─▶ Config (50) ─▶ [NeedAuth (60)] ─▶ IpConfig (70) ─▶ IpCheck (80) ─▶ Activated (100)
                                     │                                                    │
                                     │ (reason NO_SECRETS=7)                               ▼
                                     ▼                                                 Connected
                              AuthNeeded ──(§5.1.3)                                        │
                                                                                          │ (d: desconectar)
Failed (120) ◀── (reason != NO_SECRETS: SSID_NOT_FOUND, SUPPLICANT_DISCONNECT…)           ▼
                                                                                    Deactivating (110) ─▶ Disconnected
```

Fonte de verdade: `Device.StateChanged(new, old, reason)` +
`Connection.Active.State`. Cada transição relevante emite um `Toast`
(conectando / conectado a `<SSID>` / falha: `<motivo legível>`).

#### 5.1.3 Autenticação (WPA2/WPA3-PSK)

```
                (AP protegido, precisa de PSK)
Connect intent ─────────────────────────────────▶ AwaitingSecret
                                                        │  NetworkSecretRequest → modal nativo
                                                        ▼
                             ┌────────────── usuário digita PSK (Enter) ──────────────┐
                             ▼                                                          │
                    AddAndActivateConnection(conn+psk, dev, ap)                        │ (Esc: cancela)
                             │                                                          ▼
              ┌──────────────┴───────────────┐                                    Disconnected
              ▼                              ▼
        Activated (100)              Failed + NO_SECRETS(7)
              │                              │  retry_error = "Senha incorreta"
              ▼                              └──────────▶ AwaitingSecret (loop)
          Connected
```

Espelha **exatamente** o fluxo de senha de sudo já implementado no Módulo 4
(`SudoPasswordRequest` + modal mascarado + laço de nova tentativa em caso de
falha). Reaproveitar o mesmo idioma de UX (§6.4).

#### 5.1.4 Desconexão

```
Connected ──(d)──▶ DeactivateConnection(active)  ▼  ou  Device.Disconnect()
                                          Deactivating (110) ─▶ Disconnected
```

### 5.2 Como conectar (decisão de projeto)

Ao receber `Action::NetworkConnect(ap_id)`, o backend resolve o AP no último
snapshot e decide:

| Situação | Ação D-Bus |
|----------|-----------|
| Perfil salvo existe p/ o SSID | `ActivateConnection(conn, dev, ap)` — sem pedir senha. |
| Rede **aberta**, sem perfil | `AddAndActivateConnection({conn+wireless}, dev, ap)` — sem segurança. |
| Rede **protegida**, sem perfil (ou perfil falhou com NO_SECRETS) | pede PSK pelo **canal dedicado** (§5.3) → `AddAndActivateConnection({conn+wireless+security(psk)}, dev, ap)`. |

Dicionário de settings para WPA2/WPA3-PSK (`a{sa{sv}}`):

```
connection             : { "id": <ssid>, "type": "802-11-wireless" }
802-11-wireless        : { "ssid": <ay bytes>, "mode": "infrastructure" }
802-11-wireless-security: { "key-mgmt": "wpa-psk" | "sae", "psk": <PSK> }
```

O NM **persiste** o perfil por padrão (vira `is_saved` no próximo snapshot).
WPA3 usa `key-mgmt = "sae"`; WPA2 usa `"wpa-psk"` (a mesma PSK). `AddAndActivateConnection2`
existe no host (aceita `options a{sv}`), mas a v1 usa `AddAndActivateConnection`
(mais amplamente disponível).

### 5.3 Segurança da senha Wi-Fi (canal dedicado + `Secret`)

> `Action` é `broadcast` (Clone + Debug) e chega a **todos** os backends. Uma
> PSK **jamais** deve trafegar por esse canal (vazaria para bluetooth/power/pty
> em processo, e apareceria em `Debug`/log).

**Mecanismo (gêmeo do `SudoPasswordRequest` já existente):**

```rust
/// Canal dedicado, fora do broadcast — só o backend de rede o usa.
pub struct NetworkSecretRequest {
    pub ssid: String,                 // exibido no modal
    pub retry_error: Option<String>,  // "Senha incorreta" numa nova tentativa
    pub respond: tokio::sync::oneshot::Sender<Option<Secret>>, // None = Esc
}
pub type NetworkSecretTx = tokio::sync::mpsc::UnboundedSender<NetworkSecretRequest>;
```

- O backend, ao precisar da PSK, envia `NetworkSecretRequest`; `lib::run`
  repassa ao `App`, que abre o modal mascarado (§6.4) e responde pelo `oneshot`.
- O `oneshot` **não é Clone/Debug** — exatamente o motivo pelo qual sudo já usa
  um canal separado do `AppEvent` (ver o comentário em `events/mod.rs`).
- **`Secret`** — wrapper com `Debug` redigido e *zeroize* no `Drop`:

```rust
pub struct Secret(String);
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}
impl Drop for Secret {
    fn drop(&mut self) {
        // zeroize manual (ou a crate `zeroize`, opcional — ver §7.1)
        unsafe { for b in self.0.as_bytes_mut() { std::ptr::write_volatile(b, 0); } }
    }
}
```

A PSK vive só entre o modal e a chamada `AddAndActivateConnection`; nunca é
logada, nunca entra no snapshot, nunca cruza o `broadcast<Action>`.

> **Oportunidade de refatoração (recomendada):** `SudoPromptState`/`SudoPasswordRequest`
> e `NetworkSecretRequest`/modal de senha Wi-Fi são o mesmo padrão ("pedir um
> segredo mascarado, responder por oneshot"). Vale extrair um componente
> `SecretPrompt` genérico compartilhado pelas duas features. A v1 pode duplicar
> e refatorar depois; o contrato acima já deixa a porta aberta.

### 5.4 Toggle de rádio e Polkit

- **Ligar/desligar Wi-Fi (`t`):** escrever a propriedade `WirelessEnabled` (`b`)
  na raiz via `org.freedesktop.DBus.Properties.Set`, ou `Enable(b)` para
  networking global. **Respeitar `WirelessHardwareEnabled`**: se `false`
  (rfkill físico/hardware), a habilitação por software falha — mostrar toast
  "rádio desligado por chave de hardware", sem tentar.
- **Polkit:** conectar/desconectar/toggle da sessão ativa tipicamente não pede
  senha. Se o NM retornar `NotAuthorized` (sessão headless sem agente Polkit),
  degradar com toast explicativo — **nunca** travar. (Reusar
  `is_not_authorized_error` do storage como referência de detecção.)

---

## 6. Interface TUI (Ratatui — Aba 2)

### 6.1 Layout responsivo em 2 colunas

```
┌─ Wi-Fi & Rede ───────────────────────────────────────────────────────────────┐
│ ┌── Redes (≈45%) ───────────────┐ ┌── Detalhes & Telemetria (≈55%) ─────────┐ │
│ │ * ROSANE_VIACONNECT ▆▆▆▆ WPA2 │ │  SSID:      ROSANE_VIACONNECT   (ativa)  │ │
│ │   VIZINHO_5G        ▆▆▆░ WPA3 │ │  BSSID:     48:EF:61:5A:DF:E4            │ │
│ │   CAFE_LIVRE        ▆▆░░ aberta│ │  Banda:     5 GHz (5745 MHz)  Sinal 86% │ │
│ │ ⌂ MINHA_SALVA       ▆░░░ WPA2 ✔│ │  Segurança: WPA2-PSK    Enlace: 866 Mbps│ │
│ │   <oculta>          ▆▆░░ WPA2 │ │ ─────────────────────────────────────────│ │
│ │                              │ │  IP:      192.168.3.6/24                  │ │
│ │                              │ │  Gateway: 192.168.3.1                     │ │
│ │                              │ │  DNS:     192.168.3.1                     │ │
│ │                              │ │  ↓ 1.2 MiB/s   ↑ 84 KiB/s                 │ │
│ │                              │ │  [▁▂▄▆█▆▄▂] throughput (sparkline)        │ │
│ │                              │ │ ─────────────────────────────────────────│ │
│ │                              │ │  Ações: [Enter] conectar [d] desconectar  │ │
│ │                              │ │         [f] esquecer [r] rescan [t] rádio │ │
│ └──────────────────────────────┘ └───────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────────────────────────┤
│ [Enter] conectar [d] desconect. [f] esquecer [r] rescan [t] rádio  ● toast     │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Responsividade:** `Layout::horizontal([Percentage(45), Percentage(55)])`.
  Abaixo de ~80 colunas, colapsar para **coluna única** (lista sobre detalhes,
  alternável por `Tab`/`→`), exatamente como o Módulo 4 prevê.
- Reusar `content_block` (título da aba) e `centered` (modais) de `ui/mod.rs`,
  e `human_bytes`/`truncate_str`/`kv_line`/`metric_line` de `ui/widgets.rs`.

### 6.2 Ícones e marcadores (respeitando `config.ui.icons`)

| Conceito | Nerd Font | Fallback ASCII |
|----------|-----------|----------------|
| Rede ativa | prefixo de conexão | `*` |
| Perfil salvo | marca de salvo | `✔` / `[S]` |
| Cadeado (protegida) | `\u{f023}` (lock) | `[LOCKED]` / rótulo de segurança textual |
| Sinal | glifos de sinal | barras `▂▄▆█` por faixa de `Strength` |
| Segurança | — | rótulo textual `WPA2`/`WPA3`/`aberta`/`WEP` |

Barras de sinal por faixa (função pura): `0-25→▂`, `26-50→▄`, `51-75→▆`,
`76-100→█`, com quatro células preenchidas proporcionalmente. Reaproveitar o
gradiente verde→amarelo→vermelho já usado nas barras de uso do storage.

### 6.3 Atalhos de teclado (statusline da aba)

| Tecla | Ação | Contexto |
|-------|------|----------|
| `j`/`k` `↑`/`↓` | navegar lista de redes | sempre |
| `Enter` | conectar à rede selecionada (abre modal de senha se necessário) | AP selecionado |
| `d` | desconectar da rede ativa | conectado |
| `f` | esquecer perfil salvo (com confirmação) | AP com `is_saved` |
| `r` | rescan (RequestScan) | rádio ligado |
| `t` | ligar/desligar rádio Wi-Fi | sempre (respeita rfkill) |
| `Esc` | cancelar modal (senha / confirmação de esquecer) | modais |

> **Integração com o keymap** (`events/input.rs`): hoje `r`→Refresh, `m`→mudo,
> `d`/`f`/`t` caem em `Action::Raw`. Espelhar o bloco `if active == Tab::Storage`
> com um bloco `if active == Tab::Network` que sobrepõe `Enter/d/f/r/t` para as
> intenções de rede e desvia o teclado para o modal de senha quando aberto
> (mesmo mecanismo de `text_mode`/`storage_modal_open`, generalizado). O modal
> nativo de senha (§6.4) já tem prioridade máxima via o caminho do
> `sudo_prompt_open` generalizado para `secret_prompt_open`.

### 6.4 Modal de senha Wi-Fi (WPA2/WPA3-PSK)

Espelha o `draw_sudo_prompt` já existente (`ui/storage.rs`): retângulo
`centered(56, 30)`, `Clear`, `modal_block`, senha **mascarada** (`"*".repeat(n)`
+ cursor `▏`), linha de erro em vermelho na nova tentativa, rodapé
`[Enter] Conectar   [Esc] Cancelar`.

```
┌─ Conectar a "VIZINHO_5G" ───────────────────────┐
│ Rede:  VIZINHO_5G   (WPA3-SAE, 5 GHz)            │
│                                                  │
│ Senha: ************▏                             │
│                                                  │
│ Senha incorreta — tente novamente               │  ← só na 2ª tentativa (NO_SECRETS)
│                                                  │
│ [Enter] Conectar    [Esc] Cancelar              │
└──────────────────────────────────────────────────┘
```

Estado do modal em `App` (gêmeo de `SudoPromptState`):

```rust
pub struct NetworkPasswordState {
    pub ssid: String,
    pub security: Security,
    pub password: String,          // exibido mascarado
    pub error: Option<String>,     // "Senha incorreta" na nova tentativa
}
// e o oneshot de resposta guardado no App, como `sudo_respond`.
```

Roteamento idêntico ao sudo: prioridade máxima no `dispatch`, cada caractere vira
digitação mascarada, `Enter` responde `Some(Secret(password))`, `Esc` responde
`None` (cancela a conexão).

### 6.5 Modal de confirmação "esquecer rede"

Confirmação simples (`Enter`/`y` confirma, `Esc`/`n` cancela) antes de
`Settings.Connection.Delete()` — esquecer é reversível apenas reconfigurando a
senha, então merece um passo de confirmação, mas **não** a trava de duas etapas
do flasher (não é destrutivo de dados).

### 6.6 Estados degradados na UI

| Condição | Tela |
|----------|------|
| NM ausente/off-bus | Painel `● NetworkManager indisponível` (reusa `draw_pending` enquanto `services["network"].degraded`). |
| `wifi_device == None` | "Nenhum adaptador Wi-Fi detectado" + telemetria cabeada opcional. |
| `wireless_hw_enabled == false` | "Rádio Wi-Fi desligado por chave de hardware (rfkill)". |
| `wireless_enabled == false` | "Rádio Wi-Fi desligado — pressione `t` para ligar". |
| Escaneando, lista vazia | "Escaneando redes…" com spinner. |

---

## 7. Contratos de Mensagem (extensões de `events/mod.rs`)

### 7.1 Novos `AppEvent` (backend → app)

```rust
// Snapshot completo do estado de rede (boxed, como System/Storage).
Network(Box<NetworkSnapshot>),
```

Progresso de conexão/auth **não** vira variante própria: reflete-se no
`snapshot.active.state` + `wifi_device.state`, e as transições relevantes emitem
`AppEvent::Toast(..)`. `ServiceDegraded { name: "network", .. }` continua sendo
usado para "NM ausente". A solicitação de senha usa o **canal dedicado**
`NetworkSecretTx` (§5.3), não `AppEvent` — mesmo motivo do sudo (`oneshot` não é
Clone/Debug).

### 7.2 Novos `Action` (input/app → backend)

```rust
// Intenções de UI (o App resolve a seleção → id concreto):
NetworkConnectSelected,        // Enter sobre a rede selecionada
NetworkForgetSelected,         // f  → abre confirmação
NetworkDisconnect,             // d
NetworkRescan,                 // r
NetworkToggleRadio,            // t

// Já resolvidas (com DeviceId concreto) — repassadas direto ao backend:
NetworkConnect(DeviceId),      // AP object path
NetworkForget(DeviceId),       // Settings.Connection object path

// Roteamento do modal de senha (mascarado) — reusa o mecanismo de segredo:
// (recomendado: generalizar os StorageModalChar/Backspace já existentes para um
//  "SecretModalChar", compartilhado por sudo e Wi-Fi; a v1 pode reusar
//  StorageModalChar/Backspace/Enter/ToggleConfig como o sudo já faz.)
```

> **Nenhuma PSK aparece em `Action`.** A senha só existe em `Secret`, trafegando
> pelo `oneshot` do `NetworkSecretRequest`.

### 7.3 Extensão de `App` (state em `app.rs`)

```rust
// dentro de struct App:
pub network: Option<NetworkSnapshot>,
pub network_selected: usize,             // índice na lista de redes
pub network_password: Option<NetworkPasswordState>, // modal de senha (secret prompt)
network_secret_respond: Option<tokio::sync::oneshot::Sender<Option<Secret>>>,
pub network_confirm_forget: Option<String>, // SSID pendente de confirmação
```

`handle_event` ganha o braço `AppEvent::Network(snap) => self.network = Some(*snap)`.
`dispatch` ganha, **antes** do roteamento global (espelhando os blocos de
`sudo_prompt_open`/`storage_modal_open`), a captura do modal de senha de rede e
da confirmação de esquecer. `lib::run` passa a consumir também o canal
`NetworkSecretTx` e a chamar `App::open_network_password(req)`.

### 7.4 Ponto de plugagem do backend (`backend/mod.rs`)

`network::run` deixa de ser stub e passa a receber, além de `tx` e
`actions.subscribe()`, o `NetworkSecretTx` — exatamente como `storage::run`
recebe o `SudoPasswordTx`. Ajustar `spawn_all` para criar esse canal e repassá-lo
(o segundo consumidor do padrão de segredo).

---

## 8. Limites de Escopo v1 (documentados)

| Fora do escopo v1 | Motivo / plano |
|-------------------|----------------|
| **802.1X / WPA2-Enterprise** | Exige usuário+certificado/EAP; UI mostra a rede como `[Enterprise]` e o `Enter` emite toast "use nmcli/GUI (não suportado na v1)". Roadmap: modal estendido. |
| **Redes ocultas** | AP com SSID vazio aparece como `<oculta>`; conectar exigiria digitar o SSID. Roadmap: campo de SSID no modal. |
| **IPv6** | Só IPv4 na telemetria v1; `Ip6Config` é adição não-disruptiva depois. |
| **Múltiplos adaptadores Wi-Fi** | v1 usa o primeiro device WIFI; seletor de adaptador é roadmap. |
| **Hotspot / AP mode, WPS, VPN** | Fora do módulo. |

---

## 9. Dependências e Ferramentas

### 9.1 Crates (Cargo.toml)

| Crate | Papel | Status |
|-------|-------|--------|
| `zbus` | Toda a fala com o NetworkManager | **já presente**. |
| `tokio` | tasks, `select!`, intervals, canais | **já presente**. |
| `zeroize` (opcional) | Zerar a PSK na memória | **nova, opcional** — o `Secret` pode zerar manualmente (`write_volatile`) sem crate. |

> **Nenhuma dependência C, `libnm`, `glib`, nem wrapper de CLI é adicionada.**
> A meta de binário 100% autocontido e Rust puro é preservada. `sha2`, `fatfs`,
> etc. do Módulo 4 não são tocados.

### 9.2 Runtime do host

- **NetworkManager** (`NetworkManager.service`) — obrigatório para o modo pleno.
- **Agente Polkit** ativo (sessão de desktop) para ações autenticadas; ausência
  → degrada com toast, nunca trava.
- `bin/setup.sh` deve passar a diagnosticar: NM no bus (`busctl --system list`),
  estado do rfkill (`WirelessHardwareEnabled`), presença de adaptador WIFI.

---

## 10. Estratégia de Testes

| Nível | Alvo |
|-------|------|
| **Unit (parsers puros)** | `derive_security` (todas as combinações de `flags/wpa/rsn`, incl. `RsnFlags=392`→WPA2, `SAE`→WPA3, `OWE`, `PRIVACY`-only→WEP, `0`→Open); `band_of`; decodificação de SSID `ay` (com bytes não-UTF8); `rate_bps`; parsing de `/sys/.../rx_bytes`; formatação de IP `AddressData→"a/p"`. |
| **Unit (máquinas de estado)** | Transições de conexão (`NMDeviceState`), do fluxo de auth (`NO_SECRETS`→retry, `Esc`→cancela), e do scan; ordenação/dedup da lista de APs por SSID. |
| **Unit (segurança)** | `Secret` — `Debug` **nunca** revela a PSK (`format!("{:?}", secret) == "Secret(***)"`); a PSK não aparece em nenhum `AppEvent`/`Action` (teste de tipo/compilação + revisão). |
| **Integração** | Backend com **mock de D-Bus** (trait `NmClient` injetável) publicando `AppEvent`s determinísticos: device add/remove, AP add/remove, `StateChanged` (activated / failed+NO_SECRETS), toggle de rádio. |
| **Integração** | Fluxo de senha: `NetworkSecretRequest` → resposta `Some(Secret)` → `AddAndActivateConnection` recebe a PSK; `None` (Esc) → nenhuma chamada de ativação. |
| **Smoke** | `cargo run` headless (`TERM=dumb`) com NM ausente → aba degrada, app não cai. |
| **Manual/E2E** | Conectar a uma rede **WPA2** real e a uma **WPA3** real; refletir IP/gateway/DNS e throughput; desconectar; esquecer; toggle de rádio; senha errada reabre modal. |

**Invariante de teste inegociável:** existe um teste que falha o build se a PSK
puder ser observada via `Debug`/serialização de qualquer `Action`/`AppEvent`
(a senha só existe dentro de `Secret`, e `Secret: !Serialize` + `Debug` redigido).

---

## 11. Plano de Implementação Modular & Decomposição em Tasks (Kanban)

Ordem sugerida; cada task é atômica, compilável e testável. Encaixa no Módulo 2
do `docs/03_plano_de_execucao_modular.md`.

### Épico A — Fundação D-Bus & Modelos (read-only, sem risco)

- **A1.** Definir modelos `NetworkSnapshot`/`WifiDevice`/`AccessPoint`/`Security`/
  `WifiBand`/`ActiveConnectionInfo`/`NetTelemetry` em `backend/network.rs`.
  *(unit: `derive_security`, `band_of`, decode SSID)*
- **A2.** Conexão `zbus` ao system bus + `GetDevices` → achar device WIFI
  (`DeviceType==2`) + ler estado de rádio da raiz. *(integração: mock)*
- **A3.** Adicionar `AppEvent::Network(Box<..>)` + braço em `App.handle_event`;
  emitir snapshot por `interval(network_ms)`.
- **A4.** `ServiceDegraded` quando NM ausente/off-bus (substitui o
  `pending_stub`); reconexão preguiçosa (padrão `conn = None` do storage).

### Épico B — Scan & Lista de APs (read-only)

- **B1.** `GetAllAccessPoints` + leitura de props (`GetAll`) → `Vec<AccessPoint>`;
  `is_active` via `ActiveAccessPoint`. *(unit: ordenação/dedup por SSID)*
- **B2.** Perfis salvos: `Settings.ListConnections` + `GetSettings` → casar SSID →
  `is_saved`/`saved_conn_path`. *(integração: mock Settings)*

### Épico C — UI da Aba (read-only)

- **C1.** Substituir `draw_pending` em `ui/network.rs` pelo layout 2 colunas
  (lista + detalhes). *(render puro)*
- **C2.** Lista de redes: barras de sinal, rótulo de segurança, `*` ativa,
  `✔`/cadeado salva; navegação (`j/k`) + `network_selected` em `app.rs`.
- **C3.** Painel de detalhes (SSID/BSSID/banda/sinal/segurança/enlace) +
  statusline de atalhos + i18n de **todas** as strings (pt-BR/en-US/es-ES).
- **C4.** Colapso responsivo para coluna única em telas estreitas + estados
  degradados (§6.6).

### Épico D — Telemetria

- **D1.** `IP4Config` → IP/gateway/DNS no snapshot. *(unit: formatação)*
- **D2.** Throughput RX/TX via `/sys/.../statistics` + `rate_bps` +
  `interval(1000ms)` de telemetria leve; `Bitrate` do enlace. *(unit: parser+taxa)*
- **D3.** Sparkline de throughput recente (reusa helper de widgets).

### Épico E — Monitoramento Reativo (sinais)

- **E1.** Assinar `Device.StateChanged` → dirigir a máquina de conexão + toasts +
  detecção de `NO_SECRETS`. *(integração: mock de sinais)*
- **E2.** Assinar `AccessPointAdded/Removed` + `Device.Wireless.PropertiesChanged`
  (`ActiveAccessPoint`/`Bitrate`/`LastScan`) → atualizar snapshot.
- **E3.** Assinar `PropertiesChanged` da raiz (`WirelessEnabled`/`ActiveConnections`);
  unificar sinais + `Action` + os dois intervals num `tokio::select!`.

### Épico F — Rescan & Toggle de Rádio

- **F1.** `Action::NetworkRescan` (`r`) → `RequestScan({})`; estado "escaneando".
- **F2.** `Action::NetworkToggleRadio` (`t`) → `Set WirelessEnabled`; respeitar
  `WirelessHardwareEnabled` (rfkill) com toast. *(integração)*

### Épico G — Conexão (aberta + perfil salvo)

- **G1.** Keymap `Tab::Network` em `events/input.rs` (`Enter/d/f/r/t`) +
  `NetworkConnectSelected`→`NetworkConnect(id)` no `App`.
- **G2.** Backend: rede aberta → `AddAndActivateConnection` sem segurança;
  perfil salvo → `ActivateConnection`. Toasts de progresso. *(integração)*

### Épico H — Autenticação WPA/WPA3 (canal de segredo + modal) 🔒

- **H1.** `Secret` (Debug redigido + zeroize) + `NetworkSecretRequest`/
  `NetworkSecretTx`; `spawn_all` cria e injeta o canal. *(unit: redação)*
- **H2.** Modal de senha em `App` (`NetworkPasswordState`) + render (espelha
  `draw_sudo_prompt`) + roteamento mascarado (prioridade máxima). *(unit: máquina)*
- **H3.** Backend: `AddAndActivateConnection` com `802-11-wireless-security`
  (`wpa-psk`/`sae`); laço de nova tentativa em `NO_SECRETS`. *(integração)*

### Épico I — Desconectar & Esquecer

- **I1.** `Action::NetworkDisconnect` (`d`) → `DeactivateConnection`/`Disconnect`.
- **I2.** `Action::NetworkForget*` (`f`) → confirmação → `Settings.Connection.Delete`.
  *(integração)*

### Épico J — Fechamento

- **J1.** `bin/setup.sh`: diagnosticar NM/rfkill/adaptador WIFI.
- **J2.** Suíte de testes (parsers, máquinas, redação de `Secret`, invariante) no CI.
- **J3.** Atualizar `docs/02_especificacao_das_abas.md` e
  `docs/03_plano_de_execucao_modular.md` (Módulo 2) refletindo o entregue.
- **J4.** Clippy limpo + smoke headless + revisão de i18n.

### Grafo de dependências

```
A ──▶ B ──▶ C
 │     │
 ├──▶ D (telemetria)
 │
 └──▶ E (sinais) ──▶ F (rescan/rádio)
        │
        └──▶ G (conexão) ──▶ H (auth 🔒) ──▶ I (disconnect/forget)
D,F,G,H,I ──▶ J
```

> **H depende de G** (conexão base) **e** do canal de segredo. Nenhuma operação
> que manipule a PSK entra em "pronto para dev" antes de `Secret` + canal
> dedicado estarem implementados e testados (invariante de não-vazamento).

---

## 12. Riscos & Mitigações

| Risco | Mitigação |
|-------|-----------|
| **PSK vazando** em `Debug`/log/broadcast | `Secret` redigido + zeroize; canal `oneshot` dedicado (nunca no `broadcast<Action>`); invariante de teste (§10). |
| TOCTOU (path `…/AccessPoint/N` muda entre scan e conexão) | Identidade por object path + revalidação do AP/SSID no backend antes de `Activate`. |
| rfkill de hardware (`WirelessHardwareEnabled=false`) | Detectar e informar; não tentar habilitar por software. |
| Polkit ausente (headless) → `NotAuthorized` | Degradar com toast (reusar `is_not_authorized_error`); nunca travar. |
| Versões de NM sem algum método/prop | `GetAllAccessPoints` (preferido) em vez da prop `AccessPoints` (deprecada); checar `Version`; `AddAndActivateConnection` (v1) em vez do `2`. |
| SSID não-UTF8 / oculto | Guardar `ssid_raw` (bytes) p/ ativar; exibir lossy/`<oculta>`. |
| Flood de `PropertiesChanged` (Strength de muitos APs) | Não assinar Strength por AP; recomputar sinal no `interval(network_ms)`. |
| Enterprise/oculta tentadas por engano | UI marca e bloqueia `Enter` com toast (§8). |
| Senha errada em loop | `NO_SECRETS` reabre o modal com `retry_error`; `Esc` sempre cancela e volta a Disconnected. |

---

## 13. Definição de Pronto (Módulo 2)

1. `cargo clippy -- -D warnings` limpo.
2. Aba degrada graciosamente sem NetworkManager, sem adaptador, e com rádio
   off (soft e rfkill).
3. Atalhos aparecem na statusline; **todas** as strings em i18n (pt-BR/en-US/es-ES).
4. Nenhum `.await` de I/O na thread de render.
5. **Segurança da senha testada:** invariante que impede a PSK de ser observada
   via `Debug`/serialização de `Action`/`AppEvent`; `Secret` zeroiza.
6. Conectar a uma rede **WPA2** real e a uma **WPA3** real; refletir IP/gateway/
   DNS e throughput RX/TX em tempo real (aceite do Módulo 2).
7. Desconectar, esquecer (com confirmação) e alternar o rádio funcionam e
   refletem o estado imediatamente.
8. Zero dependências C / `libnm` / wrappers de CLI adicionadas — 100% `zbus`.
