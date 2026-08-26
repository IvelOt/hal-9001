# 11 — Guia de Deploy, Empacotamento e Distribuição

> **HAL-9001** — Central TUI de Controle do Sistema & Assistente de Sistema  
> **Versão:** `0.1.0` | **Licença:** `MIT` | **Repositório:** [https://github.com/IvelOt/hall-9001](https://github.com/IvelOt/hall-9001)

---

## 1. Filosofia de Empacotamento & Degradação Graciosa

O **HAL-9001** foi projetado seguindo princípios rigorosos de portabilidade, segurança e independência de binários externos no núcleo:

1. **100% Rust Puro no Core:**
   - Comunicação com subsistemas do sistema operacional através de D-Bus assíncrono nativo (`zbus 5` com `tokio`).
   - Zero dependências de bibliotecas dinâmicas pesadas em C (zero `libbluetooth-dev`, `libnm-dev`, `libudev-dev`, etc.).
   - Formatação de partições FAT32 em Rust puro (`fatfs`), permitindo formatar pendrives sem `dosfstools` ou `mkfs.vfat`.
   - Descompressão em streaming de imagens de disco com `flate2` configurado com backend `miniz_oxide` (100% Rust puro).
2. **Degradação Graciosa (*Graceful Degradation*):**
   - Se um daemon de sistema (ex: `NetworkManager`, `bluez`, `UDisks2`, `UPower`, `pipewire`/`pulseaudio`) não estiver em execução ou não estiver instalado, o HAL-9001 não quebra nem entra em pânico. A respectiva aba entra em modo "Indisponível / N/A" e as outras abas continuam operando normalmente.
3. **Distribuição Multi-Plataforma:**
   - Suporte oficial para 4 frentes de distribuição:
     - **Arch Linux (AUR):** Compilação de código-fonte (`hall-9001`) e pacote pré-compilado (`hall-9001-bin`).
     - **Crates.io & Cargo:** Publicação oficial e instalação universal com `cargo install --locked hall-9001`.
     - **Debian & Ubuntu (.deb):** Pacotes binários nativos gerados via `cargo-deb` ou `dpkg-deb`.
     - **NixOS & Flakes:** Suporte nativo a `nix run github:IvelOt/hall-9001` e `nix build`.

---

## 2. Matriz de Distribuição e Canais

| Canal | Tipo de Pacote | Comando de Instalação | Plataformas Suportadas |
|---|---|---|---|
| **AUR (Fonte)** | `PKGBUILD` (`hall-9001`) | `paru -S hall-9001` ou `yay -S hall-9001` | Arch Linux, Manjaro, EndeavourOS (x86_64, aarch64) |
| **AUR (Binário)** | `PKGBUILD.bin` (`hall-9001-bin`) | `paru -S hall-9001-bin` ou `yay -S hall-9001-bin` | Arch Linux, Manjaro, EndeavourOS (x86_64, aarch64) |
| **Crates.io** | Crate Rust | `cargo install --locked hall-9001` | Qualquer Linux, macOS, BSD com Rust 1.80+ |
| **Debian / Ubuntu** | `.deb` | `sudo dpkg -i hall-9001_0.1.0_amd64.deb` | Debian 12+, Ubuntu 22.04+, Pop!_OS, Mint |
| **Nix Flake** | Flake / Derivation | `nix run github:IvelOt/hall-9001` | NixOS, Linux com Nix instalado, macOS com Nix |
| **GitHub Releases** | Tarball com binário estático | `curl -sSL ... \| tar -xz` | Linux (glibc e musl), x86_64 e aarch64 |

---

## 3. Matriz de Dependências por Distribuição

O HAL-9001 possui apenas dependências mínimas de compilação e execução. Todas as ferramentas de hardware são opcionais e ativam recursos adicionais conforme a tabela abaixo:

| Subsistema / Recurso | Arch Linux | Debian / Ubuntu | Fedora | NixOS | Alpine Linux |
|---|---|---|---|---|---|
| **Compilador Rust** | `rust` / `cargo` | `rustc` / `cargo` | `rust` / `cargo` | `pkgs.cargo` `pkgs.rustc` | `rust` `cargo` |
| **D-Bus Daemon** | `dbus` | `dbus` / `libdbus-1-3` | `dbus-libs` | `pkgs.dbus` | `dbus` |
| **Wi-Fi (D-Bus)** | `networkmanager` | `network-manager` | `NetworkManager` | `pkgs.networkmanager` | `networkmanager` |
| **Bluetooth (D-Bus)** | `bluez` | `bluez` | `bluez` | `pkgs.bluez` | `bluez` |
| **Discos & Partições** | `udisks2` | `udisks2` | `udisks2` | `pkgs.udisks2` | `udisks2` |
| **Energia & Bateria** | `upower` | `upower` | `upower` | `pkgs.upower` | `upower` |
| **Áudio (PipeWire)** | `pipewire` / `wireplumber` | `pipewire-bin` | `pipewire-utils` | `pkgs.wireplumber` | `pipewire` |
| **Áudio (PulseAudio)** | `pulseaudio` | `pulseaudio-utils` | `pulseaudio-utils` | `pkgs.pulseaudio` | `pulseaudio-utils` |
| **Monitores (X11)** | `xorg-xrandr` | `x11-xserver-utils` | `xorg-x11-server-utils` | `pkgs.xorg.xrandr` | `xrandr` |
| **Monitores (Wayland)**| `wlr-randr` | `wlr-randr` | `wlr-randr` | `pkgs.wlr-randr` | `wlr-randr` |
| **Monitores (Hyprland)**| `hyprland` | `hyprland` | `hyprland` | `pkgs.hyprland` | `hyprland` |
| **Terminal / Arquivos**| `yazi` | `yazi` | `yazi` | `pkgs.yazi` | `yazi` |
| **Elevação de Root** | `sudo` | `sudo` | `sudo` | `pkgs.sudo` | `sudo` |

---

## 4. Frente 1: Arch Linux & AUR

A estrutura de empacotamento para o Arch User Repository (AUR) está localizada em `packaging/arch/`.

### 4.1. Arquivos Disponibilizados

- `packaging/arch/PKGBUILD`: Empacota a partir da fonte (baixa o tarball de release do GitHub ou compila com cargo).
- `packaging/arch/PKGBUILD.bin`: Empacota a partir do binário pré-compilado disponibilizado nas releases do GitHub.

### 4.2. Testando e Compilando Localmente

Para testar localmente o pacote da fonte no Arch Linux:

```bash
# 1. Navegue até a pasta do PKGBUILD
cd packaging/arch

# 2. Compile e instale no sistema com resolução de dependências
makepkg -si

# 3. Teste a execução do binário instalado
hal9001
# ou
hall-9001
```

Para testar o pacote pré-compilado (`PKGBUILD.bin`):

```bash
cd packaging/arch
cp PKGBUILD.bin PKGBUILD
makepkg -si
```

### 4.3. Gerando o `.SRCINFO` e Publicando no AUR

Antes de enviar commits para o repositório Git do AUR:

```bash
# Gerar o arquivo .SRCINFO obrigatório do AUR
makepkg --printsrcinfo > .SRCINFO

# Validar conformidade com as diretrizes do Arch
namcap PKGBUILD
namcap hall-9001-*.pkg.tar.zst
```

Para publicar no AUR:

```bash
# Clonar o repositório do AUR (após criar o pacote no aur.archlinux.org)
git clone ssh://aur@aur.archlinux.org/hall-9001.git /tmp/aur-hall-9001
cp packaging/arch/PKGBUILD /tmp/aur-hall-9001/PKGBUILD
cd /tmp/aur-hall-9001
makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "chore: release v0.1.0"
git push origin master
```

---

## 5. Frente 2: Crates.io & Instalação Universal com Cargo

O HAL-9001 pode ser publicado como uma crate no repositório central [Crates.io](https://crates.io) e instalado por qualquer usuário com Rust instalado.

### 5.1. Metadados Configurados no `Cargo.toml`

O `Cargo.toml` foi enriquecido com metadados completos de publicação:

```toml
[package]
name = "hall-9001"
version = "0.1.0"
edition = "2021"
rust-version = "1.80"
authors = ["IvelOt <contact@ivelot.dev>"]
description = "HAL-9001 — Central TUI de Controle do Sistema & Assistente de Sistema (Rust/ratatui)"
license = "MIT"
readme = "README.md"
repository = "https://github.com/IvelOt/hall-9001"
homepage = "https://github.com/IvelOt/hall-9001"
documentation = "https://docs.rs/hall-9001"
keywords = ["tui", "system-monitor", "btop", "ratatui", "hardware-control"]
categories = ["command-line-utilities", "system::hardware", "system::monitoring"]
exclude = [
    "assets/screenshots/*",
    "docs/*",
    "packaging/*",
    ".github/*",
]
```

### 5.2. Checklist de Publicação

1. **Verificação Local do Pacote:**
   ```bash
   cargo package --list
   cargo package
   ```
2. **Dry-Run de Publicação:**
   ```bash
   cargo publish --dry-run
   ```
3. **Publicação Oficial:**
   ```bash
   cargo publish
   ```

### 5.3. Instalação Universal pelo Usuário Final

Após publicado no Crates.io, qualquer usuário pode instalar com um único comando:

```bash
cargo install --locked hall-9001
```

O binário `hal9001` será colocado automaticamente em `~/.cargo/bin/hal9001`.

---

## 6. Frente 3: Debian, Ubuntu & Derivados (.deb)

O HAL-9001 fornece suporte completo para empacotamento Debian tanto através do utilitário `cargo-deb` quanto através de um script autônomo com `dpkg-deb`.

### 6.1. Configuração do `[package.metadata.deb]` no `Cargo.toml`

```toml
[package.metadata.deb]
name = "hall-9001"
maintainer = "IvelOt <contact@ivelot.dev>"
copyright = "2026 IvelOt <contact@ivelot.dev>"
license-file = ["LICENSE", "4"]
extended-description = """\
HAL-9001 é uma central de controle e monitor de telemetria de sistema TUI \
moderno e de alta performance escrito em 100% Rust puro com Ratatui e D-Bus assíncrono. \
Permite monitorar CPU/RAM/Discos e controlar Wi-Fi, Bluetooth, Áudio (PipeWire/PulseAudio), \
Displays com auto-expansão e gravação de mídias/ISOs de forma segura."""
depends = "$auto, libdbus-1-3, systemd"
recommends = "pipewire-bin | pulseaudio-utils, network-manager, bluez, udisks2, upower, x11-xserver-utils | wlr-randr"
suggests = "yazi"
section = "utils"
priority = "optional"
assets = [
    ["target/release/hal9001", "usr/bin/hal9001", "755"],
    ["config.toml", "etc/hall-9001/config.toml", "644"],
    ["README.md", "usr/share/doc/hall-9001/README.md", "644"],
    ["packaging/desktop/hall-9001.desktop", "usr/share/applications/hall-9001.desktop", "644"],
]
```

### 6.2. Gerando o Pacote `.deb`

O script `packaging/debian/build-deb.sh` automatiza o fluxo:

```bash
# Executa a compilação e geração do .deb
make deb
# ou
bash packaging/debian/build-deb.sh
```

O arquivo `.deb` é gerado em:
`target/debian/hall-9001_0.1.0_amd64.deb`

### 6.3. Instalando e Testando no Debian / Ubuntu

```bash
# Instalação local do arquivo gerado
sudo dpkg -i target/debian/hall-9001_0.1.0_amd64.deb

# Caso falte alguma dependência recomendada:
sudo apt-get install -f

# Execução
hal9001
```

Para desinstalar:
```bash
sudo apt-get remove hall-9001
```

---

## 7. Frente 4: NixOS & Nix Flakes

O HAL-9001 inclui suporte nativo a **Nix Flakes** e derivações clássicas do Nixpkgs.

### 7.1. Arquivos Criados

- `default.nix`: Derivação `rustPlatform.buildRustPackage` com encapsulamento de binários opcionais via `makeWrapper` e links simbólicos.
- `flake.nix`: Flake com outputs para `packages.default`, `apps.default` e `devShells.default`.

### 7.2. Execução Instantânea (Nix Run)

Sem precisar instalar nada permanentemente, qualquer usuário com Nix Flakes habilitado pode rodar o HAL-9001 diretamente do repositório GitHub:

```bash
nix run github:IvelOt/hall-9001
```

### 7.3. Compilação com `nix build`

Para compilar e gerar o link simbólico `result/bin/hal9001`:

```bash
nix build
./result/bin/hal9001
```

### 7.4. Shell de Desenvolvimento (`nix develop`)

Para carregar todo o toolchain de desenvolvimento (Rust, Cargo, Clippy, rust-analyzer, D-Bus, systemd):

```bash
nix develop
```

### 7.5. Integração no `configuration.nix` (NixOS)

Adicione o flake aos inputs do seu `flake.nix` do sistema:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    hall-9001.url = "github:IvelOt/hall-9001";
  };

  outputs = { self, nixpkgs, hall-9001, ... }: {
    nixosConfigurations.meu-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ({ pkgs, ... }: {
          environment.systemPackages = [
            hall-9001.packages.${pkgs.system}.default
          ];
        })
      ];
    };
  };
}
```

---

## 8. Estrutura de Pastas de Empacotamento

A organização dos arquivos de empacotamento no projeto segue a estrutura padronizada:

```
projects/hall-9001/
├── Cargo.toml                       # Metadados de publicação Crates.io e cargo-deb
├── LICENSE                          # Licença MIT
├── Makefile                         # Targets de build, test, deb e publish-check
├── default.nix                      # Derivação Nix para NixOS / Nixpkgs
├── flake.nix                        # Definição do Flake Nix
├── packaging/
│   ├── arch/
│   │   ├── PKGBUILD                 # Build da fonte para AUR (hall-9001)
│   │   └── PKGBUILD.bin             # Build do binário pré-compilado para AUR (hall-9001-bin)
│   ├── debian/
│   │   ├── build-deb.sh             # Script unificado de geração .deb (cargo-deb / dpkg-deb)
│   │   └── control                  # Template de controle Debian para dpkg-deb
│   └── desktop/
│       └── hall-9001.desktop        # Entrada XDG Desktop com Terminal=true
└── docs/
    └── 11_guia_de_deploy_e_distribuicao.md # Este documento de especificação
