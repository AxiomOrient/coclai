use std::future::Future;
use std::pin::Pin;

use coclai_runtime::runtime::Runtime;
use coclai_runtime::PluginContractVersion;
use serde_json::Value;

use super::task;
use super::{ArtifactTaskSpec, DomainError};

pub type ArtifactAdapterFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq)]
pub struct ArtifactTurnOutput {
    pub turn_id: Option<String>,
    pub output: Value,
}

pub trait ArtifactPluginAdapter: Send + Sync {
    fn plugin_contract_version(&self) -> PluginContractVersion {
        PluginContractVersion::CURRENT
    }

    fn start_thread<'a>(&'a self) -> ArtifactAdapterFuture<'a, Result<String, DomainError>>;
    fn resume_thread<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> ArtifactAdapterFuture<'a, Result<String, DomainError>>;
    fn run_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
        spec: &'a ArtifactTaskSpec,
    ) -> ArtifactAdapterFuture<'a, Result<ArtifactTurnOutput, DomainError>>;
}

#[derive(Clone)]
pub struct RuntimeArtifactAdapter {
    runtime: Runtime,
}

impl RuntimeArtifactAdapter {
    /// Create runtime-backed artifact adapter.
    /// Allocation: none. Complexity: O(1).
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
    }
}

impl ArtifactPluginAdapter for RuntimeArtifactAdapter {
    fn start_thread<'a>(&'a self) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move { task::start_thread(&self.runtime).await })
    }

    fn resume_thread<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move { task::resume_thread(&self.runtime, thread_id).await })
    }

    fn run_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
        spec: &'a ArtifactTaskSpec,
    ) -> ArtifactAdapterFuture<'a, Result<ArtifactTurnOutput, DomainError>> {
        Box::pin(async move {
            let turn_params = task::build_turn_start_params(thread_id, prompt, spec);
            let (turn_id, output) =
                task::run_turn_and_collect_output(&self.runtime, thread_id, turn_params).await?;
            Ok(ArtifactTurnOutput { turn_id, output })
        })
    }
}
