# Relatório de Benchmarks de Performance — Baseline (Pré-Otimização)

Este documento registra a medição exata do consumo de **CPU, Memória RAM e Tamanho do Binário** do **HAL-9001 v0.1.0** antes da aplicação do plano de otimizações de Rust.

---

## 📊 1. Resumo Métrico de Baseline

| Métrica | Valor Baseline (Pré-Otimização) | Meta Pós-Otimização | Ganho Esperado |
|---|---|---|---|
| **Tamanho do Binário (`target/release/hal9001`)** | **5.71 MB** (5,991,544 bytes) | **&lt; 5.0 MB** | Redução de tamanho com LTO Thin e exclusão de código morto |
| **Uso Médio de CPU (Loop de Telemetria & Eventos)** | **6.64%** | **&lt; 0.50%** | -80% com eliminação de subprocessos N+1 no áudio |
| **RAM em Repouso (Overview Idle)** | **13.56 MB** (Pico: 13.80 MB) | **&lt; 15 MB** | Redução com eliminação de alocações transitórias |
| **RAM sob Navegação Ativa (Abas 1–8)** | **14.33 MB** (Pico: 14.70 MB) | **&lt; 20 MB** | `Arc<T>` zero-copy e buffers D-Bus reutilizados |
| **RAM sob Redimensionamento (40x20 a 200x50)** | **15.00 MB** (Pico: 15.34 MB) | **&lt; 22 MB** | Buffer 1D contíguo no PTY e zero-alloc na logo ASCII |
| **Pico Global Máximo de RAM (RSS)** | **15.34 MB** | **&lt; 22 MB** | Estabilidade sem picos de heap |

---

## 🔬 2. Metodologia do Teste de Estresse

O benchmark foi executado de forma automatizada via pseudo-terminal (`pty`) simulando uma sessão interativa real:
1. **Fase 1 — Repouso (5 segundos):** Execução do Overview com polling de telemetria ativo a cada 250ms (CPU, RAM, Discos, Bateria, Sensores de temperatura).
2. **Fase 2 — Navegação Ativa de Abas (10 segundos):** Chaveamento contínuo em alta frequência entre todas as 8 abas (`Visão Geral`, `Rede`, `Bluetooth`, `Discos`, `Áudio`, `Telas`, `Arquivos/PTY`, `Terminal/PTY`).
3. **Fase 3 — Redimensionamento Dinâmico de Terminal (10 segundos):** Redimensionamento contínuo da janela do terminal entre 7 resoluções distintas (desde tela mobile 40x20 até ultrawide 200x50) com recálculo simultâneo de layout espacial 2D e topologia TUI.

---

## 🎯 3. Gargalos Diagnosticados para Otimização

1. **Subprocessos em Loop no Áudio:** Chamadas `wpctl get-volume` a cada 500ms aumentam o uso de CPU.
2. **Deep-Clone de Snapshots:** `AppEvent::Network` e `AppEvent::Storage` clonando vetores inteiros de entidades a cada tick.
3. **Matriz 2D de Células no PTY (`Vec<Vec<PtyCell>>`):** Fragmentação da heap ao alocar 40 vetores por frame.
4. **Alocações de String na Logo ASCII:** ~150 strings criadas por frame no redesenho do Olho do HAL.

*Medição realizada em: 2026-08-26 07:33:45*
