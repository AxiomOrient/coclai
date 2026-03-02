#![allow(dead_code)]

use serde_json::Value;

pub trait ServerRequestPort {
    fn take_requests(&self, connection_id: &str, max_items: usize) -> Result<Vec<Value>, String>;
    fn respond_ok(
        &self,
        connection_id: &str,
        approval_id: &str,
        result: Value,
    ) -> Result<(), String>;
    fn respond_err(&self, connection_id: &str, approval_id: &str, err: Value)
        -> Result<(), String>;
}
