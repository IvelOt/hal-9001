#!/usr/bin/env bash
# HAL-9001 Universal Install Script

set -euo pipefail

# --- Colors and Badges ---
C_RESET="\033[0m"
C_BLUE="\033[1;34m"
C_GREEN="\033[1;32m"
C_YELLOW="\033[1;33m"
C_RED="\033[1;31m"
C_CYAN="\033[1;36m"

BADGE_HAL="[${C_BLUE}HAL-9001${C_RESET}]"
BADGE_OK="[${C_GREEN}OK${C_RESET}]"
BADGE_WARN="[${C_YELLOW}AVISO${C_RESET}]"
BADGE_ERR="[${C_RED}ERRO${C_RESET}]"
BADGE_DL="[${C_CYAN}DOWNLOAD${C_RESET}]"
BADGE_INST="[${C_GREEN}INSTALADO${C_RESET}]"

print_msg() { echo -e "${BADGE_HAL} $1"; }
print_ok() { echo -e "${BADGE_OK} $1"; }
print_warn() { echo -e "${BADGE_WARN} $1"; }
print_err() { echo -e "${BADGE_ERR} $1"; }
print_dl() { echo -e "${BADGE_DL} $1"; }
print_inst() { echo -e "${BADGE_INST} $1"; }

# --- OS Detection ---
OS="$(uname -s)"
if [ "${OS}" != "Linux" ]; then
    print_err "Sistema operacional nao suportado: ${OS}. O HAL-9001 requer Linux."
    exit 1
fi
print_ok "OS detectado: Linux"

# --- Arch Detection ---
ARCH="$(uname -m)"
case "${ARCH}" in
    x86_64|amd64)
        TARGET="x86_64-unknown-linux-musl"
        ;;
    aarch64|arm64)
        TARGET="aarch64-unknown-linux-gnu"
        ;;
    *)
        print_err "Arquitetura nao suportada: ${ARCH}"
        exit 1
        ;;
esac
print_ok "Arquitetura detectada: ${ARCH} (Target: ${TARGET})"

# --- Version Detection ---
print_msg "Verificando a ultima versao..."
API_URL="https://api.github.com/repos/IvelOt/hal-9001/releases/latest"
FALLBACK_VERSION="0.1.3"

LATEST_VERSION=""
if command -v curl >/dev/null 2>&1; then
    LATEST_VERSION=$(curl -fsSL --max-time 5 "${API_URL}" | grep '"tag_name":' | sed -E 's/.*"([^"]+)".*/\1/' | sed 's/^v//' || true)
fi

if [ -z "${LATEST_VERSION}" ]; then
    print_warn "Falha ao obter versao pela API do GitHub. Usando fallback v${FALLBACK_VERSION}"
    LATEST_VERSION="${FALLBACK_VERSION}"
fi
print_ok "Versao alvo: v${LATEST_VERSION}"

# --- Setup Paths ---
if [ "$(id -u)" -eq 0 ]; then
    BIN_DIR="/usr/local/bin"
    APP_DIR="/usr/local/share/applications"
else
    BIN_DIR="${HOME}/.local/bin"
    APP_DIR="${HOME}/.local/share/applications"
fi

mkdir -p "${BIN_DIR}"
mkdir -p "${APP_DIR}"

# --- Download ---
TARBALL="hal-9001-${LATEST_VERSION}-${TARGET}.tar.gz"
DOWNLOAD_URL="https://github.com/IvelOt/hal-9001/releases/download/v${LATEST_VERSION}/${TARBALL}"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT
cd "${TMP_DIR}"

print_dl "Baixando ${TARBALL}..."
if ! curl -fsSL -o "${TARBALL}" "${DOWNLOAD_URL}"; then
    print_err "Falha ao baixar arquivo de release de ${DOWNLOAD_URL}"
    exit 1
fi
print_ok "Download concluido."

# --- Extract & Install ---
print_msg "Extraindo arquivo..."
tar -xzf "${TARBALL}"

print_msg "Instalando binario em ${BIN_DIR}..."
cp "hal9001" "${BIN_DIR}/hal9001"
chmod +x "${BIN_DIR}/hal9001"
ln -sf "${BIN_DIR}/hal9001" "${BIN_DIR}/hal-9001"

print_msg "Instalando desktop entry em ${APP_DIR}..."
cat <<EOF > "${APP_DIR}/hal-9001.desktop"
[Desktop Entry]
Version=1.0
Type=Application
Name=HAL-9001
Comment=TUI System Control Hub
Exec=${BIN_DIR}/hal9001
Terminal=true
Categories=System;Monitor;ConsoleOnly;
EOF

print_inst "Instalacao finalizada com sucesso."

# --- PATH Warning ---
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    print_warn "O diretorio ${BIN_DIR} nao esta no seu \$PATH."
    print_warn "Adicione 'export PATH=\"\$PATH:${BIN_DIR}\"' ao seu ~/.bashrc ou ~/.zshrc."
fi

print_msg "Você pode executar o HAL-9001 digitando: hal-9001"
