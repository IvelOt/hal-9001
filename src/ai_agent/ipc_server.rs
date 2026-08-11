//! Servidor **IPC JSON-RPC 2.0** sobre **UNIX Domain Socket** para o AI Terminal Deck.
//!
//! Conforme seção 2.2 de `docs/backend_architecture.md`:
//!
//! * Escuta em `/run/user/$UID/hall-9001.sock` (derivado de `XDG_RUNTIME_DIR`).
//! * Protocolo JSON-RPC 2.0, uma requisição por linha (newline-delimited).
//! * Métodos de **leitura** (bateria, rede, discos) respondem instantaneamente.
//! * Ações **mutáveis** (montar/desmontar, Wi-Fi, execução de terminal, entrada no
//!   PTY) passam pelo [`Gatekeeper`], que exige consentimento do usuário em
//!   pop-up modal na TUI antes de executar.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{oneshot, Mutex as TokioMutex, RwLock};

use crate::ai_agent::pty_session::PtyTarget;
use crate::backend::network::Network;
use crate::backend::power::Power;
use crate::backend::storage::Storage;

/// Nome do arquivo de socket UNIX.
pub const SOCKET_FILENAME: &str = "hall-9001.sock";

/// Retorna o caminho padrão do socket: `$XDG_RUNTIME_DIR/hall-9001.sock`
/// (normalmente `/run/user/$UID/hall-9001.sock`). Fallbacks: `/run/user/$UID`
/// ou o diretório temporário.
pub fn default_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        if !runtime_dir.is_empty() {
            return PathBuf::from(runtime_dir).join(SOCKET_FILENAME);
        }
    }
    if let Ok(uid) = std::env::var("UID") {
        if !uid.is_empty() {
            let path = PathBuf::from("/run/user").join(&uid).join(SOCKET_FILENAME);
            if path.parent().map(|p| p.exists()).unwrap_or(false) {
                return path;
            }
        }
    }
    std::env::temp_dir().join(SOCKET_FILENAME)
}

// ---------------------------------------------------------------------------
// Gatekeeper de consentimento
// ---------------------------------------------------------------------------

/// Um pedido de consentimento pendente, exibido como pop-up modal pela TUI.
#[derive(Debug, Clone, Serialize)]
pub struct ConsentRequest {
    pub id: u64,
    pub method: String,
    pub description: String,
}

/// Handle para a decisão de um pedido de consentimento.
///
/// `wait().await` resolve `true` (aprovado) ou `false` (negado).
pub struct ConsentHandle {
    rx: oneshot::Receiver<bool>,
}

impl ConsentHandle {
    /// Aguarda a decisão da TUI (ou a resolução automática do gatekeeper).
    pub async fn wait(self) -> bool {
        self.rx.await.unwrap_or(false)
    }
}

#[derive(Default)]
struct GatekeeperInner {
    pending: VecDeque<ConsentRequest>,
    resolvers: HashMap<u64, oneshot::Sender<bool>>,
    next_id: u64,
    /// Modo de desenvolvimento: aprova automaticamente todo pedido.
    auto_approve: bool,
    /// `true` quando a TUI está anexada exibindo o modal de consentimento.
    listener_attached: bool,
}

/// Gatekeeper de consentimento para chamadas mutáveis de agentes de I.A.
///
/// Cloneable e compartilhável entre o servidor IPC (que registra pedidos) e a
/// TUI (que os exibe e resolve).
#[derive(Clone)]
pub struct Gatekeeper {
    inner: Arc<Mutex<GatekeeperInner>>,
}

