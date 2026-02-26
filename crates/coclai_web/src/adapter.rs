use std::future::Future;
use std::pin::Pin;

use coclai_runtime::api::ThreadStartParams;
use coclai_runtime::approvals::ServerRequest;
use coclai_runtime::errors::RuntimeError;
use coclai_runtime::events::Envelope;
use coclai_runtime::runtime::Runtime;
use coclai_runtime::PluginContractVersion;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use super::WebError;

pub type WebAdapterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug)]
pub struct WebRuntimeStreams {
    pub request_rx: mpsc::Receiver<ServerRequest>,
    pub live_rx: broadcast::Receiver<Envelope>,
}

pub trait WebPluginAdapter: Send + Sync {
    fn plugin_contract_version(&self) -> PluginContractVersion {
        PluginContractVersion::CURRENT
    }

    fn take_streams<'a>(&'a self) -> WebAdapterFuture<'a, Result<WebRuntimeStreams, WebError>>;
    fn thread_start<'a>(
        &'a self,
        params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>>;
    fn thread_resume<'a>(
        &'a self,
        thread_id: &'a str,
        params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>>;
    fn turn_start<'a>(
        &'a self,
        turn_params: Value,
    ) -> WebAdapterFuture<'a, Result<Value, WebError>>;
    fn thread_archive<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> WebAdapterFuture<'a, Result<(), WebError>>;
    fn respond_approval_ok<'a>(
        &'a self,
        approval_id: &'a str,
        result: Value,
    ) -> WebAdapterFuture<'a, Result<(), WebError>>;
    fn pending_approval_ids(&self) -> Vec<String>;
}

#[derive(Clone)]
pub struct RuntimeWebAdapter {
    runtime: Runtime,
}

impl RuntimeWebAdapter {
    /// Create runtime-backed web adapter.
    /// Allocation: none. Complexity: O(1).
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }
}

impl WebPluginAdapter for RuntimeWebAdapter {
    fn take_streams<'a>(&'a self) -> WebAdapterFuture<'a, Result<WebRuntimeStreams, WebError>> {
        Box::pin(async move {
            let request_rx = self
                .runtime
                .take_server_request_rx()
                .await
                .map_err(map_take_stream_error)?;
            let live_rx = self.runtime.subscribe_live();
            Ok(WebRuntimeStreams {
                request_rx,
                live_rx,
            })
        })
    }

    fn thread_start<'a>(
        &'a self,
        params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move {
            let thread = self
                .runtime
                .thread_start(params)
                .await
                .map_err(map_rpc_error)?;
            Ok(thread.thread_id)
        })
    }

    fn thread_resume<'a>(
        &'a self,
        thread_id: &'a str,
        params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move {
            let thread = self
                .runtime
                .thread_resume(thread_id, params)
                .await
                .map_err(map_rpc_error)?;
            Ok(thread.thread_id)
        })
    }

    fn turn_start<'a>(
        &'a self,
        turn_params: Value,
    ) -> WebAdapterFuture<'a, Result<Value, WebError>> {
        Box::pin(async move {
            self.runtime
                .call_raw("turn/start", turn_params)
                .await
                .map_err(map_rpc_error)
        })
    }

    fn thread_archive<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(async move {
            self.runtime
                .thread_archive(thread_id)
                .await
                .map_err(map_rpc_error)
        })
    }

    fn respond_approval_ok<'a>(
        &'a self,
        approval_id: &'a str,
        result: Value,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(async move {
            self.runtime
                .respond_approval_ok(approval_id, result)
                .await
                .map_err(map_runtime_error)
        })
    }

    fn pending_approval_ids(&self) -> Vec<String> {
        self.runtime
            .state_snapshot()
            .pending_server_requests
            .values()
            .map(|req| req.approval_id.clone())
            .collect()
    }
}

fn map_take_stream_error(err: RuntimeError) -> WebError {
    match err {
        RuntimeError::ServerRequestReceiverTaken => WebError::AlreadyBound,
        other => map_runtime_error(other),
    }
}

fn map_runtime_error(err: impl std::fmt::Display) -> WebError {
    WebError::Internal(format!("runtime error: {err}"))
}

fn map_rpc_error(err: impl std::fmt::Display) -> WebError {
    WebError::Internal(format!("rpc error: {err}"))
}
