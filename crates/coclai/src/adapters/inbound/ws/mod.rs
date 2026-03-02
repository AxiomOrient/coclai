use std::net::SocketAddr;

use crate::CapabilityIngress;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::response::IntoResponse;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use super::cli::normalize_token;
use super::http::{dispatch_ingress_invocation, AgentIngressState, CapabilityInvokeRequest};

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct WebSocketAuthQuery {
    #[serde(default)]
    token: Option<String>,
}

pub(crate) async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AgentIngressState>,
    ConnectInfo(caller_addr): ConnectInfo<SocketAddr>,
    Query(query): Query<WebSocketAuthQuery>,
) -> impl IntoResponse {
    let session_token = normalize_token(query.token);
    ws.on_upgrade(move |socket| handle_websocket_session(socket, state, caller_addr, session_token))
}

async fn send_websocket_json(socket: &mut WebSocket, value: Value) {
    let _ = socket.send(Message::Text(value.to_string())).await;
}

async fn handle_websocket_session(
    mut socket: WebSocket,
    state: AgentIngressState,
    caller_addr: SocketAddr,
    session_token: Option<String>,
) {
    while let Some(next_message) = socket.next().await {
        let message = match next_message {
            Ok(message) => message,
            Err(_) => break,
        };

        match message {
            Message::Text(raw) => {
                let parsed: Result<CapabilityInvokeRequest, _> = serde_json::from_str(raw.as_str());
                let request = match parsed {
                    Ok(request) => request,
                    Err(err) => {
                        send_websocket_json(
                            &mut socket,
                            json!({
                                "ok": false,
                                "status_code": 400,
                                "error_kind": "invalid_json",
                                "message": format!("invalid websocket payload: {err}"),
                            }),
                        )
                        .await;
                        continue;
                    }
                };

                match dispatch_ingress_invocation(
                    &state,
                    CapabilityIngress::WebSocketLocalhost,
                    caller_addr,
                    request,
                    session_token.clone(),
                ) {
                    Ok(value) => send_websocket_json(&mut socket, value).await,
                    Err((_, value)) => {
                        send_websocket_json(&mut socket, value).await;
                    }
                }
            }
            Message::Ping(payload) => {
                if socket.send(Message::Pong(payload)).await.is_err() {
                    break;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}
