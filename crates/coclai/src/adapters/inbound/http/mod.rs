use std::net::SocketAddr;
use std::sync::Arc;

use crate::{CapabilityIngress, CapabilityInvocation, CoclaiAgent};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::cli::normalize_token;
use super::invoke_contract::{map_dispatch_error, render_invoke_success_json};

#[derive(Clone)]
pub(crate) struct AgentIngressState {
    pub(crate) agent: Arc<CoclaiAgent>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CapabilityInvokeRequest {
    pub(crate) capability_id: String,
    #[serde(default)]
    pub(crate) correlation_id: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default = "default_invoke_payload")]
    pub(crate) payload: Value,
    #[serde(default)]
    pub(crate) auth_token: Option<String>,
}

fn default_invoke_payload() -> Value {
    json!({})
}

pub(crate) fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers
        .get("x-coclai-token")
        .and_then(|value| value.to_str().ok())
    {
        return normalize_token(Some(value.to_owned()));
    }

    let auth = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)?;
    let token = auth
        .strip_prefix("Bearer ")
        .or_else(|| auth.strip_prefix("bearer "))?;
    normalize_token(Some(token.to_owned()))
}

pub(crate) fn dispatch_ingress_invocation(
    state: &AgentIngressState,
    ingress: CapabilityIngress,
    caller_addr: SocketAddr,
    request: CapabilityInvokeRequest,
    default_auth_token: Option<String>,
) -> Result<Value, (StatusCode, Value)> {
    let invocation = CapabilityInvocation {
        capability_id: request.capability_id,
        ingress,
        correlation_id: request.correlation_id,
        session_id: request.session_id,
        caller_addr: Some(caller_addr.to_string()),
        auth_token: normalize_token(request.auth_token.or(default_auth_token)),
        payload: request.payload,
    };

    state
        .agent
        .dispatch(invocation)
        .map(render_invoke_success_json)
        .map_err(map_dispatch_error)
}

pub(crate) async fn http_health(
    State(state): State<AgentIngressState>,
    ConnectInfo(caller_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match dispatch_ingress_invocation(
        &state,
        CapabilityIngress::HttpLocalhost,
        caller_addr,
        CapabilityInvokeRequest {
            capability_id: "system/health".to_owned(),
            correlation_id: None,
            session_id: None,
            payload: json!({}),
            auth_token: None,
        },
        token_from_headers(&headers),
    ) {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err((status, value)) => (status, Json(value)),
    }
}

pub(crate) async fn http_capabilities(
    State(state): State<AgentIngressState>,
    ConnectInfo(caller_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    match dispatch_ingress_invocation(
        &state,
        CapabilityIngress::HttpLocalhost,
        caller_addr,
        CapabilityInvokeRequest {
            capability_id: "system/capability_registry".to_owned(),
            correlation_id: None,
            session_id: None,
            payload: json!({}),
            auth_token: None,
        },
        token_from_headers(&headers),
    ) {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err((status, value)) => (status, Json(value)),
    }
}

pub(crate) async fn http_invoke(
    State(state): State<AgentIngressState>,
    ConnectInfo(caller_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<CapabilityInvokeRequest>,
) -> impl IntoResponse {
    match dispatch_ingress_invocation(
        &state,
        CapabilityIngress::HttpLocalhost,
        caller_addr,
        request,
        token_from_headers(&headers),
    ) {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err((status, value)) => (status, Json(value)),
    }
}
