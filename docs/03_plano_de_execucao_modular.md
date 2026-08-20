# 03 — Plano de Execução Modular

Roadmap em módulos atômicos. Cada módulo é entregável, compilável e testável de forma isolada.
Estado atual do repositório: **Módulo 0 concluído** (skeleton compilável). Demais módulos preenchem os *stubs*.

---

## Módulo 0 — Harness & Skeleton ✅ (este commit)

- `Cargo.toml` com todas as dependências.
- Estrutura de pastas `src/{app,config,ascii,events,backend,ui}`.
- Loop principal Tokio + `crossterm` EventStream + render tick.
- Splash animada → Overview.
- 8 abas navegáveis com *placeholders*; Overview já lê dados reais via `sysinfo`.
- `Config` carregando `config.toml` com defaults.
- `bin/setup.sh` + `Makefile`.
- **Critério de aceite:** `cargo build` limpo; `cargo run` mostra splash → dashboard navegável; `q` sai restaurando o terminal.

## Módulo 1 — Overview completo

- Backend `system.rs`: OS/kernel/uptime/shell/pacotes; barras de CPU/RAM.
- Brilho (`/sys/class/backlight`) e Volume (`wpctl`).
- Paleta de cores renderizada.
- **Aceite:** todos os campos do neofetch preenchidos e atualizando por polling.

## Módulo 2 — Wi-Fi / Rede (NetworkManager)

- `network.rs` via `zbus`: listar APs, sinal, segurança; conectar/desconectar/esquecer; toggle rádio.
- Modal de senha; IP/tráfego.
- **Aceite:** conectar a uma rede WPA2 real; refletir estado; degradar se NM ausente.

## Módulo 3 — Bluetooth (bluez)

- `bluetooth.rs`: descoberta, pair/connect/disconnect/remove; bateria via `Battery1`.
- **Aceite:** parear e conectar um fone; ver bateria.

## Módulo 4 — Discos & Armazenamento (UDisks2)

- `storage.rs`: enumerar block devices/partições; montar/desmontar/ejetar.
- **Aceite:** montar/desmontar um pendrive USB com confirmação.

## Módulo 5 — Energia & Bateria (UPower)

- `power.rs`: saúde, ciclos, watts, tempo restante; perfis via power-profiles-daemon.
- Sparkline de consumo.
- **Aceite:** ler bateria de laptop e trocar de perfil.

## Módulo 6 — Atualizações

- `updates.rs`: detectar distro; contar pendentes (Arch/Debian); disparar update em PTY.
- **Aceite:** contagem correta em Arch e Debian; update roda visível.

## Módulo 7 — Yazi Integration

- `pty.rs` + `ui/files.rs`: lançar Yazi embutido; suspensão/retorno de raw mode sem artefatos.
- **Aceite:** navegar no Yazi e voltar à TUI com redraw limpo.

## Módulo 8 — Terminal Deck (PTY + VT100)

- Parser VT mínimo; foco de input; leader de escape.
- **Aceite:** rodar comandos interativos (`htop`, `vim`) dentro do deck.

---

## Ordem recomendada & dependências

```
M0 ──▶ M1 ──▶ (M2, M3, M4, M5, M6 em paralelo) ──▶ M7 ──▶ M8
                    │                                 ▲
                    └── PTY compartilhado (pty.rs) ───┘
```

`pty.rs` é fundação comum de M6/M7/M8 — priorizar seu esqueleto cedo.

## Estratégia de Testes

- **Unit:** parsing de `os-release`, keymap, cálculo de barras/percentuais, detecção de distro.
- **Integração:** backends com *mock* de D-Bus (trait `Service` trocável) publicando `AppEvent`s determinísticos.
- **Smoke:** `cargo run` em CI headless com `TERM=dumb` só para validar boot/teardown (sem alt-screen).

## Definição de Pronto (por módulo)

1. Compila sem warnings (`cargo clippy -- -D warnings`).
2. Degrada graciosamente quando o serviço de sistema não existe.
3. Atalhos documentados aparecem na statusline.
4. Sem bloqueio da UI (nenhum `.await` de I/O na thread de render).
