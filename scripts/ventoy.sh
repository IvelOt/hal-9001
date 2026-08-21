#!/usr/bin/env bash
# scripts/ventoy.sh — baixa (se necessário), verifica o checksum e instala o
# Ventoy (https://www.ventoy.net) no pendrive `/dev/sdX` informado.
#
# Uso:
#   scripts/ventoy.sh /dev/sdX
#
# Segurança (trava estrita anti-sistema, sem flag de bypass):
#   - Recusa qualquer alvo que não seja um dispositivo de bloco removível
#     (lsblk RM=1) conectado via USB.
#   - Recusa qualquer alvo que hospede uma partição montada em `/`, `/boot`,
#     `/boot/efi` ou `/home`, ou marcada como swap ativa.
#   - Falha fechado: se o checksum SHA256 publicado pela release do GitHub
#     não puder ser localizado/confirmado, a instalação é abortada — nunca
#     grava um binário não verificado.
#
# Chamado pelo hal9001 (Action::StorageVentoyInstall) após confirmação em 2
# etapas na TUI; também pode ser executado manualmente para diagnóstico.
set -euo pipefail
umask 077

log() { printf '[ventoy] %s\n' "$1"; }
die() {
  printf '[ventoy] ERRO: %s\n' "$1" >&2
  exit 1
}

