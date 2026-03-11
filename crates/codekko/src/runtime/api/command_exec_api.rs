use crate::runtime::core::Runtime;
use crate::runtime::errors::RpcError;
use crate::runtime::rpc_contract::methods;

use super::wire::{command_exec_params_to_wire, deserialize_result, serialize_params};
use super::*;

impl Runtime {
    /// Run one standalone command in the app-server sandbox.
    /// Allocation: JSON params + decoded response payload.
    /// Complexity: O(n), n = argv/env size + buffered output size.
    pub async fn command_exec(
        &self,
        p: CommandExecParams,
    ) -> Result<CommandExecResponse, RpcError> {
        let response = self
            .call_validated(methods::COMMAND_EXEC, command_exec_params_to_wire(&p))
            .await?;
        deserialize_result(methods::COMMAND_EXEC, response)
    }

    /// Write stdin bytes to a running standalone command or close stdin.
    /// Allocation: serialized params + empty response object.
    /// Complexity: O(n), n = payload size.
    pub async fn command_exec_write(
        &self,
        p: CommandExecWriteParams,
    ) -> Result<CommandExecWriteResponse, RpcError> {
        let params = serialize_params(methods::COMMAND_EXEC_WRITE, &p)?;
        let response = self
            .call_validated(methods::COMMAND_EXEC_WRITE, params)
            .await?;
        deserialize_result(methods::COMMAND_EXEC_WRITE, response)
    }

    /// Resize one PTY-backed standalone command by client process id.
    /// Allocation: serialized params + empty response object.
    /// Complexity: O(1).
    pub async fn command_exec_resize(
        &self,
        p: CommandExecResizeParams,
    ) -> Result<CommandExecResizeResponse, RpcError> {
        let params = serialize_params(methods::COMMAND_EXEC_RESIZE, &p)?;
        let response = self
            .call_validated(methods::COMMAND_EXEC_RESIZE, params)
            .await?;
        deserialize_result(methods::COMMAND_EXEC_RESIZE, response)
    }

    /// Terminate one standalone command by client process id.
    /// Allocation: serialized params + empty response object.
    /// Complexity: O(1).
    pub async fn command_exec_terminate(
        &self,
        p: CommandExecTerminateParams,
    ) -> Result<CommandExecTerminateResponse, RpcError> {
        let params = serialize_params(methods::COMMAND_EXEC_TERMINATE, &p)?;
        let response = self
            .call_validated(methods::COMMAND_EXEC_TERMINATE, params)
            .await?;
        deserialize_result(methods::COMMAND_EXEC_TERMINATE, response)
    }
}
