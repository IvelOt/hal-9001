# 01 — Arquitetura & Stack Tecnológica

> HAL-9001 — Central TUI de Controle do Sistema & Assistente de Sistema

## 1. Filosofia de Design

| Princípio | Implicação técnica |
|-----------|--------------------|
| **UI nunca bloqueia** | Todo I/O (D-Bus, CLI, PTY, sysinfo pesado) roda em *tasks* Tokio; a thread de render só consome estado já pronto. |
| **Estado único e imutável no render** | `App` é a fonte da verdade. A UI é uma função pura `fn(&App, Frame)`. |
| **Comunicação por mensagens** | Nada de `Arc<Mutex<...>>` compartilhado entre UI e backends. Fluxo unidirecional via canais. |
| **Backends plugáveis** | Cada subsistema (rede, bt, disco…) implementa um *trait* de serviço e publica `AppEvent`s. Um backend pode ser trocado por um *mock* nos testes. |
| **Degradação graciosa** | Se `NetworkManager`/`bluez`/`UDisks2` não existem no host, a aba entra em modo "indisponível" em vez de derrubar o app. |

## 2. Stack

| Camada | Crate | Papel |
|--------|-------|-------|
| Render TUI | `ratatui` 0.29 | Widgets, layout, buffers de tela. |
| Backend de terminal | `crossterm` 0.28 | Raw mode, alt-screen, `EventStream` assíncrono de teclado/mouse/resize. |
| Runtime async | `tokio` 1.x | Scheduler multi-thread, `mpsc`/`broadcast`, timers, `Command` async. |
| D-Bus | `zbus` 5 (tokio) | NetworkManager, bluez, UDisks2, UPower. |
| Introspecção | `sysinfo` 0.32 | CPU, RAM, uptime, discos, processos (fallback portável). |
| PTY | `portable-pty` 0.8 | Terminal Deck (aba 8) e lançamento do Yazi (aba 7). |
| Config | `serde` + `toml` | `config.toml` de temas/atalhos/polling. |
| Larguras | `unicode-width` | Alinhamento correto de ASCII art e ícones nerd-font. |
| Erros | `anyhow` / `thiserror` | Erros de app vs. erros tipados de backend. |
| Observabilidade | `tracing` | Log estruturado para `hal9001.log` (nunca para stdout em modo TUI). |

## 3. Estrutura de Pastas

```
hal-9001/
├── Cargo.toml
├── Makefile
├── bin/
│   └── setup.sh                 # diagnóstico de dependências de sistema + build
├── config.toml                  # config de exemplo (copiada para ~/.config/hal9001)
├── docs/
│   ├── 01_arquitetura_e_stack.md
│   ├── 02_especificacao_das_abas.md
│   ├── 03_plano_de_execucao_modular.md
│   └── 04_ascii_art_besouro.md
└── src/
    ├── main.rs                  # entrypoint: setup terminal → run → teardown
    ├── lib.rs                   # re-exports p/ testes de integração
    ├── app.rs                   # struct App, estado global, roteamento de Action
    ├── config.rs                # Config + carregamento TOML + defaults
    ├── ascii.rs                 # ASCII arts do besouro
    ├── events/
    │   ├── mod.rs               # AppEvent, Action, EventBus (mpsc)
    │   └── input.rs             # crossterm EventStream → Action (keymap)
    ├── backend/
    │   ├── mod.rs               # trait Service + registry + spawn dos workers
    │   ├── system.rs            # sysinfo → OverviewSnapshot
    │   ├── network.rs           # NetworkManager (D-Bus)
    │   ├── bluetooth.rs         # bluez (D-Bus)
    │   ├── storage.rs           # UDisks2 (D-Bus)
    │   ├── power.rs             # UPower (D-Bus)
    │   ├── updates.rs           # detecção de distro + contagem de pacotes
    │   └── pty.rs               # portable-pty p/ Terminal Deck e Yazi
    └── ui/
        ├── mod.rs               # draw(): dispatch por aba + chrome (tabbar/status)
        ├── theme.rs             # Palette derivada da Config
        ├── widgets.rs           # progress bar, gauge, key/value, sparkline helpers
        ├── splash.rs            # animação LOADING… → Bem-vindo
        ├── overview.rs          # Aba 1
        ├── network.rs           # Aba 2
        ├── bluetooth.rs         # Aba 3
        ├── storage.rs           # Aba 4
        ├── power.rs             # Aba 5
        ├── updates.rs           # Aba 6
        ├── files.rs             # Aba 7 (Yazi)
        └── terminal.rs          # Aba 8 (PTY deck)
```