# ---------------------------------------------------------------------------
# Argumentos e validação básica
# ---------------------------------------------------------------------------
DEVICE="${1:-}"
[ -n "$DEVICE" ] || die "uso: $0 /dev/sdX"
[[ "$DEVICE" == /dev/* ]] || die "dispositivo deve começar com /dev/: $DEVICE"
[ -b "$DEVICE" ] || die "não é um dispositivo de bloco: $DEVICE"

for bin in curl tar sha256sum lsblk awk sed; do
  command -v "$bin" >/dev/null 2>&1 || die "dependência ausente: $bin"
done

if [ "$(id -u)" -ne 0 ]; then
  die "é necessário root para gravar em $DEVICE (execute via sudo/pkexec)"
fi

# ---------------------------------------------------------------------------
# Camada de trava anti-sistema — inegociável, sem opção de bypass.
# ---------------------------------------------------------------------------
require_removable_usb() {
  local rm tran
  rm="$(lsblk -dno RM "$DEVICE" 2>/dev/null || true)"
  tran="$(lsblk -dno TRAN "$DEVICE" 2>/dev/null || true)"
  [ "$rm" = "1" ] || die "dispositivo não removível — recusado (trava anti-sistema)"
  if [ -n "$tran" ] && [ "$tran" != "usb" ]; then
    die "dispositivo não é USB (barramento: $tran) — recusado (trava anti-sistema)"
  fi
}

reject_if_mounted_protected() {
  local mp
  while IFS= read -r mp; do
    [ -n "$mp" ] || continue
    case "${mp%/}" in
    "" | /boot | /boot/efi | /home)
      die "partição de $DEVICE está montada em ponto protegido ($mp) — recusado"
      ;;
    esac
  done < <(lsblk -no MOUNTPOINT "$DEVICE" 2>/dev/null)

  if command -v findmnt >/dev/null 2>&1; then
    local mnt src
    for mnt in / /boot /boot/efi /home; do
      src="$(findmnt -no SOURCE "$mnt" 2>/dev/null || true)"
      if [ -n "$src" ] && [[ "$src" == "$DEVICE"* ]]; then
        die "$DEVICE hospeda o ponto de montagem protegido $mnt — recusado"
      fi
    done
  fi
}

reject_if_active_swap() {
  [ -r /proc/swaps ] || return 0
  if awk -v d="$DEVICE" 'NR>1 && index($1,d)==1 {found=1} END{exit !found}' /proc/swaps; then
    die "$DEVICE possui uma partição de swap ativa — recusado"
  fi
}

log "validando trava de segurança para $DEVICE"
require_removable_usb
reject_if_mounted_protected
reject_if_active_swap
log "trava de segurança OK — $DEVICE é removível/USB e não hospeda pontos de montagem de sistema"

# ---------------------------------------------------------------------------
# Download (se necessário) + verificação de checksum + cache local
# ---------------------------------------------------------------------------
GITHUB_API="https://api.github.com/repos/ventoy/Ventoy/releases/latest"
CACHE_DIR="${HAL9001_VENTOY_CACHE:-$HOME/.cache/hal9001/ventoy}"
mkdir -p "$CACHE_DIR"

find_cached_install() {
  find "$CACHE_DIR" -mindepth 1 -maxdepth 1 -type d -name 'ventoy-*' 2>/dev/null | sort -V | tail -n1
}

ensure_ventoy_available() {
  local existing
  existing="$(find_cached_install)"
  if [ -n "$existing" ] && [ -f "$existing/Ventoy2Disk.sh" ]; then
    log "Ventoy já disponível em cache: $(basename "$existing")"
    printf '%s\n' "$existing"
    return 0
  fi

  log "consultando a última versão do Ventoy no GitHub..."
  local json tag version asset_name asset_url expected_sha sha_url tmp_tar actual_sha extracted
  json="$(curl -fsSL --retry 3 --retry-delay 2 -H "Accept: application/vnd.github+json" "$GITHUB_API")" ||
    die "falha ao consultar a API do GitHub (releases do Ventoy)"

  tag="$(printf '%s' "$json" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
  [ -n "$tag" ] || die "não foi possível determinar a versão mais recente do Ventoy"
  version="${tag#v}"
  asset_name="ventoy-${version}-linux.tar.gz"

  asset_url="$(printf '%s' "$json" |
    grep -o "\"browser_download_url\"[[:space:]]*:[[:space:]]*\"[^\"]*${asset_name}\"" |
    sed -E 's/.*"(https[^"]+)".*/\1/' | head -n1)"
  [ -n "$asset_url" ] || die "asset $asset_name não encontrado na release $tag"

  tmp_tar="$CACHE_DIR/${asset_name}"
  log "baixando $asset_name (release $tag)..."
  curl -fSL --retry 3 --retry-delay 2 -o "$tmp_tar" "$asset_url" || die "falha ao baixar $asset_name"

  # A release do Ventoy publica os hashes SHA256 no corpo da nota de versão
  # (ex.: "sha256sum:\nXXXX  ventoy-X.Y.Z-linux.tar.gz"). Também tenta um
  # asset dedicado `<arquivo>.sha256`, publicado em algumas releases.
  expected_sha="$(printf '%s' "$json" |
    grep -oE "[0-9a-f]{64}[^0-9a-zA-Z]{1,4}${asset_name}" |
    grep -oE '^[0-9a-f]{64}' | head -n1 || true)"
  if [ -z "$expected_sha" ]; then
    sha_url="$(printf '%s' "$json" |
      grep -o "\"browser_download_url\"[[:space:]]*:[[:space:]]*\"[^\"]*${asset_name}.sha256\"" |
      sed -E 's/.*"(https[^"]+)".*/\1/' | head -n1)"
    if [ -n "$sha_url" ]; then
      expected_sha="$(curl -fsSL "$sha_url" 2>/dev/null | awk '{print $1}' | head -n1)"
    fi
  fi

  if [ -z "$expected_sha" ]; then
    rm -f "$tmp_tar"
    die "checksum SHA256 publicado não encontrado para $asset_name — abortando por segurança (falha fechado)"
  fi

  actual_sha="$(sha256sum "$tmp_tar" | awk '{print $1}')"
  if [ "$actual_sha" != "$expected_sha" ]; then
    rm -f "$tmp_tar"
    die "checksum SHA256 não confere para $asset_name (esperado $expected_sha, obtido $actual_sha) — abortando"
  fi
  log "checksum SHA256 de $asset_name verificado com sucesso"

  tar -xzf "$tmp_tar" -C "$CACHE_DIR"
  rm -f "$tmp_tar"
  extracted="$CACHE_DIR/ventoy-${version}"
  [ -d "$extracted" ] || die "extração do Ventoy falhou (diretório esperado ausente: $extracted)"
  chmod +x "$extracted/Ventoy2Disk.sh" 2>/dev/null || true
  printf '%s\n' "$extracted"
}

VENTOY_DIR="$(ensure_ventoy_available)"
log "usando Ventoy em: $VENTOY_DIR"

# ---------------------------------------------------------------------------
# Instalação (destrutiva) — segunda revalidação da trava imediatamente antes
# da escrita, mitigando TOCTOU entre o parsing acima e a execução real.
# ---------------------------------------------------------------------------
require_removable_usb
reject_if_mounted_protected
reject_if_active_swap

log "iniciando instalação do Ventoy em $DEVICE — todos os dados serão apagados"
yes | "$VENTOY_DIR/Ventoy2Disk.sh" -I "$DEVICE" || die "Ventoy2Disk.sh falhou ao instalar em $DEVICE"

sync
log "instalação do Ventoy concluída em $DEVICE"
