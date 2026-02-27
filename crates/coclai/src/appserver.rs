use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use coclai_runtime::{
    Client, ClientConfig, ClientError, RpcError, RpcErrorObject, RpcValidationMode, Runtime,
    RuntimeError, ServerRequestRx,
};

/// Canonical app-server JSON-RPC method names.
pub mod methods {
    pub use coclai_runtime::rpc_contract::methods::{
        THREAD_ARCHIVE, THREAD_FORK, THREAD_LIST, THREAD_LOADED_LIST, THREAD_READ, THREAD_RESUME,
        THREAD_ROLLBACK, THREAD_START, TURN_INTERRUPT, TURN_START,
    };
}

/// Thin, explicit JSON-RPC facade for codex app-server.
///
/// - `request_json` / `notify_json`: validated calls for known methods.
/// - `request_typed` / `notify_typed`: typed wrappers with contract validation.
/// - `*_unchecked`: bypass contract checks for experimental/custom methods.
/// - server request loop is exposed directly for approval/user-input workflows.
#[derive(Clone)]
pub struct AppServer {
    client: Client,
}

impl AppServer {
    /// Connect app-server with explicit config.
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError> {
        let client = Client::connect(config).await?;
        Ok(Self { client })
    }

    /// Connect app-server with default runtime discovery.
    pub async fn connect_default() -> Result<Self, ClientError> {
        let client = Client::connect_default().await?;
        Ok(Self { client })
    }

    /// Validated JSON-RPC request for known methods.
    pub async fn request_json(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.client.runtime().call_validated(method, params).await
    }

    /// JSON-RPC request with explicit validation mode.
    pub async fn request_json_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<Value, RpcError> {
        self.client
            .runtime()
            .call_validated_with_mode(method, params, mode)
            .await
    }

