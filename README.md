# HALL-9001

**HALL-9001** is an advanced, high-performance Rust terminal system monitor and hardware control hub featuring a modern, btop-inspired TUI. It provides comprehensive system telemetry alongside active hardware, network, bluetooth, and storage management from a single responsive dashboard.

![HALL-9001 Overview Dashboard](assets/screenshots/tab1_overview.png)

## Overview

Inspired by classic sci-fi aesthetics and engineered for maximum efficiency, HALL-9001 goes far beyond passive monitoring. Built with **100% Pure Rust** and direct asynchronous **System D-Bus (`zbus`)** integration, it requires **zero external C libraries** and zero CLI wrappers for core subsystems.

---

## Screenshots

| Overview & Telemetry (Tab 1) | Detailed Overview & Top Processes (Tab 1 `[.]`) |
|:---:|:---:|
| ![Overview](assets/screenshots/tab1_overview.png) | ![Detailed Overview](assets/screenshots/tab1_overview_detailed.png) |

| Network & Wi-Fi Access Points (Tab 2) | Bluetooth & Peripheral Devices (Tab 3) |
|:---:|:---:|
| ![Network Management](assets/screenshots/tab2_network.png) | ![Bluetooth Management](assets/screenshots/tab3_bluetooth.png) |

| Storage, Drives & Partitions (Tab 4) | Native Disk Space Analyzer (Tab 4 `[a]`) |
|:---:|:---:|
| ![Storage Management](assets/screenshots/tab4_storage.png) | ![Disk Space Analyzer](assets/screenshots/tab4_disk_analyzer.png) |

| PipeWire & PulseAudio Mixer (Tab 5) | Displays & Virtual Canvas Auto-Expand (Tab 6) |
|:---:|:---:|
| ![Audio Mixer](assets/screenshots/tab5_audio.png) | ![Displays Management](assets/screenshots/tab6_displays.png) |

---

## Built-in Themes & Color Palettes

HALL-9001 includes 8 built-in themes out of the box, switchable in real-time via the settings modal (`[c]` / `[F2]`) or in `config.toml`:

| HAL Classic (Default) | Catppuccin Mocha |
|:---:|:---:|
| ![HAL Classic](assets/screenshots/themes/theme_hal.png) | ![Catppuccin Mocha](assets/screenshots/themes/theme_catppuccin.png) |

| Dracula | Gruvbox Dark |
|:---:|:---:|
| ![Dracula](assets/screenshots/themes/theme_dracula.png) | ![Gruvbox Dark](assets/screenshots/themes/theme_gruvbox.png) |

| Nord Arctic | Tokyo Night |
|:---:|:---:|
| ![Nord](assets/screenshots/themes/theme_nord.png) | ![Tokyo Night](assets/screenshots/themes/theme_tokyonight.png) |

| Cyberpunk 2077 | Minimal Monochrome |
|:---:|:---:|
| ![Cyberpunk](assets/screenshots/themes/theme_cyberpunk.png) | ![Monochrome](assets/screenshots/themes/theme_monochrome.png) |

---

## Features & Architecture

### 1. System Overview & Telemetry (Tab 1)
- Real-time CPU, RAM, Swap, Disk I/O, Network throughput, and Top 5 Process metrics.
- Hardware sensor monitoring (CPU temperatures, thermal throttles, fan speeds).
- Backlight and audio volume controls with native keybindings (`[b]/[B]`, `[v]/[V]`, `[m]`).
- Keyboard illumination control (`[j]/[k]` in peripherals) and unified Airplane Mode radio killswitch (`[A]`).
- Power profile switcher (`[p]/[P]` cycling *Power-Saver*, *Balanced*, *Performance*).

### 2. Wi-Fi & Network (Tab 2 — Pure Rust)
- **100% Pure Rust D-Bus:** Direct asynchronous communication with `org.freedesktop.NetworkManager` via `zbus`.
- **Zero CLI Wrappers:** No reliance on `nmcli`, `iw`, or C-FFI.
- Access Point discovery with signal strength bars, frequency bands (2.4GHz, 5GHz, 6GHz Wi-Fi 6E/7), and security badges (WPA2, WPA3 SAE, OWE).
- Masked password input modal for encrypted networks.
- Real-time network interface telemetry (RX/TX throughput and total transfer).

