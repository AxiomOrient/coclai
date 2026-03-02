use serde_json::json;

use crate::agent::{AgentDispatchError, AgentSecurityPolicy, CapabilityInvocation, CoclaiAgent};
use crate::capability::CapabilityIngress;

#[test]
fn health_endpoint_is_dispatchable() {
    let agent = CoclaiAgent::new();
    let response = agent
        .dispatch(CapabilityInvocation {
            capability_id: "system/health".to_owned(),
            ingress: CapabilityIngress::Stdio,
            correlation_id: Some("corr-1".to_owned()),
            session_id: None,
            caller_addr: None,
            auth_token: None,
            payload: json!({}),
        })
        .expect("health dispatch should succeed");
    assert_eq!(response.capability_id, "system/health");
    assert_eq!(response.correlation_id.as_deref(), Some("corr-1"));
    assert_eq!(response.result["status"], "running");
}

#[test]
fn registry_endpoint_returns_rows() {
    let agent = CoclaiAgent::new();
    let response = agent
        .dispatch(CapabilityInvocation {
            capability_id: "system/capability_registry".to_owned(),
            ingress: CapabilityIngress::Stdio,
            correlation_id: None,
            session_id: None,
            caller_addr: None,
            auth_token: None,
            payload: json!({}),
        })
        .expect("registry dispatch should succeed");
    let rows = response
        .result
        .as_array()
        .expect("registry result must be array");
    assert!(!rows.is_empty(), "registry rows should not be empty");
    assert!(
        rows.iter()
            .any(|row| row["capability_id"] == "system/capability_registry"),
        "system capability must be part of registry SoT"
    );
}

#[test]
fn unknown_capability_is_rejected() {
    let agent = CoclaiAgent::new();
    let err = agent
        .dispatch(CapabilityInvocation {
            capability_id: "unknown/capability".to_owned(),
            ingress: CapabilityIngress::Stdio,
            correlation_id: None,
            session_id: None,
            caller_addr: None,
            auth_token: None,
            payload: json!({}),
        })
        .expect_err("unknown capability should fail");
    assert_eq!(
        err,
        AgentDispatchError::UnknownCapability("unknown/capability".to_owned())
    );
}

#[test]
fn known_capability_on_network_ingress_reaches_payload_validation_when_authorized() {
    let agent = CoclaiAgent::with_security_policy(AgentSecurityPolicy {
        network_loopback_only: true,
        required_network_token: Some("test-token".to_owned()),
    });
    let err = agent
        .dispatch(CapabilityInvocation {
            capability_id: "quick_run".to_owned(),
            ingress: CapabilityIngress::HttpLocalhost,
            correlation_id: None,
            session_id: None,
            caller_addr: Some("127.0.0.1:39000".to_owned()),
            auth_token: Some("test-token".to_owned()),
            payload: json!({}),
        })
        .expect_err("authorized network invocation should reach payload validation");
    assert_eq!(
        err,
        AgentDispatchError::InvalidPayload {
            capability_id: "quick_run".to_owned(),
            message: "payload.cwd must be a non-empty string".to_owned(),
        }
    );
}

#[test]
fn quick_run_requires_cwd_and_prompt_payload() {
    let agent = CoclaiAgent::new();
    let err = agent
        .dispatch(CapabilityInvocation {
            capability_id: "quick_run".to_owned(),
            ingress: CapabilityIngress::Stdio,
            correlation_id: None,
            session_id: None,
            caller_addr: None,
            auth_token: None,
            payload: json!({}),
        })
        .expect_err("quick_run without required payload should fail");
    assert_eq!(
        err,
        AgentDispatchError::InvalidPayload {
            capability_id: "quick_run".to_owned(),
            message: "payload.cwd must be a non-empty string".to_owned(),
        }
    );
}

#[test]
fn network_auth_precedes_capability_exposure_checks() {
    let agent = CoclaiAgent::with_security_policy(AgentSecurityPolicy {
        network_loopback_only: true,
        required_network_token: Some("test-token".to_owned()),
    });
    let err = agent
        .dispatch(CapabilityInvocation {
            capability_id: "quick_run".to_owned(),
            ingress: CapabilityIngress::HttpLocalhost,
            correlation_id: None,
            session_id: None,
            caller_addr: Some("127.0.0.1:39000".to_owned()),
            auth_token: None,
            payload: json!({}),
        })
        .expect_err("missing token must fail before exposure check");
    assert_eq!(
        err,
        AgentDispatchError::UnauthorizedInvocation {
            capability_id: "quick_run".to_owned(),
            ingress: "http(localhost)",
            reason: "invalid_or_missing_token",
        }
    );
}

