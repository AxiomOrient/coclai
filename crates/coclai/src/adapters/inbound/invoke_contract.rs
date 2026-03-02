use crate::{AgentDispatchError, CapabilityResponse};
use axum::http::StatusCode;
use serde_json::{json, Value};

pub(crate) fn render_invoke_success_json(response: CapabilityResponse) -> Value {
    json!({
        "ok": true,
        "capability_id": response.capability_id,
        "correlation_id": response.correlation_id,
        "result": response.result,
    })
}

pub(crate) fn map_dispatch_error(err: AgentDispatchError) -> (StatusCode, Value) {
    match err {
        AgentDispatchError::UnknownCapability(capability_id) => (
            StatusCode::NOT_FOUND,
            json!({
                "ok": false,
                "status_code": StatusCode::NOT_FOUND.as_u16(),
                "error_kind": "unknown_capability",
                "capability_id": capability_id,
                "message": format!("unknown capability: {capability_id}"),
            }),
        ),
        AgentDispatchError::CapabilityNotExposed {
            capability_id,
            ingress,
            status,
        } => (
            StatusCode::FORBIDDEN,
            json!({
                "ok": false,
                "status_code": StatusCode::FORBIDDEN.as_u16(),
                "error_kind": "capability_not_exposed",
                "capability_id": capability_id,
                "ingress": ingress,
                "exposure_status": status,
                "message": format!(
                    "capability `{capability_id}` is not exposed on ingress `{ingress}` (status={status})"
                ),
            }),
        ),
        AgentDispatchError::InvalidPayload {
            capability_id,
            message,
        } => (
            StatusCode::BAD_REQUEST,
            json!({
                "ok": false,
                "status_code": StatusCode::BAD_REQUEST.as_u16(),
                "error_kind": "invalid_payload",
                "capability_id": capability_id,
                "message": format!("invalid payload for `{capability_id}`: {message}"),
            }),
        ),
        AgentDispatchError::UnauthorizedInvocation {
            capability_id,
            ingress,
            reason,
        } => (
            StatusCode::UNAUTHORIZED,
            json!({
                "ok": false,
                "status_code": StatusCode::UNAUTHORIZED.as_u16(),
                "error_kind": "unauthorized",
                "capability_id": capability_id,
                "ingress": ingress,
                "reason": reason,
                "message": format!(
                    "unauthorized invocation: capability `{capability_id}` ingress `{ingress}` reason `{reason}`"
                ),
            }),
        ),
        AgentDispatchError::BackendFailure {
            capability_id,
            message,
        } => (
            StatusCode::BAD_GATEWAY,
            json!({
                "ok": false,
                "status_code": StatusCode::BAD_GATEWAY.as_u16(),
                "error_kind": "backend_failure",
                "capability_id": capability_id,
                "message": format!("backend failure for `{capability_id}`: {message}"),
            }),
        ),
    }
}

pub(crate) fn render_dispatch_error_json(err: AgentDispatchError) -> Value {
    let (_, value) = map_dispatch_error(err);
    value
}

#[cfg(test)]
mod tests {
    use crate::AgentDispatchError;

    use super::{map_dispatch_error, render_dispatch_error_json};

    #[test]
    fn unauthorized_error_contains_reason_and_status_code() {
        let error = AgentDispatchError::UnauthorizedInvocation {
            capability_id: "system/health".to_owned(),
            ingress: "http(localhost)",
            reason: "invalid_or_missing_token",
        };
        let (status, body) = map_dispatch_error(error);
        assert_eq!(status.as_u16(), 401);
        assert_eq!(body["status_code"], 401);
        assert_eq!(body["error_kind"], "unauthorized");
        assert_eq!(body["reason"], "invalid_or_missing_token");
    }

    #[test]
    fn invalid_payload_error_keeps_capability_context() {
        let body = render_dispatch_error_json(AgentDispatchError::InvalidPayload {
            capability_id: "quick_run".to_owned(),
            message: "payload.cwd must be a non-empty string".to_owned(),
        });
        assert_eq!(body["status_code"], 400);
        assert_eq!(body["error_kind"], "invalid_payload");
        assert_eq!(body["capability_id"], "quick_run");
        assert!(body["message"]
            .as_str()
            .expect("message should be string")
            .contains("payload.cwd"));
    }
}
