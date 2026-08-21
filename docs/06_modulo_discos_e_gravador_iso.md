# 06 — Módulo de Armazenamento / Discos & Gravador de ISO (Aba 4 — Storage)

> HAL-9001 — Planejamento arquitetural do Módulo 4 (Discos & Armazenamento) e do
> submódulo **ISO Flasher** (gravador de pendrive bootável).
> **Este documento é somente projeto.** Nenhum código de produção é escrito aqui;
> o objetivo é fixar arquitetura, contratos de mensagem, garantias de segurança,
> UX da TUI e a decomposição em tarefas atômicas para o Kanban.

---

## 0. Contexto e Estado Atual

O repositório já possui os *stubs* do Módulo 0:

- `src/backend/storage.rs` — hoje apenas registra `ServiceDegraded` via `pending_stub("storage", "Módulo 4 (UDisks2)", tx)`.
- `src/ui/storage.rs` — hoje renderiza `draw_pending(...)` com o placeholder da aba.

O fluxo unidirecional existente (ver `docs/01_arquitetura_e_stack.md`) é:

```
backend workers ──AppEvent(mpsc)──▶ App (estado) ──Action(broadcast)──▶ backend workers
                                        │
                                   ui::draw(&App, Frame)  (função pura, tick-driven)
```

Regras herdadas que este módulo **deve** respeitar:

1. **UI nunca bloqueia** — todo I/O de D-Bus, `dd`/gravação de bloco, `mkfs`, `sync`, cálculo de SHA256 roda em *tasks* Tokio; a thread de render só lê `App`.
2. **Estado único** — `App` é a fonte da verdade; a UI de storage é `fn(&App, &Palette, &mut Frame, Rect)`.
3. **Sem `Arc<Mutex<...>>` entre UI e backend** — comunicação exclusivamente por canais.
4. **Degradação graciosa** — sem UDisks2 no host, a aba entra em modo "indisponível", nunca derruba o app.
5. **i18n** — todas as strings visíveis passam por `Language::messages()` (pt-BR / en-US / es-ES).
6. **Sem warnings** — `cargo clippy -- -D warnings` limpo (Definição de Pronto do projeto).

---

## 1. Visão Geral da Arquitetura do Módulo

O Módulo 4 tem **duas responsabilidades distintas** que compartilham o mesmo
backend e a mesma aba:

| Subsistema | Descrição | Superfície de risco |
|------------|-----------|---------------------|
| **A. Disk Manager** | Enumerar/monitorar block devices, montar, desmontar, ejetar, LUKS unlock/lock, formatar e particionar. | Alta (destrói dados) |
| **B. ISO Flasher** | Selecionar `.iso`, checar SHA256, gravar imagem de bloco com progresso, verificar e `sync`. | Muito alta (sobrescreve disco inteiro) |

Ambos são orquestrados por uma única task de backend `storage::run`, que:

- Mantém uma **conexão `zbus` com `org.freedesktop.UDisks2`** para monitoramento reativo e ações de montagem/desmontagem/ejeção/LUKS/format.
- Gera **sub-tasks efêmeras** para operações longas (gravação de ISO, formatação, cálculo de checksum), cada uma emitindo eventos de progresso.
- Consome `Action`s filtrando apenas as que lhe interessam.

```
                          ┌──────────────────────────────────────────────┐
                          │                storage::run (task raiz)       │
                          │                                              │
  UDisks2 signals ───────▶│  zbus Proxy + ObjectManager (InterfacesAdded │
  (D-Bus reativo)         │   / InterfacesRemoved / PropertiesChanged)   │──┐
                          │                                              │  │
  Action (broadcast) ────▶│  dispatcher → método D-Bus ou spawn sub-task │  │ AppEvent (mpsc)
                          │                                              │  ▼
                          │  ┌─ spawn: flash_task  (dd assíncrono)       │  App.storage
                          │  ├─ spawn: verify_task (SHA256/readback)     │  (StorageState)
                          │  ├─ spawn: format_task (mkfs/partição)       │
                          │  └─ spawn: checksum_task (SHA256 do .iso)    │
                          └──────────────────────────────────────────────┘
```

### 1.1 Por que UDisks2 (e não `mount`/`umount` direto)

- **Não-root:** UDisks2 monta em `/run/media/$USER/<label>` via Polkit sem exigir `sudo`; o app permanece um binário de usuário.
- **Reatividade:** o `ObjectManager` do UDisks2 emite sinais D-Bus de hotplug — nada de *polling* de `/proc/mounts` ou `udevadm monitor` em subprocesso.
- **Cobertura:** montar, desmontar, ejetar, *power-off*, LUKS unlock/lock, `Format`, criar tabela de partição e partições — tudo via métodos D-Bus tipados.
- **Consistência:** UDisks2 é o mesmo daemon que GNOME Disks/KDE usam; estado sempre coerente com o resto do desktop.

