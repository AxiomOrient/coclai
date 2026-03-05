use std::sync::Arc;

use crate::plugin::PluginContractVersion;
use crate::runtime::core::Runtime;

mod adapter;
mod execution;
mod models;
mod store;

#[cfg(test)]
pub(crate) use adapter::{ArtifactAdapterFuture, ArtifactTurnOutput};
pub use adapter::{ArtifactPluginAdapter, RuntimeArtifactAdapter};

#[cfg(test)]
pub(crate) use models::DocEdit;
pub use models::{
    apply_doc_patch, compute_revision, validate_doc_patch, ArtifactMeta, ArtifactSession,
    ArtifactStore, ArtifactTaskKind, ArtifactTaskResult, ArtifactTaskSpec, DocPatch, DomainError,
    FsArtifactStore, PatchConflict, SaveMeta, StoreErr, ValidatedPatch,
};

#[cfg(test)]
pub(crate) use execution::collect_turn_output_from_live_with_limits;
#[cfg(test)]
pub(crate) use execution::{build_turn_prompt, build_turn_start_params};
#[cfg(test)]
pub(crate) use store::artifact_key;

#[derive(Clone)]
pub struct ArtifactSessionManager {
    adapter: Arc<dyn ArtifactPluginAdapter>,
    store: Arc<dyn ArtifactStore>,
    contract_mismatch: Option<ContractMismatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContractMismatch {
    expected: PluginContractVersion,
    actual: PluginContractVersion,
}

impl ArtifactSessionManager {
    pub fn new(runtime: Runtime, store: Arc<dyn ArtifactStore>) -> Self {
        let adapter: Arc<dyn ArtifactPluginAdapter> =
            Arc::new(RuntimeArtifactAdapter::new(runtime));
        Self::new_with_adapter(adapter, store)
    }

    pub fn new_with_adapter(
        adapter: Arc<dyn ArtifactPluginAdapter>,
        store: Arc<dyn ArtifactStore>,
    ) -> Self {
        let contract_mismatch = detect_contract_mismatch(adapter.as_ref());
        Self {
            adapter,
            store,
            contract_mismatch,
        }
    }

    async fn store_io<T: Send + 'static>(
        &self,
        op: impl FnOnce(&dyn ArtifactStore) -> Result<T, StoreErr> + Send + 'static,
    ) -> Result<T, DomainError> {
        let store = Arc::clone(&self.store);
        let joined = tokio::task::spawn_blocking(move || op(store.as_ref()))
            .await
            .map_err(|err| {
                DomainError::Store(StoreErr::Io(format!("store worker join failed: {err}")))
            })?;
        joined.map_err(DomainError::from)
    }

    pub async fn open(&self, artifact_id: &str) -> Result<ArtifactSession, DomainError> {
        self.ensure_contract_compatible()?;

        let artifact_id_owned = artifact_id.to_owned();
        let mut meta = self
            .store_io({
                let artifact_id = artifact_id_owned.clone();
                move |store| execution::load_or_default_meta(store, &artifact_id)
            })
            .await?;

        let thread_id = match meta.runtime_thread_id.as_deref() {
            Some(existing) => self.adapter.resume_thread(existing).await?,
            None => self.adapter.start_thread().await?,
        };
        meta.runtime_thread_id = Some(thread_id.clone());
        self.store_io({
            let artifact_id = artifact_id_owned.clone();
            let meta_to_store = meta.clone();
            move |store| store.set_meta(&artifact_id, meta_to_store)
        })
        .await?;

        Ok(ArtifactSession {
            artifact_id: artifact_id_owned,
            thread_id,
            format: meta.format,
            revision: meta.revision,
        })
    }

    /// Domain task runner with explicit side-effect boundary:
    /// runtime RPC call + store read/write.
    /// Allocation: prompt string + output JSON parse structures.
    /// Complexity: O(L + e) for DocEdit (L=text size, e=edit count).
    pub async fn run_task(
        &self,
        spec: ArtifactTaskSpec,
    ) -> Result<ArtifactTaskResult, DomainError> {
        execution::run_task(self, spec).await
    }

    fn ensure_contract_compatible(&self) -> Result<(), DomainError> {
        if let Some(mismatch) = self.contract_mismatch {
            return Err(DomainError::IncompatibleContract {
                expected_major: mismatch.expected.major,
                expected_minor: mismatch.expected.minor,
                actual_major: mismatch.actual.major,
                actual_minor: mismatch.actual.minor,
            });
        }
        Ok(())
    }
}

fn detect_contract_mismatch(adapter: &dyn ArtifactPluginAdapter) -> Option<ContractMismatch> {
    let expected = PluginContractVersion::CURRENT;
    let actual = adapter.plugin_contract_version();
    if expected.is_compatible_with(actual) {
        None
    } else {
        Some(ContractMismatch { expected, actual })
    }
}
#[cfg(test)]
mod tests;
