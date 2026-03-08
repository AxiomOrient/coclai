use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

use crate::runtime::{
    Client, ClientConfig, ClientError, CommandExecParams, CommandExecResizeParams,
    CommandExecResizeResponse, CommandExecResponse, CommandExecTerminateParams,
    CommandExecTerminateResponse, CommandExecWriteParams, CommandExecWriteResponse, RpcError,
    RpcErrorObject, RpcValidationMode, Runtime, RuntimeError, ServerRequestRx, SkillsListParams,
    SkillsListResponse,
};

mod service;

/// Canonical app-server JSON-RPC method names.
pub mod methods {
    pub use crate::runtime::rpc_contract::methods::{
        COMMAND_EXEC, COMMAND_EXEC_OUTPUT_DELTA, COMMAND_EXEC_RESIZE, COMMAND_EXEC_TERMINATE,
        COMMAND_EXEC_WRITE, SKILLS_CHANGED, SKILLS_LIST, THREAD_ARCHIVE, THREAD_FORK, THREAD_LIST,
        THREAD_LOADED_LIST, THREAD_READ, THREAD_RESUME, THREAD_ROLLBACK, THREAD_START,
        TURN_CANCELLED, TURN_COMPLETED, TURN_FAILED, TURN_INTERRUPT, TURN_START,
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
    fn from_client(client: Client) -> Self {
        Self { client }
    }

    /// Connect app-server with explicit config.
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError> {
        let client = service::connect(config).await?;
        Ok(Self::from_client(client))
    }

    /// Connect app-server with default runtime discovery.
    pub async fn connect_default() -> Result<Self, ClientError> {
        let client = service::connect_default().await?;
        Ok(Self::from_client(client))
    }

    /// Validated JSON-RPC request for known methods.
    pub async fn request_json(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.request_json_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// JSON-RPC request with explicit validation mode.
    pub async fn request_json_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<Value, RpcError> {
        service::request_json(&self.client, method, params, mode).await
    }

    /// Typed JSON-RPC request for known methods.
    pub async fn request_typed<P, R>(&self, method: &str, params: P) -> Result<R, RpcError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        self.request_typed_with_mode(method, params, RpcValidationMode::KnownMethods)
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
        service::request_typed(&self.client, method, params, mode).await
    }

    /// Unchecked JSON-RPC request.
    /// Use for experimental/custom methods where strict contracts are not fixed yet.
    pub async fn request_json_unchecked(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcError> {
        service::request_json_unchecked(&self.client, method, params).await
    }

    /// Typed helper for `skills/list`.
    pub async fn skills_list(
        &self,
        params: SkillsListParams,
    ) -> Result<SkillsListResponse, RpcError> {
        self.client.runtime().skills_list(params).await
    }

    /// Typed helper for `command/exec`.
    pub async fn command_exec(
        &self,
        params: CommandExecParams,
    ) -> Result<CommandExecResponse, RpcError> {
        self.client.runtime().command_exec(params).await
    }

    /// Typed helper for `command/exec/write`.
    pub async fn command_exec_write(
        &self,
        params: CommandExecWriteParams,
    ) -> Result<CommandExecWriteResponse, RpcError> {
        self.client.runtime().command_exec_write(params).await
    }

    /// Typed helper for `command/exec/resize`.
    pub async fn command_exec_resize(
        &self,
        params: CommandExecResizeParams,
    ) -> Result<CommandExecResizeResponse, RpcError> {
        self.client.runtime().command_exec_resize(params).await
    }

    /// Typed helper for `command/exec/terminate`.
    pub async fn command_exec_terminate(
        &self,
        params: CommandExecTerminateParams,
    ) -> Result<CommandExecTerminateResponse, RpcError> {
        self.client.runtime().command_exec_terminate(params).await
    }

    /// Validated JSON-RPC notification for known methods.
    pub async fn notify_json(&self, method: &str, params: Value) -> Result<(), RuntimeError> {
        self.notify_json_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// JSON-RPC notification with explicit validation mode.
    pub async fn notify_json_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<(), RuntimeError> {
        service::notify_json(&self.client, method, params, mode).await
    }

    /// Typed JSON-RPC notification for known methods.
    pub async fn notify_typed<P>(&self, method: &str, params: P) -> Result<(), RuntimeError>
    where
        P: Serialize,
    {
        self.notify_typed_with_mode(method, params, RpcValidationMode::KnownMethods)
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
        service::notify_typed(&self.client, method, params, mode).await
    }

    /// Unchecked JSON-RPC notification.
    pub async fn notify_json_unchecked(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(), RuntimeError> {
        service::notify_json_unchecked(&self.client, method, params).await
    }

    /// Take exclusive server-request stream receiver.
    ///
    /// This enables explicit handling of approval / requestUserInput / tool-call cycles.
    pub async fn take_server_requests(&self) -> Result<ServerRequestRx, RuntimeError> {
        service::take_server_requests(&self.client).await
    }

    /// Reply success payload for one server request.
    pub async fn respond_server_request_ok(
        &self,
        approval_id: &str,
        result: Value,
    ) -> Result<(), RuntimeError> {
        service::respond_server_request_ok(&self.client, approval_id, result).await
    }

    /// Reply error payload for one server request.
    pub async fn respond_server_request_err(
        &self,
        approval_id: &str,
        err: RpcErrorObject,
    ) -> Result<(), RuntimeError> {
        service::respond_server_request_err(&self.client, approval_id, err).await
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
        service::shutdown(&self.client).await
    }
}

#[cfg(test)]
mod tests;