## 4. Fluxo Assíncrono (Tokio + canais)

```
                 ┌──────────────────────────────────────────────┐
                 │                   App (estado)                │
                 └──────────────────────────────────────────────┘
                       ▲  consome AppEvent          │ emite Action
        AppEvent (mpsc)│                            ▼ (mpsc)
   ┌───────────────────┴───────────┐        ┌───────────────────────────┐
   │        Backend workers        │◀───────│      Event / Input loop    │
   │  (uma task Tokio por serviço) │ Action │  crossterm EventStream →   │
   │  system / net / bt / storage  │broadcast│  Action; ticks de polling  │
   │  power / updates / pty        │        └───────────────────────────┘
   └───────────────────────────────┘
```

- **`AppEvent`** (backend → app): dados novos (`SystemUpdated`, `WifiScan`, `BtDevices`, `Toast`, `PtyOutput`…).
- **`Action`** (input/app → backends): comandos (`WifiConnect`, `BtPair`, `MountDevice`, `RunUpdate`, `PtyInput`…).
- **Canais:**
  - `mpsc<AppEvent>` — muitos produtores (workers) → um consumidor (loop principal).
  - `broadcast<Action>` — um produtor (app) → muitos assinantes (workers), cada worker filtra o que lhe interessa.
  - PTY usa um `mpsc<Vec<u8>>` dedicado por sessão para não misturar backpressure de terminal com o resto.

### Loop principal (pseudo)

```
loop {
    tokio::select! {
        Some(ev) = app_events.recv()  => app.handle_event(ev),   // muta estado
        Some(act) = input.next()      => app.dispatch(act, &bus),// input → Action
        _ = render_tick.tick()        => terminal.draw(|f| ui::draw(&app, f)),
    }
    if app.should_quit { break }
}
```

Render é *tick-driven* (~30–60 fps configurável), desacoplado da chegada de dados: dados atualizam `App`; o próximo tick redesenha.

## 5. Modelo de Concorrência dos Backends

Cada backend segue o *trait*:

```rust
#[async_trait-like] // ver backend/mod.rs (usamos async fn nativo)
trait Service {
    fn name(&self) -> &'static str;
    async fn run(self, tx: EventTx, actions: ActionRx) -> anyhow::Result<()>;
}
```

- `run` recebe o *sender* de `AppEvent` e um *receiver* de `Action`.
- Faz seu próprio *polling* (intervalo vindo da `Config`) **e** reage a `Action`s sob demanda.
- Erros são convertidos em `AppEvent::ServiceDegraded { name, reason }` — nunca propagam para derrubar a UI.

## 6. Ciclo de Vida

1. `main` carrega `Config`, inicializa `tracing` → arquivo.
2. Entra em raw mode + alt-screen (`crossterm`).
3. Renderiza **Splash** (animação) enquanto os workers fazem o primeiro *fetch*.
4. Transição para a **Aba 1 (Overview)**.
5. Loop principal até `q`/`Ctrl-C`.
6. *Teardown* garantido via `Drop`/guard (restaura terminal mesmo em panic — hook de panic customizado).

## 7. Tratamento de Panic

Um `std::panic::set_hook` restaura o terminal (sai do alt-screen, desliga raw mode) **antes** de imprimir o backtrace, evitando deixar o terminal do usuário corrompido.
