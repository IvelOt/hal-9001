//! Gerenciamento do ciclo de vida de um **pseudo-terminal (PTY)** para hospedar
//! sessões interativas com agentes de I.A. (`opencode`, `claude`) ou `bash`.
//!
//! Conforme seção 2.1 de `docs/backend_architecture.md`:
//!
//! * O processo do agente é iniciado no `SlavePty` (`portable-pty`).
//! * A saída do `MasterPty` é lida em uma thread bloqueante e interpretada por um
//!   parser ANSI `vt100` — a tela virtual resultante alimenta o widget da TUI.
//! * Teclas / bytes vindos da TUI (ou do IPC) são escritos no master.
//! * O redimensionamento (`resize`) atualiza o `winsize` do kernel via
//!   `master_pty.resize()`, o que dispara o sinal `SIGWINCH` para o grupo de
//!   processo do agente ajustar a própria tela.

use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

/// Tipos de agentes de I.A. suportados pelo AI Terminal Deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentKind {
    /// OpenCode — agente de código interativo.
    OpenCode,
    /// Claude Code — agente de código da Anthropic.
    Claude,
    /// Shell `bash` — terminal genérico.
    Bash,
}

impl AgentKind {
    /// Nome do binário executável do agente.
    pub fn program(self) -> &'static str {
        match self {
            Self::OpenCode => "opencode",
            Self::Claude => "claude",
            Self::Bash => "bash",
        }
    }

    /// Rótulo amigável exibido na TUI.
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenCode => "OpenCode",
            Self::Claude => "Claude",
            Self::Bash => "bash",
        }
    }
}

/// Comando a ser executado dentro do PTY (binário do agente + argumentos).
#[derive(Debug, Clone)]
pub struct AgentCommand {
    pub kind: AgentKind,
    pub args: Vec<String>,
}

impl AgentCommand {
    /// Cria um comando de agente com argumentos opcionais.
    pub fn new(kind: AgentKind, args: Vec<String>) -> Self {
        Self { kind, args }
    }

    /// Monta o `CommandBuilder` do `portable-pty` com o binário e argumentos.
    pub fn command_builder(&self) -> CommandBuilder {
        let mut builder = CommandBuilder::new(self.kind.program());
        for arg in &self.args {
            builder.arg(arg);
        }
        builder
    }
}

/// Alvo de PTY compartilhável entre a TUI e o servidor IPC (métodos `&self`).
///
/// Permite que o `IpcServer` injete entrada do agente sem conhecer os detalhes
/// internos do `PtySession`.
pub trait PtyTarget: Send + Sync {
    /// Escreve bytes crus no master do PTY (teclado / entrada ANSI).
    fn write_input(&self, bytes: &[u8]) -> Result<()>;

    /// Redimensiona a tela virtual e o PTY, disparando `SIGWINCH` para o agente.
    fn resize(&self, rows: u16, cols: u16) -> Result<()>;
}

/// Sessão de PTY assíncrona do AI Terminal Deck.
///
/// O processo do agente roda dentro do `SlavePty`; a leitura do `MasterPty` é
/// feita por uma thread bloqueante que alimenta o parser `vt100` diretamente
/// (compartilhado via `Arc<Mutex>`), de forma que a tela virtual está sempre
/// atualizada e pode ser renderizada por qualquer thread.
pub struct PtySession {
    command: AgentCommand,
    /// Lado mestre do PTY — usado para `resize`/`get_size` (preenchido em `start`).
    ///
    /// Em `Mutex` porque `dyn MasterPty + Send` não é `Sync` e o `PtySession`
    /// precisa ser compartilhado entre a TUI e o servidor IPC (`Send + Sync`).
    master: Mutex<Option<Box<dyn MasterPty + Send>>>,
    /// Escritor do master — entrada do teclado/IPC enviada ao agente.
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Parser ANSI que mantém a tela virtual (compartilhado com a thread de leitura).
    parser: Arc<Mutex<vt100::Parser>>,
    /// Tamanho atual do PTY.
    size: Mutex<PtySize>,
    /// Handle do processo do agente.
    child: Option<Box<dyn Child + Send + Sync>>,
    /// Handle da thread de leitura (para `stop`).
    reader_thread: Option<JoinHandle<()>>,
    /// `true` quando a thread de leitura encerra (agente saiu / PTY fechado).
    exited: Arc<AtomicBool>,
    /// Sequência de bytes processados — permite detectar novo output (redraw).
    output_seq: Arc<AtomicU64>,
}

impl PtySession {
    /// Cria uma sessão para o comando dado com dimensões padrão (24x80).
    pub fn new(command: AgentCommand) -> Self {
        Self::with_size(command, PtySize::default())
    }