impl Default for Gatekeeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Gatekeeper {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(GatekeeperInner::default())),
        }
    }

    /// Ativa/desativa aprovação automática (útil para desenvolvimento).
    pub fn set_auto_approve(&self, enabled: bool) {
        self.inner.lock().unwrap().auto_approve = enabled;
    }

    /// A TUI chama isto ao anexar o pop-up modal de consentimento.
    pub fn attach_listener(&self) {
        self.inner.lock().unwrap().listener_attached = true;
    }

    /// A TUI chama isto ao desmontar o pop-up modal.
    pub fn detach_listener(&self) {
        self.inner.lock().unwrap().listener_attached = false;
    }

    /// `true` se uma TUI está anexada para coletar consentimento.
    pub fn has_listener(&self) -> bool {
        self.inner.lock().unwrap().listener_attached
    }

    /// `true` se pedidos são aprovados automaticamente.
    pub fn auto_approve(&self) -> bool {
        self.inner.lock().unwrap().auto_approve
    }

    /// Lista os pedidos pendentes para a TUI exibir no modal.
    pub fn pending(&self) -> Vec<ConsentRequest> {
        self.inner.lock().unwrap().pending.iter().cloned().collect()
    }

    /// Resolve um pedido pendente com a decisão do usuário.
    pub fn resolve(&self, id: u64, approved: bool) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        let sender = inner
            .resolvers
            .remove(&id)
            .ok_or_else(|| anyhow!("pedido de consentimento {id} não encontrado"))?;
        inner.pending.retain(|req| req.id != id);
        let _ = sender.send(approved);
        Ok(())
    }

    /// Registra um pedido de consentimento e retorna um handle para a decisão.
    ///
    /// * Com `auto_approve`, resolve `true` imediatamente.
    /// * Sem TUI anexada (nenhum listener), resolve `false` imediatamente —
    ///   a ação é negada de forma segura por padrão.
    pub fn request(&self, method: &str, description: &str) -> ConsentHandle {
        let mut inner = self.inner.lock().unwrap();
        let (tx, rx) = oneshot::channel();

        // Modo de desenvolvimento: aprova automaticamente.
        if inner.auto_approve {
            let _ = tx.send(true);
            return ConsentHandle { rx };
        }
        // Sem TUI anexada, o padrão seguro é negar a ação imediatamente.
        if !inner.listener_attached {
            let _ = tx.send(false);
            return ConsentHandle { rx };
        }

        inner.next_id += 1;
        let id = inner.next_id;
        inner.resolvers.insert(
            id,
            tx,
        );
        inner.pending.push_back(ConsentRequest {
            id,
            method: method.to_string(),
            description: description.to_string(),
        });
        ConsentHandle { rx }
    }
}

// ---------------------------------------------------------------------------
// Backends D-Bus (criados sob demanda)
// ---------------------------------------------------------------------------

/// Backends de leitura do sistema, instanciados preguiçosamente na primeira
/// chamada (evita falhas quando o barramento D-Bus não está disponível).
struct IpcBackends {
    power: Option<Power>,
    network: Option<Network>,
    storage: Option<Storage>,
}

impl Default for IpcBackends {
    fn default() -> Self {
        Self {
            power: None,
            network: None,
            storage: None,
        }
    }
}

impl IpcBackends {
    async fn power(&mut self) -> Result<&Power> {
        if self.power.is_none() {
            self.power = Some(Power::new().await?);
        }
        Ok(self.power.as_ref().expect("power inicializado acima"))
    }

    async fn network(&mut self) -> Result<&Network> {
        if self.network.is_none() {
            self.network = Some(Network::new().await?);
        }
        Ok(self.network.as_ref().expect("network inicializado acima"))
    }

    async fn storage(&mut self) -> Result<&Storage> {
        if self.storage.is_none() {
            self.storage = Some(Storage::new().await?);
        }
        Ok(self.storage.as_ref().expect("storage inicializado acima"))
    }
}

// ---------------------------------------------------------------------------
// Tipos JSON-RPC 2.0
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct RpcRequest {
    #[serde(rename = "jsonrpc")]
    jsonrpc: Option<String>,
    /// Ausente para notificações; `Some(Value::Null)` para id explícito `null`.
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i64,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }

    fn parse_error(detail: &str) -> Self {
        Self::new(-32700, format!("erro de parse do JSON: {detail}"))
    }

    fn invalid_request() -> Self {
        Self::new(-32600, "requisição inválida (esperado JSON-RPC 2.0)")
    }

    fn method_not_found(method: &str) -> Self {
        Self::new(-32601, format!("método não encontrado: {method}"))
    }

    fn invalid_params(detail: &str) -> Self {
        Self::new(-32602, format!("parâmetros inválidos: {detail}"))
    }

    fn consent_unavailable(method: &str) -> Self {
        Self::new(
            -32001,
            format!("consentimento indisponível para `{method}` (nenhuma TUI anexada ao gatekeeper)"),
        )
    }

    fn consent_denied(method: &str) -> Self {
        Self::new(-32000, format!("consentimento negado para `{method}`"))
    }

    fn backend(detail: impl ToString) -> Self {
        Self::new(-32003, detail.to_string())
    }

    fn no_session(detail: &str) -> Self {
        Self::new(-32002, detail.to_string())
    }
}

