#!/usr/bin/env bash
# ==============================================================================
# HAL-9001 — Gerador de Pacote Debian/Ubuntu (.deb)
# Suporta:
#   1. `cargo deb` (se cargo-deb estiver instalado)
#   2. `dpkg-deb` (se dpkg-dev estiver instalado)
#   3. Fallback universal nativo com `ar` + `tar` (padrão Debian Binary 2.0)
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../.." && pwd)"

cd "${ROOT_DIR}"

BLU='\033[0;34m'; GRN='\033[0;32m'; YEL='\033[0;33m'; RED='\033[0;31m'; RST='\033[0m'
info() { printf "${BLU}▎${RST} %s\n" "$1"; }
ok()   { printf "${GRN}✓${RST} %s\n" "$1"; }
warn() { printf "${YEL}!${RST} %s\n" "$1"; }
err()  { printf "${RED}✗${RST} %s\n" "$1"; }

VERSION=$(grep -m1 '^version' Cargo.toml | awk -F '"' '{print $2}')
ARCH=$(dpkg --print-architecture 2>/dev/null || uname -m)
case "${ARCH}" in
    x86_64) DEB_ARCH="amd64" ;;
    aarch64|arm64) DEB_ARCH="arm64" ;;
    *) DEB_ARCH="${ARCH}" ;;
esac

info "HAL-9001 :: Gerando pacote Debian (.deb) v${VERSION} (${DEB_ARCH})"

# 1. Compilação em release
info "Compilando binário otimizado em release..."
cargo build --release --locked
ok "Binário compilado: target/release/hal9001"

OUTPUT_DIR="${ROOT_DIR}/target/debian"
mkdir -p "${OUTPUT_DIR}"
DEB_FILE="${OUTPUT_DIR}/hall-9001_${VERSION}_${DEB_ARCH}.deb"

if command -v cargo-deb >/dev/null 2>&1; then
    info "Usando 'cargo-deb' para empacotamento..."
    cargo deb --no-build --output "${DEB_FILE}"
elif command -v dpkg-deb >/dev/null 2>&1; then
    info "Usando 'dpkg-deb' para empacotamento..."
    BUILD_ROOT="${OUTPUT_DIR}/pkg-root"
    rm -rf "${BUILD_ROOT}"
    mkdir -p "${BUILD_ROOT}/DEBIAN"
    mkdir -p "${BUILD_ROOT}/usr/bin"
    mkdir -p "${BUILD_ROOT}/usr/share/applications"
    mkdir -p "${BUILD_ROOT}/usr/share/doc/hall-9001"
    mkdir -p "${BUILD_ROOT}/etc/hall-9001"

    sed -e "s/^Version: .*/Version: ${VERSION}/" \
        -e "s/^Architecture: .*/Architecture: ${DEB_ARCH}/" \
        packaging/debian/control > "${BUILD_ROOT}/DEBIAN/control"

    install -m 755 target/release/hal9001 "${BUILD_ROOT}/usr/bin/hal9001"
    ln -sf "/usr/bin/hal9001" "${BUILD_ROOT}/usr/bin/hall-9001"
    install -m 644 packaging/desktop/hall-9001.desktop "${BUILD_ROOT}/usr/share/applications/hall-9001.desktop"
    install -m 644 config.toml "${BUILD_ROOT}/etc/hall-9001/config.toml"
    install -m 644 config.toml "${BUILD_ROOT}/usr/share/doc/hall-9001/config.toml.example"
    install -m 644 README.md "${BUILD_ROOT}/usr/share/doc/hall-9001/README.md"
    install -m 644 LICENSE "${BUILD_ROOT}/usr/share/doc/hall-9001/copyright"

    dpkg-deb --build --root-owner-group "${BUILD_ROOT}" "${DEB_FILE}"
    rm -rf "${BUILD_ROOT}"
else
    info "Usando gerador universal Debian 2.0 nativo ('ar' + 'tar')..."
    TMP_DIR="${OUTPUT_DIR}/deb-build-tmp"
    rm -rf "${TMP_DIR}"
    mkdir -p "${TMP_DIR}/control_dir"
    mkdir -p "${TMP_DIR}/data_dir/usr/bin"
    mkdir -p "${TMP_DIR}/data_dir/usr/share/applications"
    mkdir -p "${TMP_DIR}/data_dir/usr/share/doc/hall-9001"
    mkdir -p "${TMP_DIR}/data_dir/etc/hall-9001"

    # 1. debian-binary
    echo "2.0" > "${TMP_DIR}/debian-binary"

    # 2. Arquivos de dados
    install -m 755 target/release/hal9001 "${TMP_DIR}/data_dir/usr/bin/hal9001"
    ln -sf "/usr/bin/hal9001" "${TMP_DIR}/data_dir/usr/bin/hall-9001"
    install -m 644 packaging/desktop/hall-9001.desktop "${TMP_DIR}/data_dir/usr/share/applications/hall-9001.desktop"
    install -m 644 config.toml "${TMP_DIR}/data_dir/etc/hall-9001/config.toml"
    install -m 644 config.toml "${TMP_DIR}/data_dir/usr/share/doc/hall-9001/config.toml.example"
    install -m 644 README.md "${TMP_DIR}/data_dir/usr/share/doc/hall-9001/README.md"
    install -m 644 LICENSE "${TMP_DIR}/data_dir/usr/share/doc/hall-9001/copyright"

    # Calcula tamanho instalado em KB
    INSTALLED_SIZE=$(du -sk "${TMP_DIR}/data_dir" | awk '{print $1}')

    # 3. Control file
    sed -e "s/^Version: .*/Version: ${VERSION}/" \
        -e "s/^Architecture: .*/Architecture: ${DEB_ARCH}/" \
        packaging/debian/control > "${TMP_DIR}/control_dir/control"
    echo "Installed-Size: ${INSTALLED_SIZE}" >> "${TMP_DIR}/control_dir/control"

    # 4. Gera md5sums
    (
        cd "${TMP_DIR}/data_dir"
        find . -type f ! -path "./DEBIAN/*" -exec md5sum {} + | sed -e 's| \./| |' > "${TMP_DIR}/control_dir/md5sums"
    )

    # 5. Compacta control.tar.gz e data.tar.gz com ownership root:root
    (
        cd "${TMP_DIR}/control_dir"
        tar --owner=0 --group=0 --numeric-owner -czf "${TMP_DIR}/control.tar.gz" .
    )
    (
        cd "${TMP_DIR}/data_dir"
        tar --owner=0 --group=0 --numeric-owner -czf "${TMP_DIR}/data.tar.gz" .
    )

    # 6. Empacota com `ar` no formato exato do Debian
    rm -f "${DEB_FILE}"
    (
        cd "${TMP_DIR}"
        ar -rcD "${ROOT_DIR}/${DEB_FILE}" debian-binary control.tar.gz data.tar.gz
    )
    rm -rf "${TMP_DIR}"
fi

ok "Pacote Debian gerado com sucesso em: ${DEB_FILE}"
info "Tamanho do pacote: $(du -h "${DEB_FILE}" | awk '{print $1}')"
info "Para instalar: sudo dpkg -i ${DEB_FILE} || sudo apt install -f"
