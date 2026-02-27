use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::approvals::PendingServerRequest;
use crate::events::Envelope;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionState {
    Starting,
    Handshaking,
    Running { generation: u64 },
    Restarting { generation: u64 },
    ShuttingDown,
    Dead,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeState {
    pub connection: ConnectionState,
    pub threads: HashMap<String, ThreadState>,
    pub pending_server_requests: HashMap<String, PendingServerRequest>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct StateProjectionLimits {
    pub max_threads: usize,
    pub max_turns_per_thread: usize,
    pub max_items_per_turn: usize,
    pub max_text_bytes_per_item: usize,
    pub max_stdout_bytes_per_item: usize,
    pub max_stderr_bytes_per_item: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadState {
    pub id: String,
    pub active_turn: Option<String>,
    pub turns: HashMap<String, TurnState>,
    pub last_diff: Option<String>,
    pub plan: Option<Value>,
    pub last_seq: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TurnStatus {
    InProgress,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnState {
    pub id: String,
    pub status: TurnStatus,
    pub items: HashMap<String, ItemState>,
    pub error: Option<Value>,
    pub last_seq: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ItemState {
    pub id: String,
    pub item_type: String,
    pub started: Option<Value>,
    pub completed: Option<Value>,
    pub text_accum: String,
    pub stdout_accum: String,
    pub stderr_accum: String,
    pub text_truncated: bool,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub last_seq: u64,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            connection: ConnectionState::Starting,
            threads: HashMap::new(),
            pending_server_requests: HashMap::new(),
        }
    }
}

impl Default for StateProjectionLimits {
    fn default() -> Self {
        Self {
            max_threads: 256,
            max_turns_per_thread: 256,
            max_items_per_turn: 256,
            max_text_bytes_per_item: 256 * 1024,
            max_stdout_bytes_per_item: 256 * 1024,
            max_stderr_bytes_per_item: 256 * 1024,
        }
    }
}

/// Pure reducer: consumes old state + envelope and returns next state.
/// Delegates to `reduce_in_place_with_limits` with default retention limits.
pub fn reduce(mut state: RuntimeState, envelope: &Envelope) -> RuntimeState {
    reduce_in_place_with_limits(&mut state, envelope, &StateProjectionLimits::default());
    state
}

/// In-place reducer used by runtime projection.
/// Delegates to `reduce_in_place_with_limits` with default retention limits.
pub fn reduce_in_place(state: &mut RuntimeState, envelope: &Envelope) {
    reduce_in_place_with_limits(state, envelope, &StateProjectionLimits::default());
}

/// In-place reducer with explicit retention bounds for long-running runtimes.
/// Allocation: new map entries + appended deltas; prune candidate vectors are allocated only
/// when a cap is exceeded.
/// Complexity: O(1) average map work per event, plus O(t) touched-thread item-cap checks
/// (t <= max_turns_per_thread after pruning), and O(n log n) only when sorting eviction
/// candidates for thread/turn/item pruning.
pub fn reduce_in_place_with_limits(
    state: &mut RuntimeState,
    envelope: &Envelope,
    limits: &StateProjectionLimits,
) {
    let Some(method) = envelope.method.as_deref() else {
        return;
    };
    let seq = envelope.seq;
    let touched_thread_id = envelope.thread_id.as_deref();

    match method {
        "thread/started" => {
            let Some(thread_id) = envelope.thread_id.as_deref() else {
                return;
            };
            thread_mut(state, thread_id, seq);
        }
        "turn/started" => {
            let (Some(thread_id), Some(turn_id)) =
                (envelope.thread_id.as_deref(), envelope.turn_id.as_deref())
            else {
                return;
            };
            let thread = thread_mut(state, thread_id, seq);
            thread.active_turn = Some(turn_id.to_owned());
            let turn = turn_mut(thread, turn_id, seq);
            turn.status = TurnStatus::InProgress;
        }
        "turn/completed" => {
            let (Some(thread_id), Some(turn_id)) =
                (envelope.thread_id.as_deref(), envelope.turn_id.as_deref())
            else {
                return;
            };
            let thread = thread_mut(state, thread_id, seq);
            if thread.active_turn.as_deref() == Some(turn_id) {
                thread.active_turn = None;
            }
            let turn = turn_mut(thread, turn_id, seq);
            turn.status = TurnStatus::Completed;
        }
        "turn/failed" => {
            let (Some(thread_id), Some(turn_id)) =
                (envelope.thread_id.as_deref(), envelope.turn_id.as_deref())
            else {
                return;
            };
            let thread = thread_mut(state, thread_id, seq);
            if thread.active_turn.as_deref() == Some(turn_id) {
                thread.active_turn = None;
            }
            let turn = turn_mut(thread, turn_id, seq);
            turn.status = TurnStatus::Failed;
            turn.error = envelope
                .json
                .get("params")
                .and_then(|p| p.get("error"))
                .cloned();
        }
        "turn/interrupted" => {
            let (Some(thread_id), Some(turn_id)) =
                (envelope.thread_id.as_deref(), envelope.turn_id.as_deref())
            else {
                return;
            };
            let thread = thread_mut(state, thread_id, seq);
            if thread.active_turn.as_deref() == Some(turn_id) {
                thread.active_turn = None;
            }
            let turn = turn_mut(thread, turn_id, seq);
            turn.status = TurnStatus::Interrupted;
        }
        "turn/diff/updated" => {
            let Some(thread_id) = envelope.thread_id.as_deref() else {
                return;
            };
            let thread = thread_mut(state, thread_id, seq);
            thread.last_diff = envelope
                .json
                .get("params")
                .and_then(|p| p.get("diff"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
        }
        "turn/plan/updated" => {
            let Some(thread_id) = envelope.thread_id.as_deref() else {
                return;
            };
            let thread = thread_mut(state, thread_id, seq);
            thread.plan = envelope
                .json
                .get("params")
                .and_then(|p| p.get("plan"))
                .cloned();
        }
        "item/started" => {
            let (Some(thread_id), Some(turn_id), Some(item_id)) = (
                envelope.thread_id.as_deref(),
                envelope.turn_id.as_deref(),
                envelope.item_id.as_deref(),
            ) else {
                return;
            };

            let thread = thread_mut(state, thread_id, seq);
            let turn = turn_mut(thread, turn_id, seq);
            let item = item_mut(turn, item_id, seq);
            item.started = envelope.json.get("params").cloned();
            item.item_type = envelope
                .json
                .get("params")
                .and_then(|p| p.get("itemType"))
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();
        }
        "item/agentMessage/delta" => {
            let (Some(thread_id), Some(turn_id), Some(item_id)) = (
                envelope.thread_id.as_deref(),
                envelope.turn_id.as_deref(),
                envelope.item_id.as_deref(),
            ) else {
                return;
            };

            let delta = envelope
                .json
                .get("params")
                .and_then(|p| p.get("delta"))
                .and_then(Value::as_str)
                .unwrap_or("");

            let thread = thread_mut(state, thread_id, seq);
            let turn = turn_mut(thread, turn_id, seq);
            let item = item_mut(turn, item_id, seq);
            append_capped(
                &mut item.text_accum,
                delta,
                limits.max_text_bytes_per_item,
                &mut item.text_truncated,
            );
        }
        "item/commandExecution/outputDelta" => {
            let (Some(thread_id), Some(turn_id), Some(item_id)) = (
                envelope.thread_id.as_deref(),
                envelope.turn_id.as_deref(),
                envelope.item_id.as_deref(),
            ) else {
                return;
            };

            let stdout = envelope
                .json
                .get("params")
                .and_then(|p| p.get("stdout"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let stderr = envelope
                .json
                .get("params")
                .and_then(|p| p.get("stderr"))
                .and_then(Value::as_str)
                .unwrap_or("");

            let thread = thread_mut(state, thread_id, seq);
            let turn = turn_mut(thread, turn_id, seq);
            let item = item_mut(turn, item_id, seq);
            append_capped(
                &mut item.stdout_accum,
                stdout,
                limits.max_stdout_bytes_per_item,
                &mut item.stdout_truncated,
            );
            append_capped(
                &mut item.stderr_accum,
                stderr,
                limits.max_stderr_bytes_per_item,
                &mut item.stderr_truncated,
            );
        }
        "item/completed" => {
            let (Some(thread_id), Some(turn_id), Some(item_id)) = (
                envelope.thread_id.as_deref(),
                envelope.turn_id.as_deref(),
                envelope.item_id.as_deref(),
            ) else {
                return;
            };

            let thread = thread_mut(state, thread_id, seq);
            let turn = turn_mut(thread, turn_id, seq);
            let item = item_mut(turn, item_id, seq);
            item.completed = envelope.json.get("params").cloned();
        }
        _ => {}
    }

    prune_state(state, limits, touched_thread_id);
}

fn thread_mut<'a>(state: &'a mut RuntimeState, thread_id: &str, seq: u64) -> &'a mut ThreadState {
    let thread = state
        .threads
        .entry(thread_id.to_owned())
        .or_insert_with(|| ThreadState {
            id: thread_id.to_owned(),
            active_turn: None,
            turns: HashMap::new(),
            last_diff: None,
            plan: None,
            last_seq: seq,
        });
    thread.last_seq = seq;
    thread
}

fn turn_mut<'a>(thread: &'a mut ThreadState, turn_id: &str, seq: u64) -> &'a mut TurnState {
    thread.last_seq = seq;
    let turn = thread
        .turns
        .entry(turn_id.to_owned())
        .or_insert_with(|| TurnState {
            id: turn_id.to_owned(),
            status: TurnStatus::InProgress,
            items: HashMap::new(),
            error: None,
            last_seq: seq,
        });
    turn.last_seq = seq;
    turn
}

fn item_mut<'a>(turn: &'a mut TurnState, item_id: &str, seq: u64) -> &'a mut ItemState {
    turn.last_seq = seq;
    let item = turn
        .items
        .entry(item_id.to_owned())
        .or_insert_with(|| ItemState {
            id: item_id.to_owned(),
            item_type: "unknown".to_owned(),
            started: None,
            completed: None,
            text_accum: String::new(),
            stdout_accum: String::new(),
            stderr_accum: String::new(),
            text_truncated: false,
            stdout_truncated: false,
            stderr_truncated: false,
            last_seq: seq,
        });
    item.last_seq = seq;
    item
}

fn append_capped(out: &mut String, delta: &str, max_bytes: usize, truncated: &mut bool) {
    if delta.is_empty() {
        return;
    }
    if out.len() >= max_bytes {
        *truncated = true;
        return;
    }
    let remain = max_bytes - out.len();
    if delta.len() <= remain {
        out.push_str(delta);
        return;
    }
    let mut cut = remain;
    while cut > 0 && !delta.is_char_boundary(cut) {
        cut -= 1;
    }
    if cut > 0 {
        out.push_str(&delta[..cut]);
    }
    *truncated = true;
}

fn prune_state(
    state: &mut RuntimeState,
    limits: &StateProjectionLimits,
    touched_thread_id: Option<&str>,
) {
    if state.threads.len() > limits.max_threads {
        let remove_count = state.threads.len() - limits.max_threads;
        let mut by_age: Vec<(String, u64)> = state
            .threads
            .iter()
            .map(|(id, thread)| (id.clone(), thread.last_seq))
            .collect();
        if remove_count > 0 {
            by_age.select_nth_unstable_by_key(remove_count - 1, |(_, seq)| *seq);
        }
        for (id, _) in by_age.into_iter().take(remove_count) {
            state.threads.remove(&id);
        }
    }

    let Some(thread_id) = touched_thread_id else {
        return;
    };
    let Some(thread) = state.threads.get_mut(thread_id) else {
        return;
    };

    prune_turns(thread, limits.max_turns_per_thread);
    for turn in thread.turns.values_mut() {
        prune_items(turn, limits.max_items_per_turn);
    }
}

fn prune_turns(thread: &mut ThreadState, max_turns: usize) {
    if thread.turns.len() <= max_turns {
        return;
    }

    let active = thread.active_turn.as_deref();
    let mut candidates: Vec<(String, u64)> = thread
        .turns
        .iter()
        .filter(|(id, _)| Some(id.as_str()) != active)
        .map(|(id, turn)| (id.clone(), turn.last_seq))
        .collect();

    let removable = thread.turns.len().saturating_sub(max_turns);
    if removable > 0 && !candidates.is_empty() {
        let partition_idx = std::cmp::min(removable - 1, candidates.len() - 1);
        candidates.select_nth_unstable_by_key(partition_idx, |(_, seq)| *seq);
    }

    for (id, _) in candidates.into_iter().take(removable) {
        thread.turns.remove(&id);
    }
}

fn prune_items(turn: &mut TurnState, max_items: usize) {
    if turn.items.len() <= max_items {
        return;
    }

    let remove_count = turn.items.len() - max_items;
    let mut by_age: Vec<(String, u64)> = turn
        .items
        .iter()
        .map(|(id, item)| (id.clone(), item.last_seq))
        .collect();
    if remove_count > 0 {
        let partition_idx = std::cmp::min(remove_count - 1, by_age.len() - 1);
        by_age.select_nth_unstable_by_key(partition_idx, |(_, seq)| *seq);
    }
    for (id, _) in by_age.into_iter().take(remove_count) {
        turn.items.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::events::{Direction, Envelope, MsgKind};

    use super::*;

    fn envelope_with_seq(
        seq: u64,
        method: &str,
        thread: &str,
        turn: &str,
        item: Option<&str>,
        params: Value,
    ) -> Envelope {
        Envelope {
            seq,
            ts_millis: 0,
            direction: Direction::Inbound,
            kind: MsgKind::Notification,
            rpc_id: None,
            method: Some(method.to_owned()),
            thread_id: Some(thread.to_owned()),
            turn_id: Some(turn.to_owned()),
            item_id: item.map(ToOwned::to_owned),
            json: json!({"method": method, "params": params}),
        }
    }

    fn envelope(
        method: &str,
        thread: &str,
        turn: &str,
        item: Option<&str>,
        params: Value,
    ) -> Envelope {
        envelope_with_seq(1, method, thread, turn, item, params)
    }

    #[test]
    fn reduce_turn_lifecycle() {
        let state = RuntimeState::default();

        let state = reduce(
            state,
            &envelope("turn/started", "thr", "turn", None, json!({})),
        );
        assert_eq!(state.threads["thr"].active_turn.as_deref(), Some("turn"));
        assert_eq!(
            state.threads["thr"].turns["turn"].status,
            TurnStatus::InProgress
        );

        let state = reduce(
            state,
            &envelope("turn/completed", "thr", "turn", None, json!({})),
        );
        assert_eq!(state.threads["thr"].active_turn, None);
        assert_eq!(
            state.threads["thr"].turns["turn"].status,
            TurnStatus::Completed
        );
    }

    #[test]
    fn reduce_delta_and_output() {
        let state = RuntimeState::default();
        let state = reduce(
            state,
            &envelope("turn/started", "thr", "turn", None, json!({})),
        );
        let state = reduce(
            state,
            &envelope(
                "item/started",
                "thr",
                "turn",
                Some("item"),
                json!({"itemType":"agentMessage"}),
            ),
        );
        let state = reduce(
            state,
            &envelope(
                "item/agentMessage/delta",
                "thr",
                "turn",
                Some("item"),
                json!({"delta":"hello"}),
            ),
        );

        let state = reduce(
            state,
            &envelope(
                "item/commandExecution/outputDelta",
                "thr",
                "turn",
                Some("item"),
                json!({"stdout":"out","stderr":"err"}),
            ),
        );

        let item = &state.threads["thr"].turns["turn"].items["item"];
        assert_eq!(item.text_accum, "hello");
        assert_eq!(item.stdout_accum, "out");
        assert_eq!(item.stderr_accum, "err");
    }

    #[test]
    fn reduce_applies_text_caps_and_marks_truncated() {
        let mut state = RuntimeState::default();
        let limits = StateProjectionLimits {
            max_threads: 8,
            max_turns_per_thread: 8,
            max_items_per_turn: 8,
            max_text_bytes_per_item: 4,
            max_stdout_bytes_per_item: 3,
            max_stderr_bytes_per_item: 2,
        };

        reduce_in_place_with_limits(
            &mut state,
            &envelope_with_seq(
                1,
                "item/started",
                "thr",
                "turn",
                Some("item"),
                json!({"itemType":"agentMessage"}),
            ),
            &limits,
        );
        reduce_in_place_with_limits(
            &mut state,
            &envelope_with_seq(
                2,
                "item/agentMessage/delta",
                "thr",
                "turn",
                Some("item"),
                json!({"delta":"hello"}),
            ),
            &limits,
        );
        reduce_in_place_with_limits(
            &mut state,
            &envelope_with_seq(
                3,
                "item/commandExecution/outputDelta",
                "thr",
                "turn",
                Some("item"),
                json!({"stdout":"abcd","stderr":"xyz"}),
            ),
            &limits,
        );

        let item = &state.threads["thr"].turns["turn"].items["item"];
        assert_eq!(item.text_accum, "hell");
        assert!(item.text_truncated);
        assert_eq!(item.stdout_accum, "abc");
        assert!(item.stdout_truncated);
        assert_eq!(item.stderr_accum, "xy");
        assert!(item.stderr_truncated);
    }

    #[test]
    fn reduce_prunes_old_threads_turns_and_items() {
        let mut state = RuntimeState::default();
        let limits = StateProjectionLimits {
            max_threads: 2,
            max_turns_per_thread: 2,
            max_items_per_turn: 2,
            max_text_bytes_per_item: 1024,
            max_stdout_bytes_per_item: 1024,
            max_stderr_bytes_per_item: 1024,
        };

        reduce_in_place_with_limits(
            &mut state,
            &envelope_with_seq(1, "thread/started", "thr_1", "turn_a", None, json!({})),
            &limits,
        );
        reduce_in_place_with_limits(
            &mut state,
            &envelope_with_seq(2, "thread/started", "thr_2", "turn_a", None, json!({})),
            &limits,
        );
        reduce_in_place_with_limits(
            &mut state,
            &envelope_with_seq(3, "thread/started", "thr_3", "turn_a", None, json!({})),
            &limits,
        );
        assert!(!state.threads.contains_key("thr_1"));
        assert!(state.threads.contains_key("thr_2"));
        assert!(state.threads.contains_key("thr_3"));

        for seq in 10..=12 {
            let turn = format!("turn_{seq}");
            reduce_in_place_with_limits(
                &mut state,
                &envelope_with_seq(
                    seq,
                    "turn/started",
                    "thr_3",
                    &turn,
                    None,
                    json!({ "threadId":"thr_3", "turnId": turn }),
                ),
                &limits,
            );
        }
        let thr = state.threads.get("thr_3").expect("thread");
        assert!(thr.turns.len() <= 2);

        let turn_id = thr.active_turn.clone().expect("active turn");
        for seq in 20..=22 {
            let item = format!("item_{seq}");
            reduce_in_place_with_limits(
                &mut state,
                &envelope_with_seq(
                    seq,
                    "item/started",
                    "thr_3",
                    &turn_id,
                    Some(&item),
                    json!({"itemType":"agentMessage"}),
                ),
                &limits,
            );
        }

        let thr = state.threads.get("thr_3").expect("thread");
        let turn = thr.turns.get(&turn_id).expect("turn");
        assert!(turn.items.len() <= 2);
    }
}
