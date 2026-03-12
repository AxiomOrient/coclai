use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::runtime::approvals::ServerRequest;
use crate::runtime::errors::{RpcErrorObject, RuntimeError};

use super::dispatch::{send_rpc_error, send_rpc_result, validate_server_request_result_payload};
use super::state_projection::state_remove_pending_server_request;
use super::{PendingServerRequestEntry, Runtime};

impl Runtime {
    pub async fn take_server_request_rx(
        &self,
    ) -> Result<mpsc::Receiver<ServerRequest>, RuntimeError> {
        self.inner
            .io
            .server_request_rx
            .lock()
            .await
            .take()
            .ok_or(RuntimeError::ServerRequestReceiverTaken)
    }

    pub async fn respond_approval_ok(
        &self,
        approval_id: &str,
        result: Value,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .take_pending_server_request_entry(approval_id, |entry| {
                validate_server_request_result_payload(&entry.method, &result)
            })
            .await?;
        send_rpc_result(&self.inner, &entry.rpc_id, result).await
    }

    pub async fn respond_approval_err(
        &self,
        approval_id: &str,
        err: RpcErrorObject,
    ) -> Result<(), RuntimeError> {
        let entry = self
            .take_pending_server_request_entry(approval_id, |_| Ok(()))
            .await?;
        send_rpc_error(
            &self.inner,
            &entry.rpc_id,
            json!({
                "code": err.code,
                "message": err.message,
                "data": err.data
            }),
        )
        .await
    }

    async fn take_pending_server_request_entry<F>(
        &self,
        approval_id: &str,
        validate: F,
    ) -> Result<PendingServerRequestEntry, RuntimeError>
    where
        F: FnOnce(&PendingServerRequestEntry) -> Result<(), RuntimeError>,
    {
        let entry = {
            let mut guard = self.inner.io.pending_server_requests.lock().await;
            let entry = guard
                .get(approval_id)
                .cloned()
                .ok_or_else(|| approval_not_found_error(approval_id))?;
            validate(&entry)?;
            guard
                .remove(approval_id)
                .ok_or_else(|| approval_not_found_error(approval_id))?
        };
        self.inner.metrics.dec_pending_server_request();
        state_remove_pending_server_request(&self.inner, &entry.rpc_key);
        Ok(entry)
    }
}

fn approval_not_found_error(approval_id: &str) -> RuntimeError {
    RuntimeError::Internal(format!("approval id not found: {approval_id}"))
}