`sysinfo` permanece como **fallback de leitura** (uso por montagem) quando UDisks2 estiver ausente, exatamente como o Overview já faz.

---

## 2. Detecção e Gerenciamento de Discos (UDisks2 / D-Bus & Sysfs)

### 2.1 Objetos e interfaces relevantes do UDisks2

| Interface D-Bus | Uso no módulo |
|-----------------|---------------|
| `org.freedesktop.UDisks2.Manager` | `GetBlockDevices`, resolução inicial. |
| `org.freedesktop.DBus.ObjectManager` | `GetManagedObjects` (snapshot inicial) + sinais `InterfacesAdded`/`InterfacesRemoved` (hotplug). |
| `org.freedesktop.UDisks2.Block` | Propriedades: `Device`, `Drive`, `IdType` (FS), `IdLabel`, `IdUUID`, `Size`, `ReadOnly`, `CryptoBackingDevice`, `HintSystem`. Métodos: `Format`. |
| `org.freedesktop.UDisks2.Filesystem` | `MountPoints`, métodos `Mount`/`Unmount`. |
| `org.freedesktop.UDisks2.Partition` | `Number`, `Type`, `Offset`, `Size`, `IsContained`. |
| `org.freedesktop.UDisks2.PartitionTable` | `Type` (gpt/dos), método `CreatePartition`. |
| `org.freedesktop.UDisks2.Drive` | `Removable`, `Ejectable`, `MediaRemovable`, `ConnectionBus` (usb/sata), `Model`, `Vendor`, `Serial`, método `Eject`, `PowerOff`. |
| `org.freedesktop.UDisks2.Encrypted` | LUKS: métodos `Unlock`/`Lock`, propriedade `CleartextDevice`. |
| `org.freedesktop.UDisks2.Block` (`OpenDevice`) | Abre um FD do dispositivo via Polkit para a gravação de bloco (ISO Flasher) sem root. |

### 2.2 Monitoramento reativo assíncrono (hotplug)

Fluxo de inicialização e monitoramento na `storage::run`:

1. Conectar ao **system bus** via `zbus::Connection::system().await`.
2. `GetManagedObjects` → construir o **snapshot inicial** da árvore (drives → blocks → partitions → filesystems).
3. Assinar os *streams* de sinais:
   - `InterfacesAdded` → novo pendrive/partição apareceu → adicionar ao modelo, emitir `Toast` "dispositivo conectado", reenviar snapshot.
   - `InterfacesRemoved` → dispositivo removido → remover do modelo, invalidar seleção se necessário.
   - `PropertiesChanged` em `Filesystem.MountPoints` / `Encrypted` → refletir montagem/unlock em tempo real.
4. `tokio::select!` unificando: os *streams* de sinais + o `broadcast::Receiver<Action>` + um `tokio::time::interval(storage_ms)` de *refresh* leve (recalcular uso de FS montados, que o UDisks2 não notifica por si só).

> **Nota de concorrência:** o snapshot é reconstruído no backend e enviado inteiro
> como um `AppEvent::Storage(Box<StorageSnapshot>)` (boxed, como já se faz com
> `SystemSnapshot`). Isso mantém `App` imutável no render e evita estado
> compartilhado. Diffs finos são desnecessários — a árvore de discos é pequena.

### 2.3 Enriquecimento via Sysfs (complemento ao UDisks2)

Alguns dados são mais baratos/completos em `/sys` e `/proc`:

- `/sys/block/<dev>/removable`, `/sys/block/<dev>/ro` — confirmação de removível/somente-leitura.
- `/sys/block/<dev>/queue/rotational` — HDD vs SSD (ícone/rótulo).
- `/proc/mounts` — reconciliação de pontos de montagem (fallback quando UDisks2 ausente).
- `statvfs()` (via `sysinfo` ou syscall) — uso real (usado/total) por FS montado, para a **barra de uso**.

### 2.4 Montagem / Desmontagem não-root

- **Montar:** `Filesystem.Mount({})` → UDisks2 monta em `/run/media/$USER/<label>` e retorna o caminho; emitir `Toast::success` com o mountpoint.
- **Desmontar:** `Filesystem.Unmount({})`. Tratar `org.freedesktop.UDisks2.Error.DeviceBusy` → toast de erro claro ("dispositivo em uso; feche programas que o utilizam").
- **Polkit:** se a ação exigir autenticação e não houver agente Polkit ativo (sessão headless), UDisks2 retorna `NotAuthorized` → degradar com toast explicativo, nunca travar.

### 2.5 Ejeção segura de unidades USB

Sequência para o atalho `e` (ejetar):