    /// Typed JSON-RPC request for known methods.
    pub async fn request_typed<P, R>(&self, method: &str, params: P) -> Result<R, RpcError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        self.client
            .runtime()
            .call_typed_validated(method, params)
            .await
    }

    /// Typed JSON-RPC request with explicit validation mode.
    pub async fn request_typed_with_mode<P, R>(
        &self,
        method: &str,
        params: P,
        mode: RpcValidationMode,
    ) -> Result<R, RpcError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        self.client
            .runtime()
            .call_typed_validated_with_mode(method, params, mode)
            .await
    }

    /// Unchecked JSON-RPC request.
    /// Use for experimental/custom methods where strict contracts are not fixed yet.
    pub async fn request_json_unchecked(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        self.client.runtime().call_raw(method, params).await
    }

    /// Validated JSON-RPC notification for known methods.
    pub async fn notify_json(&self, method: &str, params: Value) -> Result<(), RuntimeError> {
        self.client.runtime().notify_validated(method, params).await
    }

    /// JSON-RPC notification with explicit validation mode.
    pub async fn notify_json_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<(), RuntimeError> {
        self.client
            .runtime()
            .notify_validated_with_mode(method, params, mode)
            .await
    }

    /// Typed JSON-RPC notification for known methods.
    pub async fn notify_typed<P>(&self, method: &str, params: P) -> Result<(), RuntimeError>
    where
        P: Serialize,
    {
        self.client
            .runtime()
            .notify_typed_validated(method, params)
            .await
    }

    /// Typed JSON-RPC notification with explicit validation mode.
    pub async fn notify_typed_with_mode<P>(
        &self,
        method: &str,
        params: P,
        mode: RpcValidationMode,
    ) -> Result<(), RuntimeError>
    where
        P: Serialize,
    {
        self.client
            .runtime()
            .notify_typed_validated_with_mode(method, params, mode)
            .await
    }

    /// Unchecked JSON-RPC notification.
    pub async fn notify_json_unchecked(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(), RuntimeError> {
        self.client.runtime().notify_raw(method, params).await
    }

    /// Take exclusive server-request stream receiver.
    ///
    /// This enables explicit handling of approval / requestUserInput / tool-call cycles.
    pub async fn take_server_requests(&self) -> Result<ServerRequestRx, RuntimeError> {
        self.client.runtime().take_server_request_rx().await
    }

    /// Reply success payload for one server request.
    pub async fn respond_server_request_ok(
        &self,
        approval_id: &str,
        result: Value,
    ) -> Result<(), RuntimeError> {
        self.client
            .runtime()
            .respond_approval_ok(approval_id, result)
            .await
    }

    /// Reply error payload for one server request.
    pub async fn respond_server_request_err(
        &self,
        approval_id: &str,
        err: RpcErrorObject,
    ) -> Result<(), RuntimeError> {
        self.client
            .runtime()
            .respond_approval_err(approval_id, err)
            .await
    }

    /// Borrow server runtime for full low-level control.
    pub fn runtime(&self) -> &Runtime {
        self.client.runtime()
    }

    /// Borrow underlying connected client.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Explicit shutdown.
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.client.shutdown().await
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::*;

    #[derive(Debug)]
    struct TempDir {
        root: PathBuf,
    }

    impl TempDir {
        fn new(prefix: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("{prefix}_{nonce}"));
            fs::create_dir_all(&root).expect("create temp root");
            Self { root }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn workspace_schema_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../SCHEMAS/app-server/active")
    }

    fn write_mock_cli_script(root: &std::path::Path) -> PathBuf {
        let path = root.join("mock_appserver.py");
        let script = r#"#!/usr/bin/env python3
import json
import sys

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue

    method = msg.get("method")
    rpc_id = msg.get("id")
    params = msg.get("params") or {}

    if rpc_id is None:
        continue

    if method == "initialize":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True, "userAgent": "Codex Desktop/0.104.0"}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/start":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_rpc"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/interrupt":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;
        fs::write(&path, script).expect("write mock cli");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path).expect("script metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).expect("set script executable");
        }
        path
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_json_validates_known_method_payload() {
        let temp = TempDir::new("coclai_appserver_validated");
        let cli = write_mock_cli_script(&temp.root);
        let app = AppServer::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .with_schema_dir(workspace_schema_dir()),
        )
        .await
        .expect("connect appserver");

        let out = app
            .request_json(methods::THREAD_START, json!({}))
            .await
            .expect("validated request");
        assert_eq!(out["thread"]["id"], "thr_rpc");

        app.shutdown().await.expect("shutdown");
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct ThreadStartTypedResult {
        thread: ThreadIdOnly,
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct ThreadIdOnly {
        id: String,
    }

    #[derive(Debug, Serialize)]
    struct TurnInterruptNotifyMissingTurnId {
        #[serde(rename = "threadId")]
        thread_id: String,
    }

    #[derive(Debug, Serialize)]
    struct TurnInterruptNotifyParams {
        #[serde(rename = "threadId")]
        thread_id: String,
        #[serde(rename = "turnId")]
        turn_id: String,
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_typed_validates_and_deserializes_known_method_payload() {
        let temp = TempDir::new("coclai_appserver_typed");
        let cli = write_mock_cli_script(&temp.root);
        let app = AppServer::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .with_schema_dir(workspace_schema_dir()),
        )
        .await
        .expect("connect appserver");

        let out: ThreadStartTypedResult = app
            .request_typed(methods::THREAD_START, json!({}))
            .await
            .expect("typed request");
        assert_eq!(out.thread.id, "thr_rpc");

        app.shutdown().await.expect("shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_json_rejects_invalid_known_params() {
        let temp = TempDir::new("coclai_appserver_invalid");
        let cli = write_mock_cli_script(&temp.root);
        let app = AppServer::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .with_schema_dir(workspace_schema_dir()),
        )
        .await
        .expect("connect appserver");

        let err = app
            .request_json(methods::TURN_INTERRUPT, json!({"threadId":"thr"}))
            .await
            .expect_err("missing turnId must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        app.shutdown().await.expect("shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn request_json_unchecked_supports_custom_methods() {
        let temp = TempDir::new("coclai_appserver_unchecked");
        let cli = write_mock_cli_script(&temp.root);
        let app = AppServer::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .with_schema_dir(workspace_schema_dir()),
        )
        .await
        .expect("connect appserver");

        let out = app
            .request_json_unchecked("echo/custom", json!({"k":"v"}))
            .await
            .expect("unchecked custom request");
        assert_eq!(out["echoMethod"], "echo/custom");
        assert_eq!(out["params"]["k"], "v");

        app.shutdown().await.expect("shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn notify_json_rejects_invalid_known_params() {
        let temp = TempDir::new("coclai_appserver_notify");
        let cli = write_mock_cli_script(&temp.root);
        let app = AppServer::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .with_schema_dir(workspace_schema_dir()),
        )
        .await
        .expect("connect appserver");

        let err = app
            .notify_json(methods::TURN_INTERRUPT, json!({"threadId":"thr"}))
            .await
            .expect_err("missing turnId must fail");
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));

        app.shutdown().await.expect("shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn notify_typed_rejects_invalid_known_params() {
        let temp = TempDir::new("coclai_appserver_notify_typed_invalid");
        let cli = write_mock_cli_script(&temp.root);
        let app = AppServer::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .with_schema_dir(workspace_schema_dir()),
        )
        .await
        .expect("connect appserver");

        let err = app
            .notify_typed(
                methods::TURN_INTERRUPT,
                TurnInterruptNotifyMissingTurnId {
                    thread_id: "thr".to_owned(),
                },
            )
            .await
            .expect_err("missing turnId must fail");
        assert!(matches!(err, RuntimeError::InvalidConfig(_)));

        app.shutdown().await.expect("shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn notify_typed_accepts_valid_known_params() {
        let temp = TempDir::new("coclai_appserver_notify_typed_valid");
        let cli = write_mock_cli_script(&temp.root);
        let app = AppServer::connect(
            ClientConfig::new()
                .with_cli_bin(cli)
                .with_schema_dir(workspace_schema_dir()),
        )
        .await
        .expect("connect appserver");

        app.notify_typed(
            methods::TURN_INTERRUPT,
            TurnInterruptNotifyParams {
                thread_id: "thr".to_owned(),
                turn_id: "turn".to_owned(),
            },
        )
        .await
        .expect("typed notify");

        app.shutdown().await.expect("shutdown");
    }

    #[test]
    fn method_constants_are_stable() {
        assert_eq!(methods::THREAD_START, "thread/start");
        assert_eq!(methods::TURN_INTERRUPT, "turn/interrupt");
    }
}
