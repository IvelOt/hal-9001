# HAL-9001 — Central TUI de Controle do Sistema & Assistente de Sistema

> *"I'm sorry, Dave. I'm afraid I can't let you use poorly designed dashboards."*

---

## 👁️ A Visão do Projeto

O **HAL-9001** é uma proposta de central de controle no terminal (TUI) para Linux, concebida para unir:
1. **Monitoramento e Gestão de Hardware em Tempo Real:** Interface densa, estética e rápida (inspirada na estética do *btop* e painéis sci-fi);
2. **Controle Ativo de Periféricos e Sistema:** Gerenciamento nativo de Bluetooth (`bluez`), Wi-Fi (`NetworkManager`), Volumes e Discos (`udisks2`), Áudio (`pipewire`/`wpctl`), Bateria e Brilho;
3. **AI Terminal Deck (Assistente de Agentes):** Painel integrado para visualização, monitoramento e interação com agentes de inteligência artificial autônomos e terminais PTY;
4. **Experiência Fluida e Minimalista:** Projetado para funcionar perfeitamente com gerenciadores de janela em mosaico (*i3wm*, *sway*, *bspwm*) e ambientes headless/workstations.

---

## 🧭 Status Atual: Reset Conceitual & Levantamento de Requisitos

O projeto passou por uma limpeza total de código legado para permitir a reestruturação completa da arquitetura, das tecnologias de base e dos requisitos do produto do zero.

---

## 🎯 Próximos Passos:
- [ ] Mapeamento e discussão dos casos de uso reais do dia a dia;
- [ ] Definição da stack tecnológica definitiva (Rust com Ratatui vs Go com Bubbletea vs C/Zig);
- [ ] Especificação da interface visual (wireframes TUI, atalhos, navegação por abas/painéis);
- [ ] Design da camada de integração de baixo nível (D-Bus vs CLI wrappers vs sockets IPC).
