#!/usr/bin/env bash
# HAL-9001 — diagnóstico de dependências de sistema + build.
# Uso: bin/setup.sh [--build] [--run]
set -euo pipefail

BLU='\033[0;34m'; GRN='\033[0;32m'; YEL='\033[0;33m'; RED='\033[0;31m'; RST='\033[0m'

info() { printf "${BLU}▎${RST} %s\n" "$1"; }
ok()   { printf "${GRN}✓${RST} %s\n" "$1"; }
warn() { printf "${YEL}!${RST} %s\n" "$1"; }
err()  { printf "${RED}✗${RST} %s\n" "$1"; }

check_bin() {
  if command -v "$1" >/dev/null 2>&1; then ok "$1 encontrado"; else warn "$1 ausente — $2"; fi
}

info "HAL-9001 :: diagnóstico do ambiente"

# --- Toolchain Rust ---
if command -v cargo >/dev/null 2>&1; then
  ok "cargo $(cargo --version | awk '{print $2}')"
else
  err "cargo não encontrado. Instale via https://rustup.rs"
  exit 1
fi

# --- Serviços de sistema esperados pelos backends ---
info "Serviços de sistema (degradação graciosa se ausentes):"
check_bin nmcli    "NetworkManager (aba Wi-Fi)"
check_bin bluetoothctl "bluez (aba Bluetooth)"
check_bin udisksctl "UDisks2 (aba Discos)"
check_bin upower   "UPower (aba Energia)"
check_bin wpctl    "pipewire/wireplumber (volume no Overview)"
check_bin yazi     "Yazi (aba Arquivos)"

# --- Detecção de distro para a aba de updates ---
if [ -r /etc/os-release ]; then
  . /etc/os-release
  ok "Distro: ${PRETTY_NAME:-$ID}"
fi

# --- Ações opcionais ---
if [[ "${1:-}" == "--build" || "${2:-}" == "--build" ]]; then
  info "Compilando (release)…"
  cargo build --release
  ok "Binário: target/release/hal9001"
fi

if [[ "${1:-}" == "--run" || "${2:-}" == "--run" ]]; then
  info "Executando…"
  cargo run
fi

ok "Diagnóstico concluído."
