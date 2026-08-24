//! Fundação de PTY (`portable-pty`) para o Terminal Deck (aba 8) e o Yazi
//! (aba 7). Cada sessão roda um filho num pseudo-terminal nativo, cuja saída
//! é interpretada por um `vt100::Parser` numa thread dedicada (a leitura do
//! master é bloqueante) e publicada como `AppEvent::PtyScreenUpdate`. A
//! escrita (`Action::PtyInput`) e o redimensionamento (`Action::PtyResize`)
//! chegam pelo canal `broadcast` de `Action`, tratados neste módulo.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use tokio::sync::broadcast;

use crate::events::{Action, AppEvent, EventTx, PtyCell, PtyColor, PtyScreenSnapshot, PtyTarget};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const SCROLLBACK_LINES: usize = 2000;

/// Handle de uma sessão PTY em execução: escrita e redimensionamento.
/// A leitura vive inteiramente na thread spawnada por [`spawn_session`].
struct PtySessionHandle {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Box<dyn MasterPty + Send>,
    /// Compartilhado com a thread leitora: redimensionar só o master do PTY
    /// (nível do SO) não muda a grade que o `vt100::Parser` já interpretou —
    /// é preciso chamar `Parser::set_size` também, daí o `Mutex` cruzando as
    /// duas threads.
    parser: Arc<Mutex<vt100::Parser>>,
}

impl PtySessionHandle {
    fn write(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        let _ = self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        if let Ok(mut parser) = self.parser.lock() {
            parser.set_size(rows, cols);
        }
    }
}

/// Sobe uma sessão PTY executando `cmd`, iniciando a thread leitora que
/// alimenta o `vt100::Parser` e publica snapshots via `tx`.
fn spawn_session(
    target: PtyTarget,
    cmd: CommandBuilder,
    tx: EventTx,
) -> anyhow::Result<PtySessionHandle> {
    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: DEFAULT_ROWS,
        cols: DEFAULT_COLS,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    pair.slave.spawn_command(cmd)?;

    let reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let parser = Arc::new(Mutex::new(vt100::Parser::new(
        DEFAULT_ROWS,
        DEFAULT_COLS,
        SCROLLBACK_LINES,
    )));

    std::thread::spawn({
        let parser = Arc::clone(&parser);
        move || read_loop(target, reader, parser, tx)
    });

    Ok(PtySessionHandle {
        writer: Arc::new(Mutex::new(writer)),
        master: pair.master,
        parser,
    })
}

/// Loop bloqueante executado numa thread dedicada: lê bytes brutos do PTY,
/// alimenta o parser VT100 compartilhado e publica um snapshot da tela a
/// cada leitura. Ao encerrar (EOF ou erro de leitura), publica
/// `AppEvent::PtyExited`.
fn read_loop(
    target: PtyTarget,
    mut reader: Box<dyn Read + Send>,
    parser: Arc<Mutex<vt100::Parser>>,
    tx: EventTx,
) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let screen = {
                    let Ok(mut parser) = parser.lock() else {
                        break;
                    };
                    parser.process(&buf[..n]);
                    snapshot(parser.screen())
                };
                if tx
                    .send(AppEvent::PtyScreenUpdate {
                        target,
                        screen: Box::new(screen),
                    })
                    .is_err()
                {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    let _ = tx.send(AppEvent::PtyExited { target });
}

/// Converte o `vt100::Screen` atual num snapshot neutro (`events::Pty*`)
/// pronto para render.
fn snapshot(screen: &vt100::Screen) -> PtyScreenSnapshot {
    let (rows, cols) = screen.size();
    let mut cells = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut line = Vec::with_capacity(cols as usize);
        for col in 0..cols {
            let cell = screen.cell(row, col);
            line.push(match cell {
                Some(c) => PtyCell {
                    ch: c.contents().chars().next().unwrap_or(' '),
                    fg: convert_color(c.fgcolor()),
                    bg: convert_color(c.bgcolor()),
                    bold: c.bold(),
                    underline: c.underline(),
                    inverse: c.inverse(),
                    italic: c.italic(),
                },
                None => PtyCell::default(),
            });
        }
        cells.push(line);
    }

    PtyScreenSnapshot {
        cols,
        rows,
        cells,
        cursor: screen.cursor_position(),
        cursor_visible: !screen.hide_cursor(),
    }
}

