use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{json, Map, Value};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration, MissedTickBehavior};
use uuid::Uuid;

use crate::approvals::{route_server_request, ServerRequest, ServerRequestRoute, TimeoutAction};
use crate::errors::RuntimeError;
use crate::events::{Direction, Envelope, JsonRpcId, MsgKind};
use crate::metrics::RuntimeMetrics;
use crate::rpc::{extract_message_metadata, map_rpc_error};
use crate::sink::EventSink;

use super::rpc_io::resolve_transport_closed_pending;
use super::state_projection::{
    state_apply_envelope, state_insert_pending_server_request, state_remove_pending_server_request,
};
use super::{now_millis, PendingServerRequestEntry, RuntimeInner};

const APPROVAL_TIMEOUT_SWEEP_INTERVAL: Duration = Duration::from_millis(50);

pub(super) async fn dispatcher_loop(inner: Arc<RuntimeInner>, mut read_rx: mpsc::Receiver<Value>) {
    let mut timeout_sweep = interval(APPROVAL_TIMEOUT_SWEEP_INTERVAL);
    timeout_sweep.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            maybe_json = read_rx.recv() => {
                let Some(json) = maybe_json else {
                    break;
                };
        inner.metrics.record_ingress();
        let metadata = extract_message_metadata(&json);
        let kind = metadata.kind;
        let response_id = metadata.response_id;
        let request_id = metadata.rpc_id.clone();

        match kind {
            MsgKind::Response => {
                if let Some(id) = response_id {
                    let response = if let Some(err) = json.get("error") {
                        Err(map_rpc_error(err))
                    } else {
                        Ok(json.get("result").cloned().unwrap_or(Value::Null))
                    };

                    if let Some(tx) = inner.io.pending.lock().await.remove(&id) {
                        inner.metrics.dec_pending_rpc();
                        let _ = tx.send(response);
                    }
                }
            }
            MsgKind::ServerRequest => {
                if let (Some(id), Some(method)) = (request_id, metadata.method.as_deref()) {
                    let params = json.get("params").cloned().unwrap_or(Value::Null);
                    match route_server_request(
                        method,
                        inner.spec.server_request_cfg.auto_decline_unknown,
                    ) {
                        ServerRequestRoute::AutoDecline => {
                            let _ = respond_with_timeout_policy(&inner, &id, method).await;
                        }
                        ServerRequestRoute::Queue => {
                            let approval_id = Uuid::new_v4().to_string();
                            let now = now_millis();
                            let deadline =
                                now + inner.spec.server_request_cfg.default_timeout_ms as i64;
                            let rpc_key = jsonrpc_state_key(&id);
                            inner.io.pending_server_requests.lock().await.insert(
                                approval_id.clone(),
                                PendingServerRequestEntry {
                                    rpc_id: id,
                                    rpc_key: rpc_key.clone(),
                                    method: method.to_owned(),
                                    created_at_millis: now,
                                    deadline_millis: deadline,
                                },
                            );
                            inner.metrics.inc_pending_server_request();
                            state_insert_pending_server_request(
                                &inner,
                                &rpc_key,
                                crate::approvals::PendingServerRequest {
                                    approval_id: approval_id.clone(),
                                    deadline_unix_ms: deadline,
                                    method: method.to_owned(),
                                    params: params.clone(),
                                },
                            );

                            let req = ServerRequest {
                                approval_id: approval_id.clone(),
                                method: method.to_owned(),
                                params: params.clone(),
                            };
                            if inner.io.server_request_tx.send(req).await.is_err() {
                                // Approval queue is unavailable: resolve immediately using
                                // timeout policy so pending maps do not grow until timer expiry.
                                let pending = inner
                                    .io
                                    .pending_server_requests
                                    .lock()
                                    .await
                                    .remove(&approval_id);
                                if let Some(pending) = pending {
                                    inner.metrics.dec_pending_server_request();
                                    state_remove_pending_server_request(&inner, &pending.rpc_key);
                                    let _ = respond_with_timeout_policy(
                                        &inner,
                                        &pending.rpc_id,
                                        &pending.method,
                                    )
                                    .await;
                                }
                                continue;
                            }
                        }
                    }
                }
            }
            MsgKind::Notification | MsgKind::Unknown => {}
        }

        let seq = inner.counters.next_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let envelope = Envelope {
            seq,
            ts_millis: now_millis(),
            direction: Direction::Inbound,
            kind,
            rpc_id: metadata.rpc_id,
            method: metadata.method,
            thread_id: metadata.thread_id,
            turn_id: metadata.turn_id,
            item_id: metadata.item_id,
            json,
        };
        state_apply_envelope(&inner, &envelope);
        route_event_sink(&inner, &envelope);
        if inner.io.live_tx.send(envelope).is_err() {
            inner.metrics.record_broadcast_send_failed();
        }
            }
            _ = timeout_sweep.tick() => {
                expire_pending_server_requests(&inner).await;
            }
        }
    }

    resolve_transport_closed_pending(&inner).await;
    inner.io.transport_closed_signal.notify_one();
}

