use std::collections::HashSet;

use serde_json::Value;

use crate::events::Envelope;

/// Incremental assistant text collector for one turn stream.
/// Keeps explicit state to avoid duplicate text from both delta and completed payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssistantTextCollector {
    assistant_item_ids: HashSet<String>,
    assistant_items_with_delta: HashSet<String>,
    text: String,
}

impl AssistantTextCollector {
    /// Create empty collector.
    /// Allocation: none. Complexity: O(1).
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one envelope and update internal text state.
    /// Allocation: O(delta) for appended text and newly seen item ids.
    /// Complexity: O(1).
    pub fn push_envelope(&mut self, envelope: &Envelope) {
        track_assistant_item(&mut self.assistant_item_ids, envelope);
        append_text_from_envelope(
            &mut self.text,
            &self.assistant_item_ids,
            &mut self.assistant_items_with_delta,
            envelope,
        );
    }

    /// Borrow collected raw text.
    /// Allocation: none. Complexity: O(1).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Take ownership of collected raw text.
    /// Allocation: none. Complexity: O(1).
    pub fn into_text(self) -> String {
        self.text
    }
}

/// Parse thread id from common JSON-RPC result shapes.
/// Allocation: one String on match. Complexity: O(1).
pub fn parse_thread_id(value: &Value) -> Option<String> {
    value
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("threadId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| value.as_str().map(ToOwned::to_owned))
}

/// Parse turn id from common JSON-RPC result shapes.
/// Allocation: one String on match. Complexity: O(1).
pub fn parse_turn_id(value: &Value) -> Option<String> {
    value
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("turnId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| value.as_str().map(ToOwned::to_owned))
}

fn track_assistant_item(assistant_item_ids: &mut HashSet<String>, envelope: &Envelope) {
    if envelope.method.as_deref() != Some("item/started") {
        return;
    }

    let params = envelope.json.get("params");
    let item_type = params
        .and_then(|p| p.get("itemType"))
        .and_then(Value::as_str)
        .unwrap_or("");
    if item_type != "agentMessage" && item_type != "agent_message" {
        return;
    }
    if let Some(item_id) = envelope.item_id.as_ref() {
        assistant_item_ids.insert(item_id.clone());
    }
}

fn append_text_from_envelope(
    out: &mut String,
    assistant_item_ids: &HashSet<String>,
    assistant_items_with_delta: &mut HashSet<String>,
    envelope: &Envelope,
) {
    let params = envelope.json.get("params");
    match envelope.method.as_deref() {
        Some("item/agentMessage/delta") => {
            if let Some(delta) = params.and_then(|p| p.get("delta")).and_then(Value::as_str) {
                if let Some(item_id) = envelope.item_id.as_ref() {
                    assistant_items_with_delta.insert(item_id.clone());
                }
                out.push_str(delta);
            }
        }
        Some("item/completed") => {
            let is_assistant_item = envelope
                .item_id
                .as_ref()
                .map(|id| assistant_item_ids.contains(id))
                .unwrap_or(false)
                || params
                    .and_then(|p| p.get("item"))
                    .and_then(|v| v.get("type"))
                    .and_then(Value::as_str)
                    .map(|t| t == "agent_message" || t == "agentMessage")
                    .unwrap_or(false);
            if !is_assistant_item {
                return;
            }
            if envelope
                .item_id
                .as_ref()
                .map(|id| assistant_items_with_delta.contains(id))
                .unwrap_or(false)
            {
                return;
            }

            if let Some(text) = params.and_then(extract_text_from_params) {
                if !text.is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&text);
                }
            }
        }
        Some("turn/completed") => {
            if let Some(text) = params.and_then(extract_text_from_params) {
                merge_turn_completed_text(out, &text);
            }
        }
        _ => {}
    }
}

fn merge_turn_completed_text(out: &mut String, text: &str) {
    if text.is_empty() {
        return;
    }
    if out.is_empty() {
        out.push_str(text);
        return;
    }
    if out == text {
        return;
    }
    // If turn/completed includes the full final text and we only collected a prefix
    // from deltas, promote to the complete payload instead of duplicating.
    if text.starts_with(out.as_str()) {
        out.clear();
        out.push_str(text);
        return;
    }
    if out.ends_with(text) {
        return;
    }
    out.push('\n');
    out.push_str(text);
}

fn extract_text_from_params(params: &Value) -> Option<String> {
    if let Some(text) = params
        .get("item")
        .and_then(|i| i.get("text"))
        .and_then(Value::as_str)
    {
        return Some(text.to_owned());
    }
    if let Some(text) = params.get("text").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(text) = params.get("outputText").and_then(Value::as_str) {
        return Some(text.to_owned());
    }
    if let Some(text) = params
        .get("output")
        .and_then(|o| o.get("text"))
        .and_then(Value::as_str)
    {
        return Some(text.to_owned());
    }
    if let Some(content) = params
        .get("item")
        .and_then(|item| item.get("content"))
        .and_then(Value::as_array)
    {
        let mut joined = String::new();
        for part in content {
            if let Some(text) = part.get("text").and_then(Value::as_str) {
                joined.push_str(text);
            }
        }
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::events::{Direction, MsgKind};

    use super::*;

    fn envelope(method: &str, item_id: Option<&str>, params: Value) -> Envelope {
        Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some(method.to_owned()),
            thread_id: Some("thr".to_owned()),
            turn_id: Some("turn".to_owned()),
            item_id: item_id.map(ToOwned::to_owned),
            json: json!({"method": method, "params": params}),
        }
    }

    #[test]
    fn collector_prefers_delta_and_ignores_completed_duplicate() {
        let mut collector = AssistantTextCollector::new();
        collector.push_envelope(&envelope(
            "item/started",
            Some("it_1"),
            json!({"itemType":"agentMessage"}),
        ));
        collector.push_envelope(&envelope(
            "item/agentMessage/delta",
            Some("it_1"),
            json!({"delta":"hello"}),
        ));
        collector.push_envelope(&envelope(
            "item/completed",
            Some("it_1"),
            json!({"item":{"type":"agent_message","text":"hello"}}),
        ));
        assert_eq!(collector.text(), "hello");
    }

    #[test]
    fn collector_reads_completed_text_without_delta() {
        let mut collector = AssistantTextCollector::new();
        collector.push_envelope(&envelope(
            "item/started",
            Some("it_2"),
            json!({"itemType":"agent_message"}),
        ));
        collector.push_envelope(&envelope(
            "item/completed",
            Some("it_2"),
            json!({"item":{"type":"agent_message","text":"world"}}),
        ));
        assert_eq!(collector.text(), "world");
    }

    #[test]
    fn collector_dedups_turn_completed_text_after_item_completed() {
        let mut collector = AssistantTextCollector::new();
        collector.push_envelope(&envelope(
            "item/started",
            Some("it_3"),
            json!({"itemType":"agent_message"}),
        ));
        collector.push_envelope(&envelope(
            "item/completed",
            Some("it_3"),
            json!({"item":{"type":"agent_message","text":"final answer"}}),
        ));
        collector.push_envelope(&envelope(
            "turn/completed",
            None,
            json!({"text":"final answer"}),
        ));
        assert_eq!(collector.text(), "final answer");
    }

    #[test]
    fn parse_ids_from_result_shapes() {
        let v = json!({"thread":{"id":"thr_1"},"turn":{"id":"turn_1"}});
        assert_eq!(parse_thread_id(&v).as_deref(), Some("thr_1"));
        assert_eq!(parse_turn_id(&v).as_deref(), Some("turn_1"));
    }
}