fn convert_color(c: vt100::Color) -> PtyColor {
    match c {
        vt100::Color::Default => PtyColor::Default,
        vt100::Color::Idx(i) => PtyColor::Indexed(i),
        vt100::Color::Rgb(r, g, b) => PtyColor::Rgb(r, g, b),
    }
}

/// `TERM` reportado aos processos filhos do PTY. Fixo em vez de herdar o
/// `$TERM` do processo pai (tipicamente `tmux-256color`/`screen-256color`
/// quando o HAL-9001 roda dentro de um multiplexador): esses terminfo
/// habilitam recursos (sequências de shell-integration OSC 133, cursor
/// programável, etc.) que o `vt100::Parser` — um parser VT100/xterm básico —
/// não entende, corrompendo a grade renderizada. `xterm-256color` é o alvo
/// que o crate `vt100` já foi escrito para interpretar.
const PTY_TERM: &str = "xterm-256color";

/// Monta o `CommandBuilder` da sessão Terminal Deck: `$SHELL`, com fallback
/// para `/bin/bash` e depois `/bin/sh`.
fn terminal_command() -> CommandBuilder {
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty());
    let program = shell.unwrap_or_else(|| {
        if Path::new("/bin/bash").exists() {
            "/bin/bash".to_string()
        } else {
            "/bin/sh".to_string()
        }
    });
    let mut cmd = CommandBuilder::new(program);
    cmd.env("TERM", PTY_TERM);
    if let Ok(home) = std::env::var("HOME") {
        cmd.cwd(home);
    }
    cmd
}

/// Varre `$PATH` em busca de um binário `yazi` executável — implementação
/// 100% Rust pura (sem `command -v`/`which` via shell), consistente com o
/// restante do projeto.
fn find_yazi() -> Option<PathBuf> {
    find_in_path("yazi")
}

fn find_in_path(bin: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    find_in_path_var(bin, &path_var)
}

/// Varre os diretórios listados em `path_var` (formato `$PATH`) em busca de
/// um arquivo executável chamado `bin`. Recebe o valor de `PATH` como
/// parâmetro (em vez de ler `std::env` diretamente) para ser exercitada por
/// `tests/pty.rs` com um diretório temporário, sem depender de um binário
/// real instalado nem mutar variáveis de ambiente globais do processo.
pub fn find_in_path_var(bin: &str, path_var: &std::ffi::OsStr) -> Option<PathBuf> {
    for dir in std::env::split_paths(path_var) {
        let candidate = dir.join(bin);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(meta) => meta.is_file() && meta.permissions().mode() & 0o111 != 0,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Sobe as sessões PTY (Terminal Deck sempre; Yazi apenas se encontrado no
/// `$PATH`) e trata `Action::PtyInput`/`Action::PtyResize` vindas do input.
pub async fn run(tx: EventTx, mut actions: broadcast::Receiver<Action>) -> anyhow::Result<()> {
    let terminal_session = spawn_session(PtyTarget::Terminal, terminal_command(), tx.clone())?;

    let files_session = match find_yazi() {
        Some(bin) => {
            let mut cmd = CommandBuilder::new(bin);
            cmd.env("TERM", PTY_TERM);
            Some(spawn_session(PtyTarget::Files, cmd, tx.clone())?)
        }
        None => {
            let _ = tx.send(AppEvent::PtyUnavailable {
                target: PtyTarget::Files,
                reason: "yazi não encontrado no $PATH".to_string(),
            });
            None
        }
    };

    loop {
        match actions.recv().await {
            Ok(Action::PtyInput { target, bytes }) => {
                let session = match target {
                    PtyTarget::Terminal => Some(&terminal_session),
                    PtyTarget::Files => files_session.as_ref(),
                };
                if let Some(session) = session {
                    session.write(&bytes);
                }
            }
            Ok(Action::PtyResize { target, cols, rows }) => {
                let session = match target {
                    PtyTarget::Terminal => Some(&terminal_session),
                    PtyTarget::Files => files_session.as_ref(),
                };
                if let Some(session) = session {
                    session.resize(cols.max(1), rows.max(1));
                }
            }
            Ok(_) => {}
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => break,
        }
    }

    Ok(())
}
