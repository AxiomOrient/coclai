use super::*;

#[tokio::test(flavor = "current_thread")]
async fn collect_turn_output_times_out_without_matching_events() {
    let (_tx, mut rx) = tokio::sync::broadcast::channel::<Envelope>(8);

    let err = collect_turn_output_from_live_with_limits(
        &mut rx,
        "thr_timeout",
        "turn_timeout",
        8,
        Duration::from_millis(25),
    )
    .await
    .expect_err("must timeout");

    assert!(matches!(err, DomainError::Runtime(RuntimeError::Timeout)));
}

#[tokio::test(flavor = "current_thread")]
async fn collect_turn_output_budget_counts_only_matching_turn_events() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<Envelope>(32);

    for _ in 0..16 {
        tx.send(envelope_for_turn(
            "turn/completed",
            "thr_other",
            "turn_other",
            json!({"output":{"ignored": true}}),
        ))
        .expect("send unrelated");
    }

    tx.send(envelope_for_turn(
        "turn/completed",
        "thr_target",
        "turn_target",
        json!({"output":{"status":"ok"}}),
    ))
    .expect("send target");

    let output = collect_turn_output_from_live_with_limits(
        &mut rx,
        "thr_target",
        "turn_target",
        1,
        Duration::from_secs(1),
    )
    .await
    .expect("must collect output");

    assert_eq!(output["status"], "ok");
}

#[tokio::test(flavor = "current_thread")]
async fn collect_turn_output_returns_validation_error_on_cancelled_terminal() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<Envelope>(8);
    tx.send(envelope_for_turn(
        "turn/cancelled",
        "thr_target",
        "turn_target",
        json!({}),
    ))
    .expect("send cancelled");

    let err = collect_turn_output_from_live_with_limits(
        &mut rx,
        "thr_target",
        "turn_target",
        8,
        Duration::from_secs(1),
    )
    .await
    .expect_err("cancelled terminal must fail");

    assert!(
        matches!(err, DomainError::Validation(message) if message.contains("turn interrupted"))
    );
}
