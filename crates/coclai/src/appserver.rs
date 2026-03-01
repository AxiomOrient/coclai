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
mod tests;