#[derive(Serialize)]
struct RpcResponse {
    jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

impl RpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, error: RpcError) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(error),
        }
    }
}

// ---------------------------------------------------------------------------
// Servidor
// ---------------------------------------------------------------------------

/// Servidor IPC JSON-RPC 2.0 do AI Terminal Deck.
pub struct IpcServer {
    listener: UnixListener,
    socket_path: PathBuf,
    backends: Arc<TokioMutex<IpcBackends>>,
    gatekeeper: Gatekeeper,
    pty: Arc<RwLock<Option<Arc<dyn PtyTarget + Send + Sync>>>>,
}

impl IpcServer {
    /// Cria o listener no caminho dado, removendo sockets órfãos.
    pub async fn bind(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(anyhow!(
                    "já existe um servidor hall-9001 ouvindo em {path:?}"
                ));
            }
            std::fs::remove_file(path)
                .with_context(|| format!("falha ao remover socket órfão {path:?}"))?;
        }
        let listener = UnixListener::bind(path)
            .with_context(|| format!("falha ao abrir socket UNIX em {path:?}"))?;
        Ok(Self {
            listener,
            socket_path: path.to_path_buf(),
            backends: Arc::new(TokioMutex::new(IpcBackends::default())),
            gatekeeper: Gatekeeper::new(),
            pty: Arc::new(RwLock::new(None)),
        })
    }

    /// Cria o listener no caminho padrão (`/run/user/$UID/hall-9001.sock`).
    pub async fn bind_default() -> Result<Self> {
        Self::bind(default_socket_path()).await
    }

    /// Caminho do socket UNIX em que o servidor está ouvindo.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Clone do gatekeeper para a TUI anexar e resolver consentimentos.
    pub fn gatekeeper(&self) -> Gatekeeper {
        self.gatekeeper.clone()
    }

    /// Anexa a sessão PTY ativa ao servidor, permitindo `system.exec` e `pty.input`.
    pub fn attach_pty(&self, session: Arc<dyn PtyTarget + Send + Sync>) {
        let pty = self.pty.clone();
        tokio::spawn(async move {
            *pty.write().await = Some(session);
        });
    }

    /// Desanexa a sessão PTY ativa.
    pub async fn detach_pty(&self) {
        *self.pty.write().await = None;
    }

    /// Loop principal de aceitação de conexões.
    pub async fn serve(self) -> Result<()> {
        eprintln!(
            "[ipc] AI Terminal Deck ouvindo em {} (JSON-RPC 2.0)",
            self.socket_path.display()
        );
        loop {
            let (stream, _addr) = self
                .listener
                .accept()
                .await
                .context("falha ao aceitar conexão IPC")?;
            let backends = self.backends.clone();
            let gatekeeper = self.gatekeeper.clone();
            let pty = self.pty.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_client(stream, backends, gatekeeper, pty).await {
                    eprintln!("[ipc] cliente encerrado com erro: {e}");
                }
            });
        }
    }
}

type PtySlot = Arc<RwLock<Option<Arc<dyn PtyTarget + Send + Sync>>>>;

/// Atende um cliente: lê requisições linha a linha e responde em JSON-RPC 2.0.
async fn handle_client(
    stream: UnixStream,
    backends: Arc<TokioMutex<IpcBackends>>,
    gatekeeper: Gatekeeper,
    pty: PtySlot,
) -> Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .await
            .context("falha ao ler requisição IPC")?;
        if read == 0 {
            return Ok(()); // cliente fechou a conexão
        }
        let request = line.trim();
        if request.is_empty() {
            continue;
        }
        let response = handle_request(request, &backends, &gatekeeper, &pty).await;
        if let Some(response) = response {
            let mut payload = serde_json::to_string(&response)
                .context("falha ao serializar resposta IPC")?;
            payload.push('\n');
            writer
                .write_all(payload.as_bytes())
                .await
                .context("falha ao escrever resposta IPC")?;
            writer
                .flush()
                .await
                .context("falha ao descarregar resposta IPC")?;
        }
    }
}