async fn expire_pending_server_requests(inner: &Arc<RuntimeInner>) {
    let now = now_millis();
    let expired: Vec<PendingServerRequestEntry> = {
        let mut pending = inner.io.pending_server_requests.lock().await;
        let mut expired = Vec::new();
        pending.retain(|_, entry| {
            if entry.deadline_millis <= now {
                expired.push(entry.clone());
                false
            } else {
                true
            }
        });
        expired
    };

    for entry in expired {
        inner.metrics.dec_pending_server_request();
        state_remove_pending_server_request(inner, &entry.rpc_key);
        let _ = respond_with_timeout_policy(inner, &entry.rpc_id, &entry.method).await;
    }
}

/// Forward one envelope to the optional sink queue without blocking core flow.
/// Allocation: one `Envelope` clone only when sink is configured.
/// Complexity: O(1).
fn route_event_sink(inner: &Arc<RuntimeInner>, envelope: &Envelope) {
    let Some(tx) = inner.io.event_sink_tx.as_ref() else {
        return;
    };

    match tx.try_send(envelope.clone()) {
        Ok(()) => {
            inner.metrics.inc_event_sink_queue_depth();
        }
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            inner.metrics.record_event_sink_drop();
            tracing::warn!(
                seq = envelope.seq,
                method = ?envelope.method,
                "event sink queue full; dropping envelope"
            );
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            inner.metrics.record_event_sink_drop();
            tracing::warn!(
                seq = envelope.seq,
                method = ?envelope.method,
                "event sink queue closed; dropping envelope"
            );
        }
    }
}

/// Isolated sink worker. Sink failures are logged and never fail runtime dispatch.
/// Allocation: none in control path; sink-specific allocation happens in `on_envelope`.
/// Complexity: O(1) per envelope plus sink-specific I/O.
pub(super) async fn event_sink_loop(
    sink: Arc<dyn EventSink>,
    metrics: Arc<RuntimeMetrics>,
    mut rx: mpsc::Receiver<Envelope>,
) {
    while let Some(envelope) = rx.recv().await {
        metrics.dec_event_sink_queue_depth();
        let started = std::time::Instant::now();
        let write_result = sink.on_envelope(&envelope).await;
        let elapsed_micros = started.elapsed().as_micros() as u64;
        metrics.record_sink_write(elapsed_micros, write_result.is_err());
        if let Err(err) = write_result {
            tracing::warn!(
                seq = envelope.seq,
                method = ?envelope.method,
                error = %err,
                "event sink write failed"
            );
        }
    }
}

async fn respond_with_timeout_policy(
    inner: &Arc<RuntimeInner>,
    rpc_id: &JsonRpcId,
    method: &str,
) -> Result<(), RuntimeError> {
    if method == "account/chatgptAuthTokens/refresh" {
        return send_timeout_error(inner, rpc_id, method).await;
    }

    match inner.spec.server_request_cfg.on_timeout {
        TimeoutAction::Decline => {
            send_rpc_result(inner, rpc_id, timeout_result_payload(method, false)).await
        }
        TimeoutAction::Cancel => {
            send_rpc_result(inner, rpc_id, timeout_result_payload(method, true)).await
        }
        TimeoutAction::Error => send_timeout_error(inner, rpc_id, method).await,
    }
}

