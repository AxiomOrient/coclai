use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

use crate::runtime::api::CommandExecOutputDeltaNotification;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum JsonRpcId {
    Number(u64),
    Text(String),
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    Inbound,
    Outbound,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum MsgKind {
    Response,
    ServerRequest,
    Notification,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Envelope {
    pub seq: u64,
    pub ts_millis: i64,
    pub direction: Direction,
    pub kind: MsgKind,
    pub rpc_id: Option<JsonRpcId>,
    pub method: Option<Arc<str>>,
    pub thread_id: Option<Arc<str>>,
    pub turn_id: Option<Arc<str>>,
    pub item_id: Option<Arc<str>>,
    pub json: Arc<Value>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillsChangedNotification {}

/// Detect the zero-payload `skills/changed` invalidation notification.
/// Allocation: none. Complexity: O(1).
pub fn extract_skills_changed_notification(
    envelope: &Envelope,
) -> Option<SkillsChangedNotification> {
    if envelope.kind == MsgKind::Notification
        && envelope.method.as_deref() == Some(crate::runtime::rpc_contract::methods::SKILLS_CHANGED)
    {
        Some(SkillsChangedNotification {})
    } else {
        None
    }
}

/// Parse one `command/exec/outputDelta` notification into a typed payload.
/// Allocation: one params clone for serde deserialization. Complexity: O(n), n = delta payload size.
pub fn extract_command_exec_output_delta(
    envelope: &Envelope,
) -> Option<CommandExecOutputDeltaNotification> {
    if envelope.kind != MsgKind::Notification
        || envelope.method.as_deref()
            != Some(crate::runtime::rpc_contract::methods::COMMAND_EXEC_OUTPUT_DELTA)
    {
        return None;
    }

    let params = envelope.json.get("params")?.clone();
    serde_json::from_value(params).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detects_skills_changed_notification() {
        let envelope = Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some(Arc::from("skills/changed")),
            thread_id: None,
            turn_id: None,
            item_id: None,
            json: Arc::new(json!({"method":"skills/changed","params":{}})),
        };

        assert_eq!(
            extract_skills_changed_notification(&envelope),
            Some(SkillsChangedNotification {})
        );
    }

    #[test]
    fn rejects_non_skills_changed_notification() {
        let envelope = Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::ServerRequest,
            rpc_id: Some(JsonRpcId::Number(1)),
            method: Some(Arc::from("skills/changed")),
            thread_id: None,
            turn_id: None,
            item_id: None,
            json: Arc::new(json!({"id":1,"method":"skills/changed","params":{}})),
        };

        assert_eq!(extract_skills_changed_notification(&envelope), None);
    }

    #[test]
    fn detects_command_exec_output_delta_notification() {
        let envelope = Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some(Arc::from("command/exec/outputDelta")),
            thread_id: None,
            turn_id: None,
            item_id: None,
            json: Arc::new(json!({
                "method":"command/exec/outputDelta",
                "params":{
                    "processId":"proc-1",
                    "stream":"stdout",
                    "deltaBase64":"aGVsbG8=",
                    "capReached":false
                }
            })),
        };

        let notification =
            extract_command_exec_output_delta(&envelope).expect("typed output delta notification");
        assert_eq!(notification.process_id, "proc-1");
        assert_eq!(notification.delta_base64, "aGVsbG8=");
    }
}
