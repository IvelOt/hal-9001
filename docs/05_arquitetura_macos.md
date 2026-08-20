# 05 — Arquitetura de Suporte ao macOS (Darwin) 🍏

Este documento especifica a estratégia de compatibilidade multiplataforma do **HAL-9001** para sistemas **macOS** (Apple Silicon M1/M2/M3/M4 e Mac Intel).

---

## 1. Filosofia de Portabilidade

O HAL-9001 adota uma arquitetura em camadas onde:
- **TUI (Ratatui/Crossterm), Loop de Eventos (Tokio), Configuração (Serde/TOML) e PTY (Portable-PTY)** são **100% universais e agnósticos a sistema operacional**.
- Apenas a camada de **Introspecção de Sistema e Drivers de I/O** (`src/backend/`) utiliza compilação condicional em Rust (`#[cfg(target_os = "linux")]` vs `#[cfg(target_os = "macos")]`).

```
┌──────────────────────────────────────────────────────────┐
│             Interface Ratatui & Widgets (TUI)            │
├──────────────────────────────────────────────────────────┤
│             Barramento de Eventos Tokio (AppEvent)       │
├────────────────────────────┬─────────────────────────────┤
│   Backend Linux (sysfs)    │    Backend macOS (Darwin)   │
│ ├── D-Bus (NetworkManager) │ ├── IOKit (Power/Battery)   │
│ ├── bluez (Bluetooth)      │ ├── CoreAudio / osascript   │
│ ├── UDisks2 (Storage)      │ ├── diskutil (Storage)      │
│ └── /sys/class/backlight   │ └── Homebrew / MacPorts     │
└────────────────────────────┴─────────────────────────────┘
```

---

## 2. Mapeamento de Recursos no macOS

| Recurso | Mecanismo no Linux | Mecanismo no macOS | Estratégia de Fallback |
| :--- | :--- | :--- | :--- |
| **CPU / Memória / Uptime** | `/proc` via `sysinfo` | `host_statistics64` via `sysinfo` | Nativo do crate `sysinfo` |
| **Gerenciador de Pacotes** | `pacman`, `dpkg`, `rpm` | `brew list --formula`, `brew list --cask`, `mas list` | Execução não-bloqueante async via Tokio |
| **Bateria & Consumo** | `/sys/class/power_supply` | `pmset -g batt` / `IOKit.framework` | Parsing regex de `pmset -g batt` |
| **Brilho de Tela** | `/sys/class/backlight` | `brightness` CLI / `DisplayServices` | Degrada para `None` em Mac Mini/Studio |
| **Áudio & Volume** | `wpctl` (PipeWire) / `amixer` | `osascript -e "output volume of..."` | CoreAudio API direta |
| **Rede & Wi-Fi** | NetworkManager via D-Bus | `networksetup` / `airport -s` | Parsing de lista de SSIDs e sinal |
| **Bluetooth** | bluez via D-Bus | `blueutil --paired` / `IOBluetooth` | Interação via CLI `blueutil` |
| **Armazenamento / Discos**| UDisks2 via D-Bus | `diskutil list -plist` / `diskutil mount` | `diskutil` nativo do macOS |
| **Perfis de Energia** | `power-profiles-daemon` | `pmset -a lowpowermode 1/0` | Modo de Pouca Energia nativo Apple |

---

## 3. Estratégia de Compilação e Binários

### Alvos de Compilação (Rustup Targets)
- `aarch64-apple-darwin` — Apple Silicon (M1, M2, M3, M4, Pro, Max, Ultra).
- `x86_64-apple-darwin` — Macs com processadores Intel.

### Criação de Binário Universal Apple (`lipo`)
```bash
# Compilação cruzada ou nativa no runner macOS:
cargo build --release --target aarch64-apple-darwin
cargo build --release --target x86_64-apple-darwin

# Fusão em binário universal executável em qualquer Mac:
lipo -create -output hal9001-mac-universal \
  target/aarch64-apple-darwin/release/hal9001 \
  target/x86_64-apple-darwin/release/hal9001
```

---

## 4. Distribuição no macOS

- **Homebrew Tap:** Criação de fórmula `brew install ivelot/tap/hal9001`;
- **Standalone Binary:** Download direto de arquivo binário universal assinado.
