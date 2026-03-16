use std::future::Future;
use std::time::Duration;

use tokio::sync::broadcast::{error::RecvError, Receiver as BroadcastReceiver};
use tokio::time::{timeout, Instant};

use crate::runtime::core::Runtime;
use crate::runtime::events::Envelope;
use crate::runtime::turn_output::{TurnStreamCollector, TurnTerminalEvent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LaggedTurnTerminal {
    Completed { assistant_text: Option<String> },
    Failed { message: Option<String> },
    Interrupted,
}

impl LaggedTurnTerminal {
    fn as_terminal_event(&self) -> TurnTerminalEvent {
        match self {
            Self::Completed { .. } => TurnTerminalEvent::Completed,
            Self::Failed { .. } => TurnTerminalEvent::Failed,
            Self::Interrupted => TurnTerminalEvent::Interrupted,
        }
    }
}

#[derive(Debug)]
pub(crate) enum TurnCollectError<E> {
    Timeout,
    StreamClosed,
    EventBudgetExceeded,
    TargetEnvelope(E),
    LagProbe(E),
}

pub(crate) async fn collect_turn_terminal_with_limits<E, F, Fut, O>(
    live_rx: &mut BroadcastReceiver<Envelope>,
    stream: &mut TurnStreamCollector,
    max_turn_event_scan: usize,
    wait_timeout: Duration,
    mut on_target_envelope: O,
    mut on_lagged: F,
) -> Result<(TurnTerminalEvent, Option<LaggedTurnTerminal>), TurnCollectError<E>>
where
    O: FnMut(&Envelope) -> Result<(), E>,
    F: FnMut(Duration) -> Fut,
    Fut: Future<Output = Result<Option<LaggedTurnTerminal>, E>>,
{
    let deadline = Instant::now() + wait_timeout;
    let mut turn_event_budget = max_turn_event_scan;

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Err(TurnCollectError::Timeout);
        }
        let remaining = deadline.saturating_duration_since(now);

        let envelope = match timeout(remaining, live_rx.recv()).await {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(RecvError::Lagged(_))) => {
                let lag_probe_budget = deadline.saturating_duration_since(Instant::now());
                if lag_probe_budget.is_zero() {
                    return Err(TurnCollectError::Timeout);
                }
                match on_lagged(lag_probe_budget).await {
                    Ok(Some(snapshot)) => {
                        return Ok((snapshot.as_terminal_event(), Some(snapshot)));
                    }
                    Ok(None) => continue,
                    Err(err) => return Err(TurnCollectError::LagProbe(err)),
                }
            }
            Ok(Err(RecvError::Closed)) => return Err(TurnCollectError::StreamClosed),
            Err(_) => return Err(TurnCollectError::Timeout),
        };

        if !stream.is_target_envelope(&envelope) {
            continue;
        }

        if turn_event_budget == 0 {
            return Err(TurnCollectError::EventBudgetExceeded);
        }
        turn_event_budget = turn_event_budget.saturating_sub(1);

        on_target_envelope(&envelope).map_err(TurnCollectError::TargetEnvelope)?;
        if let Some(terminal) = stream.push_envelope(&envelope) {
            return Ok((terminal, None));
        }
    }
}

pub(crate) async fn interrupt_turn_best_effort_with_timeout(
    runtime: &Runtime,
    thread_id: &str,
    turn_id: &str,
    timeout_duration: Duration,
) {
    let _ = runtime
        .turn_interrupt_with_timeout(thread_id, turn_id, timeout_duration)
        .await;
}

pub(crate) fn interrupt_turn_best_effort_detached(
    runtime: Runtime,
    thread_id: String,
    turn_id: String,
    timeout_duration: Duration,
) {
    tokio::spawn(async move {
        interrupt_turn_best_effort_with_timeout(&runtime, &thread_id, &turn_id, timeout_duration)
            .await;
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use crate::runtime::events::{Direction, MsgKind};

    use super::*;

    fn envelope_for_turn(
        method: &str,
        thread_id: &str,
        turn_id: &str,
        params: serde_json::Value,
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
            item_id: None,
            json: Arc::new(json!({"method": method, "params": params})),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn collect_turn_terminal_returns_completed_on_matching_terminal() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<Envelope>(8);
        tx.send(envelope_for_turn(
            "turn/completed",
            "thr_target",
            "turn_target",
            json!({"text":"ok"}),
        ))
        .expect("send completed");

        let mut stream = TurnStreamCollector::new("thr_target", "turn_target");
        let (terminal, lagged) = collect_turn_terminal_with_limits::<(), _, _, _>(
            &mut rx,
            &mut stream,
            8,
            Duration::from_secs(1),
            |_| Ok(()),
            |_| async { Ok(None) },
        )
        .await
        .expect("collect terminal");

        assert_eq!(terminal, TurnTerminalEvent::Completed);
        assert!(lagged.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn collect_turn_terminal_uses_lagged_probe_snapshot() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<Envelope>(1);
        tx.send(envelope_for_turn(
            "turn/completed",
            "thr_target",
            "turn_target",
            json!({"text":"old"}),
        ))
        .expect("send first event");
        tx.send(envelope_for_turn(
            "turn/completed",
            "thr_target",
            "turn_target",
            json!({"text":"new"}),
        ))
        .expect("send second event");

        let mut stream = TurnStreamCollector::new("thr_target", "turn_target");

        let (terminal, lagged) = collect_turn_terminal_with_limits::<(), _, _, _>(
            &mut rx,
            &mut stream,
            8,
            Duration::from_millis(20),
            |_| Ok(()),
            |_| async {
                Ok(Some(LaggedTurnTerminal::Completed {
                    assistant_text: Some("lagged".to_owned()),
                }))
            },
        )
        .await
        .expect("lagged probe should resolve terminal");

        assert_eq!(terminal, TurnTerminalEvent::Completed);
        assert!(matches!(
            lagged,
            Some(LaggedTurnTerminal::Completed {
                assistant_text: Some(text)
            }) if text == "lagged"
        ));
    }
}
