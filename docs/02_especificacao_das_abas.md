# 02 — Especificação das Abas

Navegação global: `Tab`/`Shift-Tab` ou teclas `1..8` alternam abas. `?` abre ajuda. `q` sai.
A *tabbar* fica no topo; a *statusline* (contexto + atalhos da aba ativa + toasts) fica no rodapé.

---

## Splash Screen (pré-abas)

- Sequência: `LOADING...` (pontos animados / barra de progresso) → fade para `Bem-vindo, <usuário>!` → dissolve para a Aba 1.
- Duração mínima configurável (`splash.min_ms`), mas nunca segura a UI além do primeiro fetch dos backends.
- ASCII do besouro aparece atenuado ao fundo e "acende" na transição.

---

## Aba 1 — Overview (estética Neofetch / Fastfetch)

**Layout:** duas colunas. Esquerda = ASCII art do **Besouro**; direita = resumo linha-a-linha.

| Linha | Fonte | Widget |
|-------|-------|--------|
| OS / Distro | `/etc/os-release`, sysinfo | ícone + texto |
| Kernel | `uname` / sysinfo | texto |
| Uptime | sysinfo | texto humano (`2h 14m`) |
| Pacotes | backend updates (contagem instalada) | texto |
| Shell | `$SHELL` | texto |
| CPU | sysinfo | nome + **barra de uso %** |
| Memória RAM | sysinfo | `usada/total` + **barra** |
| Energia/Bateria | UPower | % + estado (⚡/🔋) + **barra** |
| Discos | UDisks2/sysinfo | por montagem: **barra** de uso |
| Brilho | `/sys/class/backlight` | **barra** |
| Volume | `wpctl`/pipewire | **barra** |
| Paleta | tema | blocos de cor (16 cores) |

Barras usam gradiente do tema (verde→amarelo→vermelho conforme %).

---

## Aba 2 — Wi-Fi / Rede (NetworkManager via D-Bus)

- **Lista** de redes: SSID, sinal (barras/dBm), segurança (WPA2/WPA3/Open), `*` na conectada.
- Ações: `Enter` conectar (modal de senha se protegida), `d` desconectar, `f` esquecer, `r` rescan, `t` ligar/desligar rádio.
- Rodapé: IP local, gateway, tráfego ↓/↑ (taxa instantânea).
- Estados: escaneando, conectando, falha de autenticação (toast).

## Aba 3 — Bluetooth (bluez via D-Bus)

- **Lista** de dispositivos: nome, tipo (ícone), pareado/conectado, bateria (se exposta).
- Ações: `s` scan on/off, `p` parear, `Enter` conectar, `d` desconectar, `x` remover, `t` ligar/desligar adaptador.
- Monitora `Battery1` para fones/periféricos.

## Aba 4 — Discos & Armazenamento (UDisks2 via D-Bus)

- Árvore: dispositivo → partições. Colunas: rótulo, FS, tamanho, uso (**barra**), ponto de montagem.
- Ações: `m` montar, `u` desmontar, `e` ejetar com segurança. Destaque para USB removível.
- Confirmação antes de ejetar; toast de sucesso/erro.

## Aba 5 — Energia & Bateria (UPower)

- Saúde da bateria (capacidade atual/design %), ciclos, consumo em **W**, tempo restante (carga/descarga).
- Perfis de energia (`power-profiles-daemon`): performance / balanced / power-saver — seleção com `←/→`.
- Gráfico *sparkline* de consumo recente.

## Aba 6 — Atualizações do Sistema

- Detecção de distro:
  - **Arch:** `checkupdates` (+ AUR via `yay`/`paru` se presentes).
  - **Debian/Ubuntu:** `apt list --upgradable` / `apt-get -s upgrade`.
- Mostra contagem de pendentes + lista (pacote, versão atual → nova).
- Ação `U` dispara o comando de atualização **em um PTY** (aba herda o Terminal Deck) — nunca escondido.

## Aba 7 — Gerenciador de Arquivos (Yazi)

- Lança `yazi` embutido via PTY, ocupando a área de conteúdo.
- Suspensão/retorno do raw mode sem artefatos: ao entrar, cede input ao PTY; ao sair (`q` do Yazi ou atalho de escape), restaura o chrome da TUI e faz *full redraw*.
- Fallback: se `yazi` não instalado → instrução de instalação.

## Aba 8 — Terminal Deck (PTY + VT100)

- Terminal interativo embutido: `portable-pty` gera um shell (`$SHELL`), parser VT mínimo renderiza a grade.
- Foco: quando ativo, teclas vão ao PTY (exceto o *leader* de escape, ex. `Ctrl-a` para voltar ao chrome).
- Base para o "AI Terminal Deck": múltiplas sessões PTY em painéis (roadmap).

---

## Convenções de Teclado (global)

| Tecla | Ação |
|-------|------|
| `1`..`8` / `Tab` / `Shift-Tab` | trocar aba |
| `j`/`k` ou `↑`/`↓` | navegar listas |
| `Enter` | ação primária do item |
| `r` | refresh/rescan da aba |
| `?` | ajuda |
| `q` / `Ctrl-c` | sair |

Atalhos específicos por aba são exibidos na *statusline*.