pub(super) fn validate_server_request_result_payload(
    method: &str,
    result: &Value,
) -> Result<(), RuntimeError> {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            let decision = result.get("decision");
            match decision {
                Some(Value::String(_)) => Ok(()),
                Some(Value::Object(obj)) if !obj.is_empty() => Ok(()),
                _ => Err(RuntimeError::Internal(format!(
                    "invalid approval payload for {method}: missing decision"
                ))),
            }
        }
        "item/tool/requestUserInput" => {
            let Some(obj) = result.as_object() else {
                return Err(RuntimeError::Internal(
                    "invalid requestUserInput payload: expected object".to_owned(),
                ));
            };
            if !matches!(obj.get("answers"), Some(Value::Object(_))) {
                return Err(RuntimeError::Internal(
                    "invalid requestUserInput payload: missing answers object".to_owned(),
                ));
            }
            Ok(())
        }
        "item/tool/call" => {
            let Some(obj) = result.as_object() else {
                return Err(RuntimeError::Internal(
                    "invalid dynamic tool call payload: expected object".to_owned(),
                ));
            };
            if !matches!(obj.get("success"), Some(Value::Bool(_))) {
                return Err(RuntimeError::Internal(
                    "invalid dynamic tool call payload: missing success boolean".to_owned(),
                ));
            }
            if !matches!(obj.get("contentItems"), Some(Value::Array(_))) {
                return Err(RuntimeError::Internal(
                    "invalid dynamic tool call payload: missing contentItems array".to_owned(),
                ));
            }
            Ok(())
        }
        "account/chatgptAuthTokens/refresh" => {
            let Some(obj) = result.as_object() else {
                return Err(RuntimeError::Internal(
                    "invalid auth refresh payload: expected object".to_owned(),
                ));
            };
            if !matches!(obj.get("accessToken"), Some(Value::String(_))) {
                return Err(RuntimeError::Internal(
                    "invalid auth refresh payload: missing accessToken".to_owned(),
                ));
            }
            if !matches!(obj.get("chatgptAccountId"), Some(Value::String(_))) {
                return Err(RuntimeError::Internal(
                    "invalid auth refresh payload: missing chatgptAccountId".to_owned(),
                ));
            }
            if !matches!(
                obj.get("chatgptPlanType"),
                None | Some(Value::String(_)) | Some(Value::Null)
            ) {
                return Err(RuntimeError::Internal(
                    "invalid auth refresh payload: chatgptPlanType must be string|null".to_owned(),
                ));
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn timeout_result_payload(method: &str, cancel: bool) -> Value {
    match method {
        "item/tool/requestUserInput" => json!({ "answers": {} }),
        "item/tool/call" => json!({ "success": false, "contentItems": [] }),
        _ => {
            let decision = if cancel { "cancel" } else { "decline" };
            json!({ "decision": decision })
        }
    }
}

async fn send_timeout_error(
    inner: &Arc<RuntimeInner>,
    rpc_id: &JsonRpcId,
    method: &str,
) -> Result<(), RuntimeError> {
    send_rpc_error(
        inner,
        rpc_id,
        json!({
            "code": -32000,
            "message": "server request timed out",
            "data": { "method": method }
        }),
    )
    .await
}

pub(super) async fn send_rpc_result(
    inner: &Arc<RuntimeInner>,
    rpc_id: &JsonRpcId,
    result: Value,
) -> Result<(), RuntimeError> {
    let outbound_tx = inner
        .io
        .outbound_tx
        .load_full()
        .ok_or(RuntimeError::TransportClosed)?;

    let mut message = Map::<String, Value>::new();
    message.insert("id".to_owned(), jsonrpc_id_to_value(rpc_id));
    message.insert("result".to_owned(), result);
    outbound_tx
        .send(Value::Object(message))
        .await
        .map_err(|_| RuntimeError::TransportClosed)
}

pub(super) async fn send_rpc_error(
    inner: &Arc<RuntimeInner>,
    rpc_id: &JsonRpcId,
    error: Value,
) -> Result<(), RuntimeError> {
    let outbound_tx = inner
        .io
        .outbound_tx
        .load_full()
        .ok_or(RuntimeError::TransportClosed)?;

    let mut message = Map::<String, Value>::new();
    message.insert("id".to_owned(), jsonrpc_id_to_value(rpc_id));
    message.insert("error".to_owned(), error);
    outbound_tx
        .send(Value::Object(message))
        .await
        .map_err(|_| RuntimeError::TransportClosed)
}

fn jsonrpc_id_to_value(id: &JsonRpcId) -> Value {
    match id {
        JsonRpcId::Number(v) => Value::Number((*v).into()),
        JsonRpcId::Text(v) => Value::String(v.clone()),
    }
}

fn jsonrpc_state_key(id: &JsonRpcId) -> String {
    match id {
        JsonRpcId::Number(v) => format!("n:{v}"),
        JsonRpcId::Text(v) => format!("s:{v}"),
    }
}
