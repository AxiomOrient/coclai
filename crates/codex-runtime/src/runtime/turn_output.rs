use std::collections::HashSet;

use serde_json::Value;

use crate::runtime::events::{extract_text_from_params, Envelope};
use crate::runtime::id::{parse_result_thread_id, parse_result_turn_id};
use crate::runtime::rpc_contract::methods as events;

use std::sync::Arc;

/// Incremental assistant text collector for one turn stream.
/// Keeps explicit state to avoid duplicate text from both delta and completed payloads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssistantTextCollector {
    assistant_item_ids: HashSet<Arc<str>>,
    assistant_items_with_delta: HashSet<Arc<str>>,
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

/// Terminal state of one turn observed from live stream events.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnTerminalEvent {
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

/// Shared turn stream collector engine used by runtime prompt and artifact execution flows.
/// It filters by `(thread_id, turn_id)`, accumulates assistant text, and reports terminal events.
#[derive(Clone, Debug)]
pub struct TurnStreamCollector {
    thread_id: Arc<str>,
    turn_id: Arc<str>,
    matching_turn_events: usize,
    assistant: AssistantTextCollector,
}

impl TurnStreamCollector {
    /// Create collector bound to one target turn.
    pub fn new(thread_id: &str, turn_id: &str) -> Self {
        Self {
            thread_id: Arc::from(thread_id),
            turn_id: Arc::from(turn_id),
            matching_turn_events: 0,
            assistant: AssistantTextCollector::new(),
        }
    }

    /// Consume one envelope. Returns terminal event when this envelope closes the target turn.
    pub fn push_envelope(&mut self, envelope: &Envelope) -> Option<TurnTerminalEvent> {
        if envelope.thread_id.as_deref() != Some(self.thread_id.as_ref())
            || envelope.turn_id.as_deref() != Some(self.turn_id.as_ref())
        {
            return None;
        }

        self.matching_turn_events = self.matching_turn_events.saturating_add(1);
        self.assistant.push_envelope(envelope);

        match envelope.method.as_deref() {
            Some(events::TURN_COMPLETED) => Some(TurnTerminalEvent::Completed),
            Some(events::TURN_FAILED) => Some(TurnTerminalEvent::Failed),
            Some(events::TURN_INTERRUPTED) => Some(TurnTerminalEvent::Interrupted),
            Some(events::TURN_CANCELLED) => Some(TurnTerminalEvent::Cancelled),
            _ => None,
        }
    }

    /// Whether one envelope belongs to this collector turn target.
    pub fn is_target_envelope(&self, envelope: &Envelope) -> bool {
        envelope.thread_id.as_deref() == Some(self.thread_id.as_ref())
            && envelope.turn_id.as_deref() == Some(self.turn_id.as_ref())
    }

    /// Number of consumed envelopes that matched the target turn.
    pub fn matching_turn_events(&self) -> usize {
        self.matching_turn_events
    }

    /// Borrow current collected assistant text.
    pub fn assistant_text(&self) -> &str {
        self.assistant.text()
    }

    /// Take ownership of collected assistant text.
    pub fn into_assistant_text(self) -> String {
        self.assistant.into_text()
    }
}

/// Parse thread id from common JSON-RPC result shapes.
/// Allocation: one String on match. Complexity: O(1).
pub fn parse_thread_id(value: &Value) -> Option<String> {
    parse_result_thread_id(value).map(ToOwned::to_owned)
}

/// Parse turn id from common JSON-RPC result shapes.
/// Allocation: one String on match. Complexity: O(1).
pub fn parse_turn_id(value: &Value) -> Option<String> {
    parse_result_turn_id(value).map(ToOwned::to_owned)
}

fn track_assistant_item(assistant_item_ids: &mut HashSet<Arc<str>>, envelope: &Envelope) {
    if envelope.method.as_deref() != Some(events::ITEM_STARTED) {
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
    assistant_item_ids: &HashSet<Arc<str>>,
    assistant_items_with_delta: &mut HashSet<Arc<str>>,
    envelope: &Envelope,
) {
    let params = envelope.json.get("params");
    match envelope.method.as_deref() {
        Some(events::ITEM_AGENT_MESSAGE_DELTA) => {
            if let Some(delta) = params.and_then(|p| p.get("delta")).and_then(Value::as_str) {
                if let Some(item_id) = envelope.item_id.as_ref() {
                    assistant_items_with_delta.insert(item_id.clone());
                }
                out.push_str(delta);
            }
        }
        Some(events::ITEM_COMPLETED) => {
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
        Some(events::TURN_COMPLETED) => {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::runtime::events::{Direction, MsgKind};

    use super::*;

    fn envelope_for_turn(
        method: &str,
        thread_id: &str,
        turn_id: &str,
        item_id: Option<&str>,
        params: Value,
    ) -> Envelope {
        Envelope {
            seq: 1,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some(Arc::from(method)),
            thread_id: Some(Arc::from(thread_id)),
            turn_id: Some(Arc::from(turn_id)),
            item_id: item_id.map(Arc::from),
            json: Arc::new(json!({"method": method, "params": params})),
        }
    }

    fn envelope(method: &str, item_id: Option<&str>, params: Value) -> Envelope {
        envelope_for_turn(method, "thr", "turn", item_id, params)
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

    #[test]
    fn parse_ids_reject_loose_id_fallback_and_empty_values() {
        assert_eq!(parse_thread_id(&json!({"id":"thr_loose"})), None);
        assert_eq!(parse_turn_id(&json!("turn_loose")), None);
        assert_eq!(parse_thread_id(&json!({"threadId":""})), None);
        assert_eq!(parse_turn_id(&json!({"turn":{"id":"  "}})), None);
    }

    #[test]
    fn turn_stream_collector_ignores_other_turn_and_tracks_target_terminal() {
        let mut stream = TurnStreamCollector::new("thr_target", "turn_target");

        assert_eq!(
            stream.push_envelope(&envelope(
                "turn/completed",
                None,
                json!({"threadId":"thr_other","turnId":"turn_other"}),
            )),
            None
        );
        assert_eq!(stream.matching_turn_events(), 0);

        assert_eq!(
            stream.push_envelope(&envelope_for_turn(
                "turn/completed",
                "thr_target",
                "turn_target",
                None,
                json!({"threadId":"thr_target","turnId":"turn_target"}),
            )),
            Some(TurnTerminalEvent::Completed)
        );
        assert_eq!(stream.matching_turn_events(), 1);
    }

    #[test]
    fn turn_stream_collector_classifies_cancelled_terminal() {
        let mut stream = TurnStreamCollector::new("thr", "turn");

        let terminal = stream.push_envelope(&envelope_for_turn(
            "turn/cancelled",
            "thr",
            "turn",
            None,
            json!({"threadId":"thr","turnId":"turn"}),
        ));

        assert_eq!(terminal, Some(TurnTerminalEvent::Cancelled));
    }
}