1. Desmontar **todos** os filesystems montados do *drive* (iterar partições).
2. `Drive.Eject({})` (mídia ejetável) **e/ou** `Drive.PowerOff({})` (corta energia do controlador USB — "safe to remove").
3. Toast: "Seguro remover <modelo>".

### 2.6 Unidades criptografadas (LUKS)

- **Detecção:** bloco com `IdType == "crypto_LUKS"` e interface `Encrypted` presente.
- **Unlock (`Enter` sobre bloco LUKS bloqueado):** modal de senha (reaproveita o padrão do modal de senha Wi-Fi do Módulo 2) → `Encrypted.Unlock(passphrase, {})` → aparece um `CleartextDevice` que é então montável normalmente.
- **Lock (`u`/`l` sobre um LUKS aberto):** desmontar o cleartext → `Encrypted.Lock({})`.
- **Segurança da senha:** a passphrase vive apenas na `Action` até ser passada ao método D-Bus; **zeroizar** o buffer após uso (evitar `Debug`/log da string; usar um wrapper `Secret(String)` com `Debug` redigido).

---

## 3. Formatação e Particionamento

> **Este é o eixo de maior risco de perda de dados.** Toda a seção 5 (Segurança)
> se aplica integralmente aqui.

### 3.1 Criação de sistemas de arquivos

Via `Block.Format(type, options)` do UDisks2, que encapsula os `mkfs.*`:

| FS alvo | `type` UDisks2 | Ferramenta subjacente | Observação |
|---------|----------------|-----------------------|------------|
| FAT32 | `vfat` | `mkfs.vfat` | Máx. 4 GB por arquivo; ideal p/ boot UEFI. |
| exFAT | `exfat` | `mkfs.exfat` | Grandes arquivos, cross-platform. |
| ext4 | `ext4` | `mkfs.ext4` | Linux nativo; opção de rótulo. |
| NTFS | `ntfs` | `mkfs.ntfs` (`--fast`) | Requer `ntfs-3g`. |
| btrfs | `btrfs` | `mkfs.btrfs` | Requer `btrfs-progs`. |

Opções passadas ao `Format`: `label`, `take-ownership` (para ext4/btrfs), `erase` (opcional; ver 3.3), `no-block` (para receber progresso via task assíncrona).

**Degradação:** antes de oferecer um FS no modal, checar disponibilidade da ferramenta (`which mkfs.<x>` ou capacidades reportadas pelo UDisks2). FS indisponível aparece esmaecido com dica "instale `<pkg>`".

### 3.2 Tabela de partição (GPT / MBR)

- **Wipe + nova tabela:** `Block.Format("gpt" | "dos", {})` sobre o *drive* inteiro cria uma tabela de partição vazia.
- **Criar partição:** `PartitionTable.CreatePartition(offset, size, type, name, options)`.
- **Fluxo típico "limpar pendrive":** `Format(gpt)` → `CreatePartition(0, max, ...)` → `Format(vfat)` na partição criada.

### 3.3 Limpeza (wipe)

- **Rápido:** apenas nova tabela de partição (dados recuperáveis).
- **Seguro (opcional):** opção `erase: "zero"` no `Format` (zera o dispositivo) — lento, exibido com aviso de tempo. **Nunca** oferecer wipe seguro como padrão.

### 3.4 Garantias absolutas de segurança (trava de software)

Ver seção 5 — a validação `is_system_disk()` é chamada **antes** de qualquer
`Format`/`CreatePartition`/flash, e discos de sistema são **filtrados da própria
lista de alvos selecionáveis**, não apenas bloqueados na confirmação.

---

## 4. Gravador de Pendrive Bootável a partir de ISO (ISO Flasher)

### 4.1 Fluxo completo (máquina de estados)

```
Idle
  │  (usuário aciona 'g'/'b')
  ▼
SelectingIso ──── navega/seleciona .iso ────▶ IsoSelected
  │                                              │ (opcional: 'c' calcular checksum)
  │                                              ▼
  │                                        Checksumming ──▶ ChecksumReady (SHA256 exibido)
  │                                              │
  └──────────── seleciona disco alvo ───────────┤
                                                 ▼
                                     ConfirmStage1  (revisar: ISO, alvo, tamanho)
                                                 │  (digitar/confirmar)
                                                 ▼
                                     ConfirmStage2  (confirmação final destrutiva)
                                                 │
                                                 ▼
                                        Flashing (progresso contínuo)
                                                 │
                                                 ▼
                                        Syncing (flush + sync garantido)
                                                 │
                                                 ▼
                                        Verifying (readback SHA256 vs. ISO)
                                                 │
                                        ┌────────┴────────┐
                                        ▼                 ▼
                                    Success             Failed(reason)
```

### 4.2 Seleção do arquivo `.iso`

