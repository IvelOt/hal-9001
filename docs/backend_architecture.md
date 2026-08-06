# Especificação Técnica e Arquitetura do Backend & Integração com Agentes de I.A.

Este documento especifica a arquitetura detalhada do backend, a camada de interação com o sistema operacional (System Interactivity Layer) e a infraestrutura para hospedagem e comunicação com agentes de inteligência artificial (AI Terminal Deck) no projeto `hall-9001`.

---

## 1. Abstração de Subsistemas (System Interactivity Layer)

O `hall-9001` gerencia e monitora os recursos de hardware do sistema utilizando comunicação assíncrona com o barramento de sistema **D-Bus** (via crate `zbus` em Rust) e a execução segura de utilitários externos de linha de comando. Esta camada atua como o intermediário entre a interface de usuário (TUI) e o sistema operacional.

```
+-----------------------------------------------------------+
|                   TUI (Ratatui / Tokio)                   |
+-----------------------------------------------------------+
                              |
     +------------------------+------------------------+
     | (zbus D-Bus)                                    | (std::process::Command)
     v                                                 v
+------------------------------------------------+ +-----------------+
| D-Bus Services                                 | | CLI Utilities  |
| - org.bluez (Bluetooth)                        | | - wpctl         |
| - org.freedesktop.NetworkManager (Wi-Fi)       | | - brightnessctl |
| - org.freedesktop.UDisks2 (Discos/USB)         | +-----------------+
| - org.freedesktop.UPower (Bateria)             |
+------------------------------------------------+
```

### 1.1 Bluetooth (`org.bluez`)

A interação com a pilha de Bluetooth do Linux é feita por meio do serviço `org.bluez` usando a biblioteca `zbus`. O backend gerencia adaptadores e dispositivos.

* **Varredura (Scanning)**:
  * O método `StartDiscovery()` na interface `org.bluez.Adapter1` é invocado para iniciar a busca ativa por dispositivos.
  * O método `StopDiscovery()` encerra a busca para economizar recursos e energia.
  * O backend escuta os sinais do D-Bus `InterfacesAdded` no caminho `/` para capturar novos dispositivos descobertos em tempo real.
* **Pareamento e Conexão**:
  * Para cada dispositivo (representado por caminhos como `/org/bluez/hci0/dev_XX_XX_XX_XX_XX_XX`), lemos propriedades como `Paired` e `Connected` da interface `org.bluez.Device1`.
  * Os métodos `Connect()` e `Disconnect()` são acionados de maneira assíncrona.
  * Para lidar com agentes de pareamento (PINs e confirmação de chave passiva), o backend implementa um objeto D-Bus local sob a interface `org.bluez.Agent1` registrado no `AgentManager1`.

---

### 1.2 Network Wi-Fi (`org.freedesktop.NetworkManager`)

A monitoração e o controle do estado do Wi-Fi são realizados através da integração direta com o NetworkManager.

* **SSID Ativo e Força do Sinal**:
  * O backend lê a propriedade `ActiveConnections` da interface principal `org.freedesktop.NetworkManager`.
  * Cada objeto de conexão ativa expõe a interface `org.freedesktop.NetworkManager.Connection.Active`. Lemos a propriedade `Devices` para obter os dispositivos de rede associados.
  * Para dispositivos sem fio, lemos a propriedade `ActiveAccessPoint` exposta em `org.freedesktop.NetworkManager.Device.Wireless`.
  * O ponto de acesso correspondente expõe a interface `org.freedesktop.NetworkManager.AccessPoint`, de onde lemos as propriedades:
    * `Ssid`: Array de bytes (`Vec<u8>`) contendo o nome da rede (deve ser decodificado como UTF-8).
    * `Strength`: Valor do tipo `u8` (0 a 100) representando a potência percentual do sinal.
* **Ativação e Desativação**:
  * A interface de rede sem fio é ativada ou desativada de forma global alterando a propriedade mutável `WirelessEnabled` na interface `/org/freedesktop/NetworkManager`.

---

### 1.3 Discos e Dispositivos USB (`org.freedesktop.UDisks2`)

A montagem e a desmontagem de mídias de armazenamento externas (USB, cartões de memória, discos secundários) devem ocorrer de forma assíncrona, segura e sem exigir privilégios de superusuário (`sudo`).

* **Mecanismo Polkit**:
  * A comunicação D-Bus é feita com o daemon `UDisks2` (`org.freedesktop.UDisks2`), que delega validação de segurança ao Polkit.
  * O backend faz chamadas diretas sobre objetos sob a interface `org.freedesktop.UDisks2.Filesystem` de partições não montadas (por exemplo, `/org/freedesktop/UDisks2/block_devices/sdb1`).
  * Por padrão, políticas de segurança locais (como `org.freedesktop.udisks2.filesystem-mount`) autorizam usuários em sessões ativas no terminal local a montar mídias em caminhos padrão sob `/run/media/$USER/<LABEL_OU_UUID>`.