#[test]
fn network_ingress_requires_loopback_caller_address() {
    let agent = CoclaiAgent::with_security_policy(AgentSecurityPolicy {
        network_loopback_only: true,
        required_network_token: Some("test-token".to_owned()),
    });
    let err = agent
        .dispatch(CapabilityInvocation {
            capability_id: "system/health".to_owned(),
            ingress: CapabilityIngress::HttpLocalhost,
            correlation_id: None,
            session_id: None,
            caller_addr: Some("10.10.0.3:8080".to_owned()),
            auth_token: Some("test-token".to_owned()),
            payload: json!({}),
        })
        .expect_err("non-loopback caller must fail");
    assert_eq!(
        err,
        AgentDispatchError::UnauthorizedInvocation {
            capability_id: "system/health".to_owned(),
            ingress: "http(localhost)",
            reason: "non_loopback_caller",
        }
    );
}

#[test]
fn network_ingress_requires_valid_token() {
    let agent = CoclaiAgent::with_security_policy(AgentSecurityPolicy {
        network_loopback_only: true,
        required_network_token: Some("expected-token".to_owned()),
    });
    let err = agent
        .dispatch(CapabilityInvocation {
            capability_id: "system/health".to_owned(),
            ingress: CapabilityIngress::WebSocketLocalhost,
            correlation_id: None,
            session_id: None,
            caller_addr: Some("127.0.0.1:39000".to_owned()),
            auth_token: Some("wrong-token".to_owned()),
            payload: json!({}),
        })
        .expect_err("token mismatch must fail");
    assert_eq!(
        err,
        AgentDispatchError::UnauthorizedInvocation {
            capability_id: "system/health".to_owned(),
            ingress: "ws(localhost)",
            reason: "invalid_or_missing_token",
        }
    );
}

#[test]
fn network_ingress_allows_loopback_with_matching_token() {
    let agent = CoclaiAgent::with_security_policy(AgentSecurityPolicy {
        network_loopback_only: true,
        required_network_token: Some("ok-token".to_owned()),
    });
    let response = agent
        .dispatch(CapabilityInvocation {
            capability_id: "system/health".to_owned(),
            ingress: CapabilityIngress::HttpLocalhost,
            correlation_id: None,
            session_id: None,
            caller_addr: Some("localhost:8181".to_owned()),
            auth_token: Some("ok-token".to_owned()),
            payload: json!({}),
        })
        .expect("loopback caller with token should pass");
    assert_eq!(response.capability_id, "system/health");
}

#[test]
fn network_ingress_allows_ipv6_loopback_with_matching_token() {
    let agent = CoclaiAgent::with_security_policy(AgentSecurityPolicy {
        network_loopback_only: true,
        required_network_token: Some("ok-token".to_owned()),
    });
    let response = agent
        .dispatch(CapabilityInvocation {
            capability_id: "system/health".to_owned(),
            ingress: CapabilityIngress::HttpLocalhost,
            correlation_id: None,
            session_id: None,
            caller_addr: Some("[::1]:8181".to_owned()),
            auth_token: Some("ok-token".to_owned()),
            payload: json!({}),
        })
        .expect("ipv6 loopback caller with token should pass");
    assert_eq!(response.capability_id, "system/health");
}

#[test]
fn network_ingress_denies_when_token_not_configured() {
    let agent = CoclaiAgent::with_security_policy(AgentSecurityPolicy {
        network_loopback_only: true,
        required_network_token: None,
    });
    let err = agent
        .dispatch(CapabilityInvocation {
            capability_id: "system/health".to_owned(),
            ingress: CapabilityIngress::HttpLocalhost,
            correlation_id: None,
            session_id: None,
            caller_addr: Some("127.0.0.1:39000".to_owned()),
            auth_token: Some("any".to_owned()),
            payload: json!({}),
        })
        .expect_err("network ingress must be denied when token is not configured");
    assert_eq!(
        err,
        AgentDispatchError::UnauthorizedInvocation {
            capability_id: "system/health".to_owned(),
            ingress: "http(localhost)",
            reason: "network_token_not_configured",
        }
    );
}