- Diálogo de seleção embutido (navegador de arquivos mínimo na aba, filtrando `*.iso`/`*.img`), ou colar caminho absoluto.
- Validar: existe, é arquivo regular, legível, tamanho > 0, **≤ tamanho do disco alvo**.

### 4.3 Checksum SHA256

- Task assíncrona lê a ISO em blocos (ex. 4 MiB), alimentando um hasher (`sha2` crate — **nova dependência**), emitindo `AppEvent::StorageChecksumProgress { pct }`.
- Resultado exibido em hex; se o usuário tiver um SHA256 de referência, permitir colar para comparação (match/mismatch visual). Opcional na v1, mas o *hook* de UI é planejado.

### 4.4 Gravação de imagem de bloco

Duas estratégias, escolhidas por disponibilidade:

1. **Preferencial (não-root):** `Block.OpenDevice("w", { flags: O_SYNC | O_EXCL })` do UDisks2 → retorna um FD via Polkit → gravar a ISO nesse FD em blocos, com `O_EXCL` garantindo que o dispositivo não esteja montado. Mantém o app sem privilégios.
2. **Fallback:** invocar `dd`/gravação direta **somente** se o usuário já for root ou via `pkexec` explícito — evitado por padrão.

Detalhes da task de gravação (`flash_task`):

- Buffer de I/O de 1–4 MiB; `write` + contagem de bytes.
- **Relatório de progresso contínuo** emitido a cada ~200 ms (não a cada bloco, para não inundar o canal):
  - `%` concluída = `bytes_escritos / tamanho_iso`.
  - **Taxa em MB/s** = média móvel curta (janela ~1 s).
  - **ETA em segundos** = `(tamanho - escrito) / taxa_atual`.
- Respeitar `O_EXCL`/dispositivo desmontado; abortar se aparecer montagem concorrente.
- **Cancelável:** a task escuta um `tokio::sync::watch`/`CancellationToken`; `Esc` durante o flash sinaliza cancelamento (com aviso de que o pendrive ficará inconsistente).

### 4.5 Verificação pós-gravação e `sync` garantido

1. **Syncing:** `fsync(fd)` + `libc::sync()` global antes de considerar concluído — o progresso "100%" **não** é sucesso até o sync retornar (evita a armadilha clássica do `dd` com cache).
2. **Verifying:** reabrir o dispositivo em leitura, ler os primeiros N bytes correspondentes ao tamanho da ISO e comparar SHA256 (ou comparação bloco-a-bloco em streaming). Emitir progresso análogo.
3. Divergência → `Failed { reason: "verificação falhou no offset X" }`.

### 4.6 Modal de confirmação em duas etapas

- **Etapa 1 — Revisão:** card com ISO (nome + SHA256 curto), disco alvo (modelo, `/dev/sdX`, tamanho, barramento USB), aviso "TODOS OS DADOS SERÃO APAGADOS". Ação: destacar botão *Prosseguir* (não default; requer navegação/`Enter` consciente).
- **Etapa 2 — Confirmação destrutiva:** exigir uma ação deliberada difícil de acionar por engano — ex. **digitar o modelo do dispositivo** ou segurar/`Enter` duplo em um botão "GRAVAR" em vermelho. Só então dispara `Action::StorageFlashStart`.
- Em ambas as etapas `Esc` cancela e volta ao estado anterior.

---

## 5. Segurança — Garantias Absolutas (Trava de Software)

> Esta é a seção mais crítica do módulo. **Nenhuma operação destrutiva
> (`Format`, `CreatePartition`, flash, wipe) pode tocar um disco de sistema.**

### 5.1 Definição de "disco de sistema"

Um *drive* é **protegido** se **qualquer** de suas partições (ou ele próprio)
satisfizer **qualquer** critério:

1. Contém um FS **montado** em `/`, `/boot`, `/boot/efi`, `/home`, `/var`, `/usr` ou qualquer ancestral de `$HOME` — resolvido via `Filesystem.MountPoints` do UDisks2 e reconciliado com `/proc/mounts`.
2. `Block.HintSystem == true` (UDisks2 já marca dispositivos de sistema).
3. Contém a **partição de swap** ativa (`/proc/swaps`).
4. É o *drive* que hospeda o **root do processo em execução** (resolver `stat` do `/` → `st_dev` → *drive*).
5. `Drive.Removable == false` **e** `ConnectionBus != "usb"` (heurística conservadora: discos internos fixos nunca são alvo por padrão).

### 5.2 Camadas de defesa (defense in depth)