### 3. Bluetooth & Peripheral Hub (Tab 3 — Pure Rust)
- **100% Pure Rust D-Bus:** Direct asynchronous communication with `org.bluez` (`Adapter1`, `Device1`, `Battery1`, `ObjectManager`).
- **Zero C Dependencies:** No `libbluetooth`, `glib`, `bluez-libs`, or `bluetoothctl` calls.
- Device discovery (BLE & Classic) with 30-second battery-saving auto-timeout.
- Smart categorization: Audio/Headsets, Gamepads/Controllers, Keyboards, Mice, Phones, PCs.
- Live battery level telemetry (`org.bluez.Battery1`) for TWS earbuds and headsets.
- One-key actions: Connect/Disconnect (`[Enter]`), Pair (`[p]`), Scan (`[r]`), Forget (`[f]`), Radio On/Off (`[t]`), Block/Unblock (`[b]`).

### 4. Storage, Partitioning, ISO Flasher & Disk Analyzer (Tab 4)
- Simplified, drive-centric view with hierarchical partition tree (`org.freedesktop.UDisks2`).
- **Native Disk Space Analyzer (`[a]`):** Pure Rust recursive directory size scanner with live streaming progress, animated spinner, and drill-down navigation (`[Enter]` dive in, `[Backspace]` go up).
- **5-Layer Safety Lock:** Hard protection preventing accidental format, eject, or flash operations on system/root disks.
- **Pure Rust FAT32 Formatting:** Embedded volume formatting using `fatfs` without requiring `dosfstools`/`mkfs.vfat`.
- **Bootable ISO Flasher:** Raw image flasher with SHA-256 integrity verification, speed/ETA calculator, and file picker.
- **Multi-Boot / Ventoy Manager:** Prepare USB drives and manage ISO collections in `/ISOs/` directly from the TUI.
- **Native Masked Sudo Elevation:** Secure in-TUI password modal (`*`) for privileged storage actions.

### 5. Audio Mixer & Hardware Hub (Tab 5 — Pure Rust)
- **PipeWire & PulseAudio Engine:** Native asynchronous integration with WirePlumber / PipeWire (`wpctl`) and PulseAudio fallback.
- **3 Specialized Sub-Panels:**
  - **`[1] Audio Outputs (Sinks)`**: Internal Speakers, Headphones (P2/Bluetooth A2DP), HDMI/DisplayPort audio.
  - **`[2] Applications (Streams)`**: Individual volume sliders and mute toggles per running app (**Spotify**, **Firefox/Chrome**, **Discord**, **Steam**, **VLC**, games).
  - **`[3] Microphones (Sources)`**: Input gain and mute control for internal mics, headsets, and USB microphones.
- **Volume Overdrive (0..=150%):** Visual color progression (accent -> green -> yellow/red overdrive).
- **One-Key Shortcuts:** Volume (`[+/-]` or `[h/l]`), Mute (`[m]`), Set Default Device (`[Enter]`), Switch Category (`[Tab]` or `[1/2/3]`).

### 6. Displays, Monitors & Auto-Expand Hub (Tab 6 — Wayland & X11)
- **Multi-Server Detection:** Dynamic support for Wayland (`wlr-randr`, `hyprctl`) and X11 (`xrandr`).
- **Automatic Hotplug Auto-Expand:** Instantly detects when an external monitor (HDMI/DisplayPort/USB-C) is connected and automatically activates **Extend-Right Mode** with instant TUI toast notifications.
- **Interactive 2D Canvas Diagram:** Spatial ASCII representation of connected screens with real-time resolution, position, primary badge, and refresh rates.
- **Display Modes:**
  - `[1] Extend Right (Default)`
  - `[2] Extend Left`
  - `[3] Mirror Screens`
  - `[4] External Monitor Only`
  - `[5] Notebook Screen Only`
- **Output Management:** Set Primary Display (`[p]`), change resolutions/Hz, and toggle displays.
- **Unified Hardware Toast System:** Statusline notifications for Monitor connect/disconnect, USB insertions/ejections, Bluetooth pairing, and network transitions.