```

---

## 9. Pipeline Automatizado de CI/CD para Releases (GitHub Actions)

Abaixo está o modelo de workflow para `.github/workflows/release.yml` para compilação cruzada automatizada e publicação de releases multi-arquitetura:

```yaml
name: Release Multi-Platform

on:
  push:
    tags:
      - 'v*'

jobs:
  build-release:
    name: Build ${{ matrix.target }}
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            artifact_name: hal9001
            use_cross: false
          - os: ubuntu-latest
            target: x86_64-unknown-linux-musl
            artifact_name: hal9001
            use_cross: true
          - os: ubuntu-latest
            target: aarch64-unknown-linux-gnu
            artifact_name: hal9001
            use_cross: true

    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}

      - name: Install dependencies (Ubuntu)
        run: sudo apt-get update && sudo apt-get install -y libdbus-1-dev pkg-config

      - name: Build with Cargo or Cross
        run: |
          if [ "${{ matrix.use_cross }}" = "true" ]; then
            cargo install cross --git https://github.com/cross-rs/cross
            cross build --release --target ${{ matrix.target }} --locked
          else
            cargo build --release --target ${{ matrix.target }} --locked
          fi

      - name: Package Tarball
        run: |
          TAG_NAME=${GITHUB_REF#refs/tags/}
          ARCHIVE="hal9001-${TAG_NAME}-${{ matrix.target }}.tar.gz"
          tar -czf "$ARCHIVE" -C target/${{ matrix.target }}/release hal9001 -C ../../ config.toml README.md LICENSE
          echo "ARCHIVE_NAME=$ARCHIVE" >> $GITHUB_ENV

      - name: Upload Release Artifact
        uses: softprops/action-gh-release@v2
        with:
          files: ${{ env.ARCHIVE_NAME }}
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

---

## 10. Resumo e Próximos Passos

Com essa infraestrutura:
1. O HAL-9001 está pronto para submissão imediata no **AUR** (`hall-9001` e `hall-9001-bin`).
2. O `Cargo.toml` está 100% validado para o **Crates.io** (`cargo publish`).
3. O pacote **Debian/Ubuntu (.deb)** pode ser gerado em um único comando (`make deb`).
4. O **Nix Flake** permite execução instantânea com `nix run github:IvelOt/hall-9001`.
5. A documentação técnica detalhada fica disponível centralizadamente em `docs/11_guia_de_deploy_e_distribuicao.md`.