| Camada | Mecanismo |
|--------|-----------|
| **1. Filtragem de UI** | Discos protegidos aparecem na árvore **marcados com 🔒 e não-selecionáveis** para ações destrutivas. Não basta bloquear no fim — o alvo nem entra na lista de flash/format. |
| **2. Guarda no dispatch** | `App.dispatch` recusa `Format`/`Flash`/`CreatePartition` cujo alvo seja protegido, emitindo `Toast::error` "operação bloqueada: disco de sistema". |
| **3. Guarda no backend** | `storage::run` **revalida** `is_system_disk(target)` imediatamente antes de chamar o método D-Bus (o snapshot pode ter mudado entre a UI e a execução — TOCTOU). Falha → aborta + toast. |
| **4. Confirmação em 2 etapas** | (Seção 4.6) — só para discos já validados como não-sistema. |
| **5. `O_EXCL` / desmontagem obrigatória** | Gravação exige dispositivo não montado; o kernel recusa se alguém montar no meio. |

### 5.3 Regras adicionais

- **Nunca** aceitar caminho de dispositivo digitado à mão que burle a lista validada (sem "modo avançado" que aceite `/dev/X` arbitrário na v1).
- **Re-resolver por UUID/serial**, não só por `/dev/sdX` — nomes de nó podem trocar entre replug (TOCTOU de nomeação).
- **Read-only e ocupado:** respeitar `Block.ReadOnly` e `DeviceBusy`.
- **Auditoria:** toda ação destrutiva registra em `tracing` (nível `warn`) o alvo (modelo+serial), a operação e o resultado — para o `hal9001.log`.
- **Senhas LUKS:** wrapper `Secret` com `Debug` redigido; nunca logar; zeroizar após uso.

---

## 6. Interface TUI (Ratatui — Aba 4)

### 6.1 Layout responsivo em 2 colunas

```
┌─ Discos & Armazenamento ─────────────────────────────────────────────────────┐
│ ┌── Árvore (≈40%) ─────────────┐ ┌── Detalhes & Ações (≈60%) ───────────────┐ │
│ │ ▾ ⬒ Samsung SSD 980  931G 🔒 │ │  Dispositivo: Kingston DataTraveler       │ │
│ │   ├ / ext4      ████████░ 82%│ │  Nó:          /dev/sdb                     │ │
│ │   └ /boot vfat  ██░░░░░░ 12% │ │  Barramento:  USB 3.0   Removível: sim     │ │
│ │ ▾ ⬓ Kingston DT  29G  [USB]  │ │  Tamanho:     28.9 GiB                     │ │
│ │   └ KINGSTON exfat ███░ 34%  │ │  Serial:      408D5C...                    │ │
│ │ ▸ 🔒 crypto_LUKS (bloqueado) │ │ ─────────────────────────────────────────  │ │
│ │                              │ │  Partição KINGSTON (exfat)                │ │
│ │                              │ │  Montada em: /run/media/ivelot/KINGSTON   │ │
│ │                              │ │  Uso: 9.8G / 28.9G  ███░░░░░ 34%           │ │
│ │                              │ │ ─────────────────────────────────────────  │ │
│ │                              │ │  Ações: [m]ontar [u]desmontar [e]jetar    │ │
│ │                              │ │         [f]ormatar  [g]ravar ISO          │ │
│ └──────────────────────────────┘ └───────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────────────────────────┤
│ [m] mount [u] umount [e] eject [f] format [g] gravar ISO [r] refresh  ● toast  │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Responsividade:** `Layout::horizontal([Constraint::Percentage(40), Percentage(60)])`. Abaixo de uma largura mínima (ex. < 80 col), colapsar para **coluna única** com o painel de detalhes sob a árvore (alternável por `Tab`/`→`).
- **Ícones nerd-font** com fallback ASCII (respeitando `config.ui.icons`): SSD/HDD/USB/LUKS/lock.
- **Barras de uso** reutilizam o helper de `ui/widgets.rs` (gradiente verde→amarelo→vermelho já existente).
- **Destaque de removível/USB** e **🔒 para discos de sistema** (não selecionáveis).

### 6.2 Atalhos de teclado (statusline da aba)

| Tecla | Ação | Contexto |
|-------|------|----------|
| `j`/`k` `↑`/`↓` | navegar árvore | sempre |
| `→`/`←` `Space` | expandir/colapsar drive | sobre drive |
| `m` | montar | partição desmontada |
| `u` | desmontar | partição montada (ou LUKS lock) |
| `e` | ejetar com segurança | drive USB |
| `Enter` | ação primária: montar, ou **unlock** se LUKS | partição |
| `f` | formatar (abre modal de FS) | alvo não-sistema |
| `g` / `b` | gravar ISO (abre flasher) | drive não-sistema |
| `c` | calcular SHA256 da ISO | no fluxo do flasher |
| `r` | refresh manual | sempre |
| `Esc` | cancelar modal / abortar operação em curso | modais |

> `c`/`C` global já é o modal de configurações — no contexto do flasher, `c`
> opera como "checksum" apenas quando o modal do flasher está aberto (captura
> local de teclas, como o modal de config já faz em `dispatch`).

### 6.3 Modais interativos

1. **Modal de senha (LUKS):** reutiliza padrão do modal de senha Wi-Fi.
2. **Modal de formatação:** seletor de FS (FAT32/exFAT/ext4/NTFS/btrfs), campo de rótulo, checkbox "nova tabela GPT", checkbox "wipe seguro (lento)"; confirmação embutida.
3. **Modal do ISO Flasher:** wizard (seleção ISO → checksum → alvo → confirmação 2 etapas).
4. **Modal de progresso:** barra + `%` + `MB/s` + `ETA`, usado por flash, verify, format e checksum. `Esc` cancela.
5. **Modal de confirmação 2 etapas:** (seção 4.6).

Todos os modais seguem o padrão de captura de input já existente em
`App.dispatch` (bloco `if self.show_config { ... return; }`) — cada modal ativo
intercepta a navegação e devolve o controle ao fechar.

---

## 7. Contratos de Mensagem (extensões de `events/mod.rs`)

### 7.1 Novos `AppEvent` (backend → app)

```rust
// Snapshot completo da árvore de discos (boxed, como SystemSnapshot).
Storage(Box<StorageSnapshot>),