    /// Cria uma sessão com dimensões iniciais explícitas.
    pub fn with_size(command: AgentCommand, size: PtySize) -> Self {
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            size.rows,
            size.cols,
            0,
        )));
        Self {
            command,
            // Preenchido em `start`; `openpty` entrega o par master/slave.
            master: Mutex::new(None),
            writer: Arc::new(Mutex::new(Box::new(std::io::sink()))),
            parser,
            size: Mutex::new(size),
            child: None,
            reader_thread: None,
            exited: Arc::new(AtomicBool::new(false)),
            output_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Inicializa o PTY (Master/Slave), dispara o subprocesso do agente e inicia
    /// a thread de leitura que alimenta o parser `vt100`.
    pub fn start(&mut self) -> Result<()> {
        let pty_system = native_pty_system();
        let size = *self.size.get_mut().unwrap();
        let pair = pty_system
            .openpty(size)
            .context("falha ao abrir pseudo-terminal (portable-pty)")?;

        let child = pair
            .slave
            .spawn_command(self.command.command_builder())
            .context("falha ao iniciar subprocesso do agente no PTY")?;

        let reader = pair
            .master
            .try_clone_reader()
            .context("falha ao obter leitor do master PTY")?;
        let writer = pair
            .master
            .take_writer()
            .context("falha ao obter escritor do master PTY")?;

        *self.master.lock().unwrap() = Some(pair.master);
        self.writer = Arc::new(Mutex::new(writer));
        self.child = Some(child);

        self.spawn_reader(reader)?;

        Ok(())
    }

    /// Inicia a thread bloqueante que lê o master e alimenta o parser `vt100`.
    fn spawn_reader(&mut self, mut reader: Box<dyn Read + Send>) -> Result<()> {
        let parser = self.parser.clone();
        let exited = self.exited.clone();
        let output_seq = self.output_seq.clone();

        let handle = std::thread::Builder::new()
            .name("ai-pty-reader".to_string())
            .spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    match reader.read(&mut buffer) {
                        // EOF: o agente saiu / o slave foi fechado.
                        Ok(0) => break,
                        Ok(n) => {
                            if let Ok(mut parser) = parser.lock() {
                                parser.process(&buffer[..n]);
                            }
                            output_seq.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
                exited.store(true, Ordering::SeqCst);
            })
            .context("falha ao criar thread de leitura do PTY")?;

        self.reader_thread = Some(handle);
        Ok(())
    }

    /// Retorna o comando do agente desta sessão.
    pub fn command(&self) -> &AgentCommand {
        &self.command
    }

    /// `true` se o agente saiu ou o PTY foi fechado.
    pub fn has_exited(&self) -> bool {
        self.exited.load(Ordering::SeqCst)
    }

    /// Sequência de bytes processados pelo parser (aumenta a cada nova saída).
    pub fn output_seq(&self) -> u64 {
        self.output_seq.load(Ordering::Relaxed)
    }

    /// Executa um fechamento com a tela virtual atual do PTY.
    ///
    /// O widget usa isso para renderizar o conteúdo: a tela é lida sob o lock do
    /// parser e o resultado é entregue ao fechamento.
    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        let parser = self.parser.lock().expect("lock do parser vt100 corrompido");
        f(parser.screen())
    }

    /// PID do processo do agente, se disponível.
    #[allow(dead_code)]
    pub fn process_id(&self) -> Option<u32> {
        self.child.as_ref().and_then(|child| child.process_id())
    }

    /// Encerra o agente (kill) e aguarda a thread de leitura terminar.
    pub fn stop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
        }
        if let Some(handle) = self.reader_thread.take() {
            let _ = handle.join();
        }
        self.exited.store(true, Ordering::SeqCst);
    }
}

impl PtyTarget for PtySession {
    fn write_input(&self, bytes: &[u8]) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("lock do escritor do PTY corrompido"))?;
        writer
            .write_all(bytes)
            .context("falha ao escrever entrada no PTY")?;
        writer.flush().context("falha ao descarregar entrada no PTY")
    }

    fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };
        {
            let master = self.master.lock().expect("lock do master PTY corrompido");
            master
                .as_ref()
                .context("sessão PTY não iniciada")?
                .resize(size)
                .context("falha ao redimensionar o PTY (TIOCSWINSZ)")?;
        }
        *self.size.lock().expect("lock do tamanho do PTY corrompido") = size;
        if let Ok(mut parser) = self.parser.lock() {
            parser.screen_mut().set_size(rows, cols);
        }
        Ok(())
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        self.stop();
    }
}
