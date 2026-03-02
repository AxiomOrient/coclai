use crate::{
    capability_registry, AgentDispatchError, CapabilityInvocation, CapabilityResponse, CoclaiAgent,
};
use serde_json::{json, Value};

use super::cli::InvocationOptions;

pub(crate) fn list_capabilities_json(ingress: crate::CapabilityIngress) -> Value {
    let rows: Vec<Value> = capability_registry()
        .iter()
        .map(|descriptor| {
            json!({
                "capability_id": descriptor.capability_id,
                "surface": descriptor.surface,
                "summary": descriptor.summary,
                "ingress_requested": ingress.as_str(),
                "ingress_status": descriptor.ingress.get(ingress).as_str(),
            })
        })
        .collect();
    Value::Array(rows)
}

pub(crate) fn invoke_capability(
    agent: &CoclaiAgent,
    capability_id: String,
    options: InvocationOptions,
) -> Result<CapabilityResponse, AgentDispatchError> {
    agent.dispatch(CapabilityInvocation {
        capability_id,
        ingress: options.ingress,
        correlation_id: None,
        session_id: None,
        caller_addr: options.caller_addr,
        auth_token: options.auth_token,
        payload: options.payload,
    })
}