// Progresso de operações longas (flash / verify / format / checksum).
StorageProgress {
    op: StorageOp,          // Flash | Verify | Format | Checksum
    target: DeviceId,       // por UUID/serial, não /dev/sdX
    pct: f32,               // 0.0..=1.0
    rate_mbs: f32,          // MB/s (0 quando N/A)
    eta_secs: Option<u64>,  // None quando indeterminado
},

// Conclusão de operação longa.
StorageOpDone {
    op: StorageOp,
    target: DeviceId,
    result: Result<String, String>, // Ok(mensagem) | Err(motivo)
},

// Resultado de checksum SHA256 da ISO selecionada.
StorageChecksum { path: PathBuf, sha256: String },
```

`ServiceDegraded { name: "storage", .. }` continua sendo usado para "UDisks2
ausente".

### 7.2 Novos `Action` (input/app → backend)

> `Action` precisa ser `Clone` (canal `broadcast`). Payloads maiores (senha,
> caminho de ISO) são aceitáveis, mas a **senha usa `Secret`** com `Debug` redigido.

```rust
StorageMount(DeviceId),
StorageUnmount(DeviceId),
StorageEject(DeviceId),
StorageLuksUnlock { target: DeviceId, passphrase: Secret },
StorageLuksLock(DeviceId),
StorageFormat { target: DeviceId, fs: FsKind, label: String, new_gpt: bool, secure_wipe: bool },
StorageCreatePartition { drive: DeviceId, size: PartSize, fs: FsKind },
StorageChecksumIso(PathBuf),
StorageFlashStart { iso: PathBuf, target: DeviceId },
StorageCancelOp(StorageOp),
StorageRefresh,
```

### 7.3 Modelos de dados (novo `backend/storage.rs`)

```rust
pub struct StorageSnapshot {
    pub udisks_available: bool,
    pub drives: Vec<DriveInfo>,
}

pub struct DriveInfo {
    pub id: DeviceId,          // serial/UUID estável
    pub dev_node: String,      // /dev/sdX (exibição)
    pub model: String,
    pub vendor: String,
    pub size: u64,
    pub removable: bool,
    pub connection_bus: String, // "usb" | "sata" | ...
    pub rotational: bool,       // HDD vs SSD
    pub is_system: bool,        // ← trava de segurança (seção 5)
    pub partitions: Vec<PartitionInfo>,
}

pub struct PartitionInfo {
    pub id: DeviceId,
    pub label: String,
    pub fs: Option<String>,     // ext4/vfat/exfat/ntfs/btrfs/crypto_LUKS
    pub size: u64,
    pub used: Option<u64>,      // via statvfs quando montada
    pub mount_points: Vec<String>,
    pub luks: Option<LuksState>, // Locked | Unlocked{cleartext}
    pub is_system: bool,
}
```

### 7.4 Extensão de `App` (state em `app.rs`)

```rust
// dentro de struct App:
pub storage: Option<StorageSnapshot>,
pub storage_ui: StorageUiState,   // seleção na árvore, expandido/colapsado

