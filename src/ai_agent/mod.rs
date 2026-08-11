//! AI Terminal Deck — terminal virtual para agentes de I.A. e servidor IPC JSON-RPC.
//!
//! Hospeda sessões interativas (OpenCode, Claude, bash) via `portable-pty` +
//! parser ANSI `vt100` ([`pty_session`]), expõe leituras do sistema e comandos
//! controlados por consentimento via socket UNIX JSON-RPC 2.0 ([`ipc_server`]) e
//! renderiza o deck na TUI Ratatui ([`widget`]).
//!
//! Conforme seção 2 de `docs/backend_architecture.md`.

pub mod ipc_server;
pub mod pty_session;
pub mod widget;