/// Processa uma linha de requisição e devolve a resposta (ou `None` p/ notificação).
async fn handle_request(
    line: &str,
    backends: &Arc<TokioMutex<IpcBackends>>,
    gatekeeper: &Gatekeeper,
    pty: &PtySlot,
) -> Option<RpcResponse> {
    let request: RpcRequest = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(e) => return Some(RpcResponse::error(None, RpcError::parse_error(&e.to_string()))),
    };

    if request.jsonrpc.as_deref() != Some("2.0") {
        return Some(RpcResponse::error(request.id.clone(), RpcError::invalid_request()));
    }

    let result = dispatch(&request.method, request.params.as_ref(), backends, gatekeeper, pty).await;
    let response = match result {
        Ok(value) => RpcResponse::success(request.id.clone(), value),
        Err(error) => RpcResponse::error(request.id.clone(), error),
    };

    // Notificações (sem `id`) não recebem resposta, conforme JSON-RPC 2.0.
    if request.id.is_none() {
        None
    } else {
        Some(response)
    }
}

/// Roteia o método JSON-RPC para o manipulador correspondente.
async fn dispatch(
    method: &str,
    params: Option<&Value>,
    backends: &Arc<TokioMutex<IpcBackends>>,
    gatekeeper: &Gatekeeper,
    pty: &PtySlot,
) -> Result<Value, RpcError> {
    match method {
        "system.info" => system_info(gatekeeper).await,
        "battery.get" => battery_get(backends).await,
        "network.wifi" => network_wifi(backends).await,
        "storage.list" => storage_list(backends).await,
        "storage.mount" | "storage.unmount" => storage_mount(backends, gatekeeper, params, method).await,
        "network.wifi_set" => network_wifi_set(backends, gatekeeper, params).await,
        "system.exec" => system_exec(gatekeeper, pty, params).await,
        "pty.input" => pty_input(gatekeeper, pty, params).await,
        _ => Err(RpcError::method_not_found(method)),
    }
}

/// Métodos de leitura — respondem instantaneamente (sem consentimento).
async fn system_info(gatekeeper: &Gatekeeper) -> Result<Value, RpcError> {
    let socket = default_socket_path();
    Ok(json!({
        "name": "hall-9001",
        "protocol": "json-rpc-2.0",
        "socket": socket,
        "gatekeeper_attached": gatekeeper.has_listener(),
        "gatekeeper_auto_approve": gatekeeper.auto_approve(),
        "methods": [
            "system.info", "battery.get", "network.wifi", "storage.list",
            "storage.mount", "storage.unmount", "network.wifi_set",
            "system.exec", "pty.input",
        ],
    }))
}

async fn battery_get(backends: &Arc<TokioMutex<IpcBackends>>) -> Result<Value, RpcError> {
    let mut backends = backends.lock().await;
    let power = backends.power().await.map_err(|e| RpcError::backend(e))?;
    let on_battery = power
        .on_battery()
        .await
        .map_err(|e| RpcError::backend(e))?;
    let batteries = power
        .batteries()
        .await
        .map_err(|e| RpcError::backend(e))?;
    let primary = batteries.iter().find(|b| b.power_supply);
    Ok(json!({
        "on_battery": on_battery,
        "primary": primary,
        "batteries": batteries,
    }))
}

async fn network_wifi(backends: &Arc<TokioMutex<IpcBackends>>) -> Result<Value, RpcError> {
    let mut backends = backends.lock().await;
    let network = backends.network().await.map_err(|e| RpcError::backend(e))?;
    let active = network.active_wifi().await.map_err(|e| RpcError::backend(e))?;
    let wireless_enabled = network
        .wireless_enabled()
        .await
        .map_err(|e| RpcError::backend(e))?;
    Ok(json!({
        "active": active,
        "wireless_enabled": wireless_enabled,
    }))
}

async fn storage_list(backends: &Arc<TokioMutex<IpcBackends>>) -> Result<Value, RpcError> {
    let mut backends = backends.lock().await;
    let storage = backends.storage().await.map_err(|e| RpcError::backend(e))?;
    let devices = storage
        .block_devices()
        .await
        .map_err(|e| RpcError::backend(e))?;
    Ok(json!({ "devices": devices }))
}