// máquina de estados do flasher/format/modais:
pub storage_modal: StorageModal,  // None | Password | Format | Flasher{stage} | Progress{op,pct,..}
```

`handle_event` ganha os braços para `AppEvent::Storage*`; `dispatch` ganha um
bloco de captura de modal de storage (espelhando o de `show_config`).

---

## 8. Dependências e Ferramentas de Sistema

### 8.1 Novas crates (Cargo.toml)

| Crate | Papel | Justificativa |
|-------|-------|---------------|
| `sha2` | SHA256 da ISO e verificação | Puro-Rust, sem processo externo. |
| `zeroize` (opcional) | Zerar senhas LUKS na memória | Segurança da passphrase. |
| `libc` | `O_EXCL`, `O_SYNC`, `fsync`, `sync`, `statvfs`, `stat` `st_dev` | Gravação de bloco correta e detecção de disco de root. |

`zbus`, `tokio`, `sysinfo` já presentes. `nix` poderia substituir `libc` se
preferido (decisão de implementação).

### 8.2 Dependências de runtime do host

- **UDisks2** (`udisks2` daemon) — obrigatório para o modo pleno.
- **Agente Polkit** ativo (para ações autenticadas em sessão de desktop).
- Ferramentas de FS: `dosfstools`, `exfatprogs`, `e2fsprogs`, `ntfs-3g`, `btrfs-progs` — detectadas em runtime; ausência esmaece o FS no modal.

`bin/setup.sh` deve passar a diagnosticar essas dependências.

---

## 9. Estratégia de Testes

| Nível | Alvo |
|-------|------|
| **Unit** | `is_system_disk()` (todos os 5 critérios da seção 5.1) com fixtures de snapshot; cálculo de `%`/`MB/s`/`ETA`; parsing de `/proc/mounts` e `/proc/swaps`; formatação de tamanhos (GiB). |
| **Unit** | Máquina de estados do flasher (transições válidas/inválidas; `Esc` em cada estado). |
| **Integração** | Backend com **mock de D-Bus** (trait `UDisksClient` injetável) publicando `AppEvent`s determinísticos: hotplug add/remove, mount/unmount, format, flash progress. |
| **Integração** | Guarda anti-disco-de-sistema: garantir que `Format`/`Flash` sobre alvo `is_system=true` **nunca** chega ao cliente D-Bus (falha antes). |
| **Property/fuzz** | `is_system_disk` nunca retorna `false` quando `/` está no drive (invariante de segurança). |
| **Smoke** | `cargo run` headless (`TERM=dumb`) com UDisks2 ausente → aba degrada, app não cai. |
| **Manual/E2E** | Pendrive real: montar/desmontar/ejetar; formatar; gravar uma ISO pequena e verificar boot. (fora do CI). |

**Invariante de teste inegociável:** existe um teste que falha o build se um
disco com `/` montado puder ser selecionado como alvo de qualquer operação
destrutiva.

---

## 10. Plano de Implementação Modular & Decomposição em Tasks (Kanban)

Ordem sugerida; cada task é atômica, compilável e testável. Encaixe no Módulo 4
do `docs/03_plano_de_execucao_modular.md`, com o ISO Flasher como Módulo 4b.

### Épico A — Fundação D-Bus & Modelos (read-only, sem risco)

- **A1.** Definir modelos `StorageSnapshot`/`DriveInfo`/`PartitionInfo`/`DeviceId`/`FsKind` em `backend/storage.rs`. *(unit: formatação de tamanhos)*
- **A2.** Conexão `zbus` ao system bus + `GetManagedObjects` → construir snapshot inicial. *(integração: mock ObjectManager)*
- **A3.** Adicionar `AppEvent::Storage(Box<..>)` e braço em `App.handle_event`. Emitir snapshot; storage_ms interval.
- **A4.** Enriquecimento sysfs/statvfs: `removable`, `rotational`, uso por FS. *(unit: parsing)*
- **A5.** `ServiceDegraded` quando UDisks2 ausente (substitui o `pending_stub`).

### Épico B — UI da Aba (read-only)

- **B1.** Substituir `draw_pending` em `ui/storage.rs` pelo layout 2 colunas (árvore + detalhes). *(render puro)*
- **B2.** Árvore drive→partição com ícones, barras de uso, destaque USB, 🔒 sistema.
- **B3.** Navegação (`j/k`, expand/colapse) + `StorageUiState` em `app.rs`.
- **B4.** Painel de detalhes + statusline de atalhos + i18n de todas as strings.
- **B5.** Colapso responsivo para coluna única em telas estreitas.

### Épico C — Monitoramento Reativo (hotplug)

- **C1.** Assinar `InterfacesAdded`/`InterfacesRemoved` → atualizar snapshot + toast. *(integração: mock hotplug)*
- **C2.** Assinar `PropertiesChanged` (MountPoints/Encrypted) → refletir estado.
- **C3.** Unificar sinais + `Action` + interval em `tokio::select!`.

### Épico D — Ações não-destrutivas

- **D1.** `Action::StorageMount`/`Unmount` + `Filesystem.Mount/Unmount` + tratamento de `DeviceBusy`. *(integração)*
- **D2.** `Action::StorageEject` (desmonta tudo → `Eject`/`PowerOff`). *(integração)*
- **D3.** `Action::StorageRefresh` + tecla `r`.

### Épico E — Trava de Segurança (pré-requisito de F e G) ⚠️

- **E1.** `is_system_disk()` com os 5 critérios (seção 5.1) + `st_dev` do root. *(unit exaustivo)*
- **E2.** Marcar `is_system` no snapshot; filtrar da seleção destrutiva na UI (camada 1).
- **E3.** Guardas em `dispatch` (camada 2) e revalidação em `storage::run` (camada 3, TOCTOU). *(integração: alvo sistema nunca chega ao cliente)*
- **E4.** Logging de auditoria (`tracing::warn`) de toda ação destrutiva.

### Épico F — LUKS

- **F1.** Detecção de `crypto_LUKS` + estado Locked/Unlocked no modelo.
- **F2.** Modal de senha (reuso Wi-Fi) + `Secret` + zeroize.
- **F3.** `Unlock`/`Lock` via `Encrypted` + montagem do cleartext. *(integração)*

### Épico G — Formatação & Particionamento (depende de E)

- **G1.** Modal de formatação (seletor FS, rótulo, GPT, wipe) + detecção de ferramentas.
- **G2.** `Action::StorageFormat` → `Block.Format` assíncrono com progresso. *(integração)*
- **G3.** `CreatePartition` + fluxo "limpar pendrive" (gpt → part → format).
- **G4.** `AppEvent::StorageProgress`/`OpDone` + modal de progresso genérico.

### Épico H — ISO Flasher (Módulo 4b, depende de E) 🔥

- **H1.** Adicionar `sha2`; `checksum_task` streaming + `AppEvent::StorageChecksum`. *(unit: hash de fixture)*
- **H2.** Seleção de `.iso` (navegador mínimo/caminho) + validações de tamanho.
- **H3.** `flash_task`: `Block.OpenDevice` + gravação em blocos + progresso (%, MB/s, ETA) + cancelamento. *(integração: mock FD)*
- **H4.** Syncing garantido (`fsync`+`sync`) — 100% ≠ sucesso até sync.
- **H5.** Verifying (readback + SHA256) + resultado. *(integração)*
- **H6.** Wizard do flasher + confirmação em 2 etapas (seção 4.6). *(unit: máquina de estados)*

### Épico I — Fechamento

- **I1.** `bin/setup.sh`: diagnosticar UDisks2/Polkit/ferramentas de FS.
- **I2.** Suíte de testes de invariante de segurança (seção 9) no CI.
- **I3.** Atualizar `docs/02_especificacao_das_abas.md` e `docs/03_plano_de_execucao_modular.md` (Módulo 4 + 4b) refletindo o entregue.
- **I4.** Clippy limpo + smoke headless + revisão de i18n.

### Grafo de dependências

```
A ──▶ B ──▶ C ──▶ D
       │
       └──▶ E ⚠️ ──▶ F
                 ├──▶ G
                 └──▶ H (4b) 🔥