* **Montagem e Desmontagem**:
  * **Método `Mount`**: Invoca `mount` passando um dicionário vazio de opções. O UDisks2 retorna a string com o ponto de montagem gerado sob `/run/media/$USER/`.
  * **Método `Unmount`**: Invoca `unmount` para limpar buffers de gravação e desvincular o dispositivo com segurança.

---

### 1.4 Bateria (`org.freedesktop.UPower`)

Para informações do estado de energia de laptops e periféricos, o barramento D-Bus interage com o daemon `UPower`.

* **Propriedades Extraídas**:
  * `State` (`u32`): Indica a condição de energia (1 = Carregando, 2 = Descarregando, 3 = Vazia, 4 = Totalmente Carregada).
  * `Percentage` (`f64`): O nível de bateria atual (0.0 a 100.0).
  * `TimeToEmpty` (`i64`) / `TimeToFull` (`i64`): Segundos restantes estimados.
  * `Capacity` (`f64`): Saúde da bateria.

---

### 1.5 Brilho e Som (`brightnessctl` & `wpctl` / `amixer`)

Os controles de hardware de resposta imediata são gerenciados via inovações seguras de utilitários externos (`brightnessctl`, `wpctl`).

---

## 2. Aba/Janela de Agentes de I.A. Agnósticos (AI Terminal Deck)

O `hall-9001` expõe um painel dedicado da TUI ("AI Terminal Deck") projetado para hospedar sessões interativas com agentes de I.A. externos (ex: `OpenCode`, `Claude Code`, `Aider`, ou scripts customizados CLI). 

```
+------------------------------------------------------------+
|                       TUI principal                        |
|                                                            |
|  +------------------------------------------------------+  |
|  | AI Terminal Deck Panel                               |  |
|  |                                                      |  |
|  |  $ claude / opencode                                 |  |
|  |  > system info is loaded                             |  |
|  |  > How can I help you today?                         |  |
|  |  > _                                                 |  |
|  |                                                      |  |
|  +------------------------------------------------------+  |
|                                                            |
+------------------------------------------------------------+
       ^                                              |
       | Transmissão de Tela (vt100)                  | Entrada (ANSI)
       v                                              v
+-----------------------+                    +---------------+
| MasterPty             | <----------------- | Input Buffer  |
+-----------------------+                    +---------------+
       |
       |  (portable-pty)
       v
+-----------------------+
| SlavePty              |
+-----------------------+
       |
       v
+-----------------------+
| AI Agent CLI Process  |
+-----------------------+
       |
       | JSON-RPC (UNIX Domain Socket)
       v
+-----------------------+
| IPC Server Backend    | <--- Acesso seguro a D-Bus & Controles
+-----------------------+
```

### 2.1 Arquitetura de Subprocesso PTY

1. **Ciclo de Vida do PTY**:
   * O backend usa `portable-pty` para instanciar o pseudo-terminal (`MasterPty` / `SlavePty`).
   * O processo do agente (`opencode`, `claude`, etc.) é iniciado no `SlavePty`.
2. **Interatividade Assíncrona**:
   * Teclas recebidas no Ratatui são repassadas à `MasterPty`.
   * A saída do agente é lida da `MasterPty`, processada por um interpretador ANSI `vt100` e renderizada em um widget TUI do Ratatui.
3. **Redimensionamento Dinâmico (Resize / SIGWINCH)**:
   * Sempre que o painel do `hall-9001` é redimensionado, dispara-se `master_pty.resize()` e o sinal `SIGWINCH` para o agente ajustar sua própria tela.

---

### 2.2 Comunicação Bidirecional via IPC (Sockets UNIX + JSON-RPC)

* **UNIX Domain Socket**: Escuta em `/run/user/$UID/hall-9001.sock` com comunicação padronizada em **JSON-RPC 2.0**.
* **Gatekeeper (Consentimento de Execução)**:
  * Consultas de leitura (bateria, rede, discos) respondem instantaneamente.
  * Ações mutáveis críticas ou comandos de terminal exigem autorização com pop-up modal na TUI do `hall-9001` antes de responder ao agente.

---

## 3. Arquitetura de Módulos Rust

```
src/
├── main.rs                   # Inicializador geral: Tokio runtime, TUI e tarefas em background.
├── config/                   # Configuração e configurações de inicialização.
├── events/                   # Agregador de eventos (Crossterm, D-Bus, IPC).
├── backend/                  # System Interactivity Layer (D-Bus, UDisks2, UPower, BlueZ, CLI).
│   ├── bluetooth.rs
│   ├── network.rs
│   ├── storage.rs
│   ├── power.rs
│   └── controls.rs
└── ai_agent/                 # Terminal Virtual & Servidor IPC do Agente de IA.
    ├── pty_session.rs
    ├── ipc_server.rs
    └── widget.rs
```
