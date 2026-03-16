use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use tokio::time::Duration;

use crate::runtime::errors::{RpcError, RuntimeError};
use crate::runtime::rpc_contract::{
    validate_rpc_request, validate_rpc_response, RpcValidationMode,
};

use super::rpc_io::{call_raw_inner, notify_raw_inner};
use super::Runtime;

impl Runtime {
    pub async fn call_raw(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_raw_internal(method, params, true, self.inner.spec.rpc_response_timeout)
            .await
    }

    /// JSON-RPC call with contract validation for known methods.
    /// Validation covers request params before send and result shape after receive.
    pub async fn call_validated(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// JSON-RPC call with explicit validation mode.
    pub async fn call_validated_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<Value, RpcError> {
        self.call_validated_with_mode_and_timeout(
            method,
            params,
            mode,
            self.inner.spec.rpc_response_timeout,
        )
        .await
    }

    pub(crate) async fn call_validated_with_mode_and_timeout(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
        timeout_duration: Duration,
    ) -> Result<Value, RpcError> {
        validate_rpc_request(method, &params, mode)?;
        let result = self
            .call_raw_internal(method, params, true, timeout_duration)
            .await?;
        validate_rpc_response(method, &result, mode)?;
        Ok(result)
    }

    #[cfg(test)]
    pub(crate) async fn call_raw_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout_duration: Duration,
    ) -> Result<Value, RpcError> {
        self.call_raw_internal(method, params, true, timeout_duration)
            .await
    }

    /// Typed JSON-RPC call with known-method contract validation.
    pub async fn call_typed_validated<P, R>(&self, method: &str, params: P) -> Result<R, RpcError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        self.call_typed_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// Typed JSON-RPC call with explicit validation mode.
    pub async fn call_typed_validated_with_mode<P, R>(
        &self,
        method: &str,
        params: P,
        mode: RpcValidationMode,
    ) -> Result<R, RpcError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let params_value = serde_json::to_value(params).map_err(|err| {
            RpcError::InvalidRequest(format!(
                "failed to serialize json-rpc params for {method}: {err}"
            ))
        })?;
        let result = self
            .call_validated_with_mode(method, params_value, mode)
            .await?;
        serde_json::from_value(result).map_err(|err| {
            RpcError::InvalidRequest(format!(
                "failed to deserialize json-rpc result for {method}: {err}"
            ))
        })
    }

    pub async fn notify_raw(&self, method: &str, params: Value) -> Result<(), RuntimeError> {
        self.notify_raw_internal(method, params, true).await
    }

    /// JSON-RPC notify with known-method request validation.
    pub async fn notify_validated(&self, method: &str, params: Value) -> Result<(), RuntimeError> {
        self.notify_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// JSON-RPC notify with explicit validation mode.
    pub async fn notify_validated_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<(), RuntimeError> {
        validate_rpc_request(method, &params, mode).map_err(|err| {
            RuntimeError::InvalidConfig(format!("invalid json-rpc notify payload: {err}"))
        })?;
        self.notify_raw_internal(method, params, true).await
    }

    /// Typed JSON-RPC notify with known-method request validation.
    pub async fn notify_typed_validated<P>(
        &self,
        method: &str,
        params: P,
    ) -> Result<(), RuntimeError>
    where
        P: Serialize,
    {
        self.notify_typed_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// Typed JSON-RPC notify with explicit validation mode.
    pub async fn notify_typed_validated_with_mode<P>(
        &self,
        method: &str,
        params: P,
        mode: RpcValidationMode,
    ) -> Result<(), RuntimeError>
    where
        P: Serialize,
    {
        let params_value = serde_json::to_value(params).map_err(|err| {
            RuntimeError::InvalidConfig(format!(
                "invalid json-rpc notify payload: failed to serialize json-rpc params for {method}: {err}"
            ))
        })?;
        self.notify_validated_with_mode(method, params_value, mode)
            .await
    }

    async fn call_raw_internal(
        &self,
        method: &str,
        params: Value,
        require_initialized: bool,
        timeout_duration: Duration,
    ) -> Result<Value, RpcError> {
        if require_initialized && !self.is_initialized() {
            return Err(RpcError::InvalidRequest(
                "runtime is not initialized".to_owned(),
            ));
        }

        call_raw_inner(&self.inner, method, params, timeout_duration).await
    }

    async fn notify_raw_internal(
        &self,
        method: &str,
        params: Value,
        require_initialized: bool,
    ) -> Result<(), RuntimeError> {
        if require_initialized && !self.is_initialized() {
            return Err(RuntimeError::NotInitialized);
        }

        notify_raw_inner(&self.inner, method, params).await
    }
}