D,F,G,H ──▶ I
```

> **E é bloqueante para F/G/H.** Nenhuma operação destrutiva entra no backlog
> "pronto para dev" antes da trava de segurança estar implementada e testada.

---

## 11. Riscos & Mitigações

| Risco | Mitigação |
|-------|-----------|
| TOCTOU (nó `/dev/sdX` troca entre UI e execução) | Identidade por UUID/serial; revalidação no backend (E3). |
| Polkit ausente (headless) | Degradar com toast; nunca travar. |
| `dd` "concluído" mas cache não sincronizado | Estado `Syncing` obrigatório antes de sucesso (H4). |
| Usuário grava no disco errado | Trava de sistema + confirmação 2 etapas + verify (E, H4-H6). |
| Senha LUKS vazando em log/`Debug` | `Secret` redigido + zeroize (F2). |
| UDisks2 versão antiga sem algum método | Detectar capacidades; esmaecer ação indisponível. |
| Flooding do canal de progresso | Throttle a ~200 ms por evento (H3). |

---

## 12. Definição de Pronto (Módulo 4 + 4b)

1. `cargo clippy -- -D warnings` limpo.
2. Aba degrada graciosamente sem UDisks2.
3. Atalhos aparecem na statusline; todas as strings em i18n (pt-BR/en-US/es-ES).
4. Nenhum `.await` de I/O na thread de render.
5. **Trava de segurança testada:** teste de invariante impede seleção de disco de sistema.
6. Montar/desmontar/ejetar um pendrive real com confirmação (aceite do Módulo 4).
7. Gravar uma ISO real e bootar dela; verify passa (aceite do Módulo 4b).