/// Métodos mutáveis — exigem consentimento do gatekeeper antes de executar.
async fn storage_mount(
    backends: &Arc<TokioMutex<IpcBackends>>,
    gatekeeper: &Gatekeeper,
    params: Option<&Value>,
    method: &str,
) -> Result<Value, RpcError> {
    let object_path = params
        .and_then(|p| p.get("object_path"))
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("campo `object_path` (string) obrigatório"))?;

    let action = if method == "storage.mount" { "montar" } else { "desmontar" };
    request_consent(gatekeeper, method, &format!("{action} o dispositivo {object_path}?")).await?;

    let mut backends = backends.lock().await;
    let storage = backends.storage().await.map_err(|e| RpcError::backend(e))?;
    if method == "storage.mount" {
        let mount_point = storage
            .mount(object_path)
            .await
            .map_err(|e| RpcError::backend(e))?;
        Ok(json!({ "mounted": true, "mount_point": mount_point }))
    } else {
        storage
            .unmount(object_path)
            .await
            .map_err(|e| RpcError::backend(e))?;
        Ok(json!({ "mounted": false }))
    }
}

async fn network_wifi_set(
    backends: &Arc<TokioMutex<IpcBackends>>,
    gatekeeper: &Gatekeeper,
    params: Option<&Value>,
) -> Result<Value, RpcError> {
    let enabled = params
        .and_then(|p| p.get("enabled"))
        .and_then(Value::as_bool)
        .ok_or_else(|| RpcError::invalid_params("campo `enabled` (boolean) obrigatório"))?;

    request_consent(
        gatekeeper,
        "network.wifi_set",
        &if enabled { "ligar o Wi-Fi?" } else { "desligar o Wi-Fi?" },
    )
    .await?;

    let mut backends = backends.lock().await;
    let network = backends.network().await.map_err(|e| RpcError::backend(e))?;
    network
        .set_wireless_enabled(enabled)
        .await
        .map_err(|e| RpcError::backend(e))?;
    Ok(json!({ "wireless_enabled": enabled }))
}

/// Executa um comando no terminal do agente (via PTY) após consentimento.
async fn system_exec(
    gatekeeper: &Gatekeeper,
    pty: &PtySlot,
    params: Option<&Value>,
) -> Result<Value, RpcError> {
    let command = params
        .and_then(|p| p.get("command"))
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("campo `command` (string) obrigatório"))?;

    request_consent(
        gatekeeper,
        "system.exec",
        &format!("executar no AI Terminal Deck: {command}"),
    )
    .await?;

    let target = { pty.read().await.clone() }
        .ok_or_else(|| RpcError::no_session("nenhuma sessão PTY ativa no AI Terminal Deck"))?;

    target
        .write_input(format!("{command}\r").as_bytes())
        .map_err(|e| RpcError::backend(e))?;
    Ok(json!({ "queued": true, "target": "ai-terminal-deck" }))
}

/// Envia entrada crua (teclas ANSI) para o agente após consentimento.
async fn pty_input(
    gatekeeper: &Gatekeeper,
    pty: &PtySlot,
    params: Option<&Value>,
) -> Result<Value, RpcError> {
    let data = params
        .and_then(|p| p.get("data"))
        .and_then(Value::as_str)
        .ok_or_else(|| RpcError::invalid_params("campo `data` (string) obrigatório"))?;

    request_consent(gatekeeper, "pty.input", "injetar entrada no agente (AI Terminal Deck)").await?;

    let target = { pty.read().await.clone() }
        .ok_or_else(|| RpcError::no_session("nenhuma sessão PTY ativa no AI Terminal Deck"))?;

    target
        .write_input(data.as_bytes())
        .map_err(|e| RpcError::backend(e))?;
    Ok(json!({ "queued": true, "bytes": data.len() }))
}

/// Registra um pedido de consentimento e aguarda a decisão do usuário.
async fn request_consent(
    gatekeeper: &Gatekeeper,
    method: &str,
    description: &str,
) -> Result<(), RpcError> {
    // Sem TUI anexada e sem modo de desenvolvimento, a ação é negada de imediato.
    if !gatekeeper.has_listener() && !gatekeeper.auto_approve() {
        return Err(RpcError::consent_unavailable(method));
    }
    let handle = gatekeeper.request(method, description);
    if handle.wait().await {
        Ok(())
    } else {
        Err(RpcError::consent_denied(method))
    }
}
