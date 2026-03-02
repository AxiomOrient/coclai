#![allow(dead_code)]

use serde_json::Value;

pub trait AgentStateStorePort {
    fn upsert_workflow_config(&self, workflow_id: &str, config: Value) -> Result<(), String>;
    fn load_workflow_config(&self, workflow_id: &str) -> Result<Option<Value>, String>;
    fn workflow_config_count(&self) -> Result<usize, String>;
    fn upsert_connection_state(&self, connection_id: &str, state: Value) -> Result<(), String>;
    fn load_connection_state(&self, connection_id: &str) -> Result<Option<Value>, String>;
    fn connection_state_count(&self) -> Result<usize, String>;
}