---

## Global Keybindings

| Key | Action |
|---|---|
| `1` .. `6` / `Tab` / `Shift-Tab` | Switch active tab |
| `j` / `k` or `Down` / `Up` | Navigate device/item lists |
| `Enter` | Primary action (Connect, Mount/Unmount, Confirm, Drill down) |
| `r` | Refresh snapshot / Trigger active rescan |
| `.` | Toggle normal vs. expanded overview telemetry |
| `c` / `F2` | Open interactive settings & theme configuration |
| `b` / `B` | Decrease / Increase screen brightness |
| `v` / `V` | Decrease / Increase audio volume (`m` for mute) |
| `p` / `P` | Cycle system power profiles |
| `?` | Toggle in-app help modal |
| `q` / `Ctrl-C` | Exit HALL-9001 |

---

## Installation & Getting Started

### Build from Source (Current Recommended Method)

Ensure you have Rust stable (1.80+) installed.

```bash
# 1. Clone the repository
git clone https://github.com/IvelOt/hal-9001.git
cd hal-9001

# 2. Run the test suite (100+ unit and integration tests)
cargo test

# 3. Compile the optimized release binary
cargo build --release

# 4. Run HALL-9001
./target/release/hal9001
```

### System Requirements & Graceful Degradation

HAL-9001 interacts with system daemons directly over asynchronous D-Bus (`zbus`). If any service is absent or not running, the application gracefully degrades without panicking:

| Subsystem | Required Daemon / Service | Fallback Behavior |
|:---|:---|:---|
| **System & Telemetry** | Linux `/proc` and `/sys` | Standard sysinfo metrics |
| **Wi-Fi & Network** | `NetworkManager` (`org.freedesktop.NetworkManager`) | Shows network card info without active scan |
| **Bluetooth** | BlueZ (`org.bluez`) | Displays "Bluetooth unavailable" badge |
| **Storage & Disks** | UDisks2 (`org.freedesktop.UDisks2`) | Falls back to mount table inspection |
| **Audio Mixer** | PipeWire (`wpctl`) or PulseAudio | Volume sliders disabled if daemon absent |
| **Displays** | `wlr-randr` / `hyprctl` (Wayland) or `xrandr` (X11) | Reads DRM connector status |

---

## Distribution Roadmap (Upcoming Packaging Pipelines)

Packaging configurations and specifications are currently prepared in `packaging/`. Public distribution channels are being deployed in the next milestone:

| Channel / Package Manager | Target Platform | Target Installation Command | Status |
|:---|:---|:---|:---:|
| **Cargo (Crates.io)** | Universal (Linux x86_64, aarch64) | `cargo install --locked hal-9001` | Planned (Next Step) |
| **Arch Linux (AUR)** | Arch, Manjaro, EndeavourOS | `paru -S hal-9001` / `hal-9001-bin` | Planned (Next Step) |
| **Debian / Ubuntu (.deb)** | Debian 12+, Ubuntu 22.04+, Mint | `sudo dpkg -i hal-9001_amd64.deb` | Planned (Next Step) |
| **NixOS & Flakes** | NixOS, Linux with Nix | `nix run github:IvelOt/hal-9001` | Planned (Next Step) |
| **Fedora / RHEL (RPM)** | Fedora Copr, openSUSE | `sudo dnf install hal-9001` | Planned (Next Step) |
| **Standalone Tarball** | Portable Musl Binary | GitHub Releases download | Planned (Next Step) |

---

## Configuration & Themes

HALL-9001 searches for configuration in order:
1. `$HAL9001_CONFIG`
2. `~/.config/hal-9001/config.toml`
3. `./config.toml`

Customizable options include UI refresh rates (15/30/60 FPS), language (`auto`, `pt-BR`, `en-US`, `es-ES`), Nerd Font icon toggles, ASCII logo styles, polling intervals, and color palettes (*HAL Classic*, *Monochrome*, *Catppuccin*, *Dracula*, *Gruvbox*, *Nord*, *Tokyo Night*, *Cyberpunk*).
