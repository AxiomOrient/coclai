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

fn envelope(method: &str, thread: &str, turn: &str, item: Option<&str>, params: Value) -> Envelope {
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
