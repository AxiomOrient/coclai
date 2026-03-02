use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::ports::outbound::agent_state_store_port::AgentStateStorePort;

#[derive(Clone, Default)]
pub struct InMemoryAgentStateStore {
    workflow_configs: Arc<Mutex<HashMap<String, Value>>>,
    connection_states: Arc<Mutex<HashMap<String, Value>>>,
}

impl InMemoryAgentStateStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl AgentStateStorePort for InMemoryAgentStateStore {
    fn upsert_workflow_config(&self, workflow_id: &str, config: Value) -> Result<(), String> {
        let mut guard = self
            .workflow_configs
            .lock()
            .map_err(|err| format!("workflow store lock poisoned: {err}"))?;
        guard.insert(workflow_id.to_owned(), config);
        Ok(())
    }

    fn load_workflow_config(&self, workflow_id: &str) -> Result<Option<Value>, String> {
        let guard = self
            .workflow_configs
            .lock()
            .map_err(|err| format!("workflow store lock poisoned: {err}"))?;
        Ok(guard.get(workflow_id).cloned())
    }

    fn workflow_config_count(&self) -> Result<usize, String> {
        let guard = self
            .workflow_configs
            .lock()
            .map_err(|err| format!("workflow store lock poisoned: {err}"))?;
        Ok(guard.len())
    }

    fn upsert_connection_state(&self, connection_id: &str, state: Value) -> Result<(), String> {
        let mut guard = self
            .connection_states
            .lock()
            .map_err(|err| format!("connection store lock poisoned: {err}"))?;
        guard.insert(connection_id.to_owned(), state);
        Ok(())
    }

    fn load_connection_state(&self, connection_id: &str) -> Result<Option<Value>, String> {
        let guard = self
            .connection_states
            .lock()
            .map_err(|err| format!("connection store lock poisoned: {err}"))?;
        Ok(guard.get(connection_id).cloned())
    }

    fn connection_state_count(&self) -> Result<usize, String> {
        let guard = self
            .connection_states
            .lock()
            .map_err(|err| format!("connection store lock poisoned: {err}"))?;
        Ok(guard.len())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::InMemoryAgentStateStore;
    use crate::ports::outbound::agent_state_store_port::AgentStateStorePort;

    #[test]
    fn workflow_config_upsert_and_count() {
        let store = InMemoryAgentStateStore::new();
        store
            .upsert_workflow_config("wf-1", json!({"cwd":"/tmp/a"}))
            .expect("upsert wf-1 should succeed");
        store
            .upsert_workflow_config("wf-2", json!({"cwd":"/tmp/b"}))
            .expect("upsert wf-2 should succeed");

        assert_eq!(store.workflow_config_count().expect("count should load"), 2);
        assert_eq!(
            store
                .load_workflow_config("wf-1")
                .expect("wf-1 should load")
                .expect("wf-1 should exist")["cwd"],
            "/tmp/a"
        );
    }

    #[test]
    fn connection_state_upsert_and_count() {
        let store = InMemoryAgentStateStore::new();
        store
            .upsert_connection_state("default", json!({"connected": true}))
            .expect("upsert default should succeed");

        assert_eq!(
            store.connection_state_count().expect("count should load"),
            1
        );
        assert_eq!(
            store
                .load_connection_state("default")
                .expect("state should load")
                .expect("state should exist")["connected"],
            true
        );
    }
}
