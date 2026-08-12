//! Testes de integração do servidor IPC **JSON-RPC 2.0** sobre **UNIX socket**.
//!
//! Cada teste inicia um [`IpcServer`] real em um socket temporário sob
//! `/tmp/test-hall-9001-*.sock`, envia requisições via `UnixStream` e valida a
//! estrutura das respostas conforme a seção 2.2 de `docs/backend_architecture.md`.

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use hall_9001::ai_agent::ipc_server::IpcServer;

/// Inicia um servidor IPC no caminho dado em uma task em segundo plano.
async fn start_server(socket_path: &str) {
    let server = IpcServer::bind(socket_path)
        .await
        .unwrap_or_else(|e| panic!("falha ao abrir socket IPC de teste {socket_path}: {e}"));
    tokio::spawn(async move {
        if let Err(e) = server.serve().await {
            panic!("servidor IPC de teste encerrou com erro: {e}");
        }
    });
}

/// Envia uma requisição JSON-RPC e devolve a resposta parseada como `Value`.
async fn rpc_call(socket_path: &str, id: u64, method: &str, params: Option<Value>) -> Value {
    let mut stream = UnixStream::connect(socket_path)
        .await
        .unwrap_or_else(|e| panic!("falha ao conectar no socket {socket_path}: {e}"));

    let mut payload = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    })
    .to_string();
    payload.push('\n');

    stream
        .write_all(payload.as_bytes())
        .await
        .expect("falha ao enviar requisição JSON-RPC");
    stream
        .flush()
        .await
        .expect("falha ao descarregar requisição JSON-RPC");

    let mut reader = tokio::io::BufReader::new(stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .await
        .expect("falha ao ler resposta JSON-RPC");

    serde_json::from_str(&response).expect("resposta não é JSON válido")
}

/// Valida o envelope JSON-RPC de uma resposta de sucesso: protocolo 2.0, id
/// ecoado, `result` presente e ausência total de `error`.
fn assert_rpc_success(response: &Value, id: u64) {
    assert_eq!(
        response["jsonrpc"], "2.0",
        "protocolo deve ser JSON-RPC 2.0: {response}"
    );
    assert_eq!(response["id"], json!(id), "id deve ecoar a requisição: {response}");
    assert!(
        response.get("error").is_none(),
        "resposta de sucesso não deve conter `error`: {response}"
    );
    assert!(
        response.get("result").is_some(),
        "resposta deve conter `result`: {response}"
    );
}

/// Valida que a resposta é JSON-RPC 2.0 bem formada — `result` ou `error`
/// (exclusivos), e, na presença de `result`, valida o campo dado.
///
/// Os métodos de leitura dependem de D-Bus; em ambientes sem barramento o
/// servidor responde com um erro JSON-RPC bem formado, e o teste continua verde.
fn assert_rpc_ok_or_well_formed_error(response: &Value, id: u64) {
    assert_eq!(
        response["jsonrpc"], "2.0",
        "protocolo deve ser JSON-RPC 2.0: {response}"
    );
    assert_eq!(response["id"], json!(id), "id deve ecoar a requisição: {response}");
    let has_result = response.get("result").is_some();
    let has_error = response.get("error").is_some();
    assert!(
        has_result || has_error,
        "resposta deve conter `result` ou `error`: {response}"
    );
    assert!(
        !(has_result && has_error),
        "resposta não pode conter `result` e `error` juntos: {response}"
    );
    if let Some(error) = response.get("error") {
        assert!(
            error.get("code").is_some(),
            "erro JSON-RPC deve conter `code`: {response}"
        );
        assert!(
            error.get("message").is_some(),
            "erro JSON-RPC deve conter `message`: {response}"
        );
    }
}

#[tokio::test]
async fn system_info_returns_metadata() {
    let path = "/tmp/test-hall-9001-system-info.sock";
    start_server(path).await;

    let response = rpc_call(path, 42, "system.info", None).await;
    assert_rpc_success(&response, 42);

    let result = &response["result"];
    assert_eq!(result["name"], "hall-9001", "nome do sistema: {result}");
    assert_eq!(result["protocol"], "json-rpc-2.0", "protocolo: {result}");
    assert!(
        result["methods"].is_array(),
        "`methods` deve ser um array: {result}"
    );
    assert!(
        result["gatekeeper_attached"].is_boolean(),
        "`gatekeeper_attached` deve ser booleano: {result}"
    );
    assert!(
        result["gatekeeper_auto_approve"].is_boolean(),
        "`gatekeeper_auto_approve` deve ser booleano: {result}"
    );
}

#[tokio::test]
async fn battery_get_returns_power_info() {
    let path = "/tmp/test-hall-9001-battery-get.sock";
    start_server(path).await;

    let response = rpc_call(path, 7, "battery.get", None).await;
    assert_rpc_ok_or_well_formed_error(&response, 7);

    if let Some(result) = response.get("result") {
        assert!(
            result.get("on_battery").is_some(),
            "result deve conter `on_battery`: {result}"
        );
        assert!(
            result["batteries"].is_array(),
            "`batteries` deve ser um array: {result}"
        );
        assert!(
            result.get("primary").is_some(),
            "result deve conter `primary`: {result}"
        );
    }
}

#[tokio::test]
async fn storage_list_returns_devices() {
    let path = "/tmp/test-hall-9001-storage-list.sock";
    start_server(path).await;

    let response = rpc_call(path, 9, "storage.list", None).await;
    assert_rpc_ok_or_well_formed_error(&response, 9);

    if let Some(result) = response.get("result") {
        assert!(
            result["devices"].is_array(),
            "`devices` deve ser um array: {result}"
        );
    }
}

#[tokio::test]
async fn unknown_method_returns_json_rpc_error() {
    let path = "/tmp/test-hall-9001-unknown-method.sock";
    start_server(path).await;

    let response = rpc_call(path, 3, "system.nonexistent", None).await;
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], json!(3));
    assert!(response.get("result").is_none(), "não deve haver result: {response}");
    assert_eq!(
        response["error"]["code"],
        -32601,
        "método desconhecido deve retornar código -32601: {response}"
    );
}

#[tokio::test]
async fn invalid_json_returns_parse_error() {
    let path = "/tmp/test-hall-9001-invalid-json.sock";
    start_server(path).await;

    let mut stream = UnixStream::connect(path)
        .await
        .expect("falha ao conectar no socket de teste");
    stream
        .write_all(b"isto nao e JSON\n")
        .await
        .expect("falha ao enviar payload inválido");

    let mut reader = tokio::io::BufReader::new(stream);
    let mut response = String::new();
    reader
        .read_line(&mut response)
        .await
        .expect("falha ao ler resposta de erro");
    let response: Value = serde_json::from_str(&response).expect("resposta não é JSON válido");

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(
        response["error"]["code"],
        -32700,
        "JSON inválido deve retornar código -32700: {response}"
    );
    assert!(response.get("result").is_none());
}

#[tokio::test]
async fn socket_file_is_removed_on_shutdown() {
    let path = "/tmp/test-hall-9001-cleanup.sock";
    let _ = std::fs::remove_file(path);

    {
        let server = IpcServer::bind(path)
            .await
            .expect("falha ao abrir socket IPC de teste");
        assert!(
            std::path::Path::new(path).exists(),
            "socket deve existir após o bind"
        );
        drop(server);
    }

    assert!(
        !std::path::Path::new(path).exists(),
        "socket deve ser removido do sistema após encerrar o servidor"
    );
}
