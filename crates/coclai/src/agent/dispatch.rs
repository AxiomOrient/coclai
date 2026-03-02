use serde_json::{json, Value};

use crate::agent::{
    AgentDispatchError, AgentHealth, CapabilityInvocation, CapabilityResponse, CoclaiAgent,
};
use crate::capability::{
    capability_by_id, capability_registry, render_capability_parity_report, CapabilityExposure,
};
use crate::ports::inbound::health_port::HealthPort;
use crate::ports::inbound::invoke_port::InvokePort;

impl CoclaiAgent {
    pub fn health(&self) -> AgentHealth {
        let workflow_registry_size = self.state_store.workflow_config_count().unwrap_or(0);
        let appserver_registry_size =
            self.state_store
                .connection_state_count()
                .unwrap_or_else(|_| {
                    self.state
                        .lock()
                        .map(|state| state.appservers.len())
                        .unwrap_or(0)
                });

        AgentHealth {
            status: "running".to_owned(),
            registry_size: capability_registry().len(),
            full_parity_gaps: crate::capability::capability_parity_gaps().len(),
            network_ingress_loopback_only: self.security_policy.network_loopback_only,
            network_ingress_token_configured: self.security_policy.required_network_token.is_some(),
            workflow_registry_size,
            appserver_registry_size,
        }
    }

    pub fn dispatch(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        self.authorize_network_invocation(&invocation)?;
        if let Some(response) = self.system_capability_response(&invocation) {
            return Ok(response);
        }

        let capability_id = invocation.capability_id.clone();
        let descriptor = capability_by_id(&capability_id)
            .ok_or_else(|| AgentDispatchError::UnknownCapability(capability_id.clone()))?;

        let exposure = descriptor.ingress.get(invocation.ingress);
        if exposure != CapabilityExposure::Available {
            return Err(AgentDispatchError::CapabilityNotExposed {
                capability_id: descriptor.capability_id.to_owned(),
                ingress: invocation.ingress.as_str(),
                status: exposure.as_str(),
            });
        }

        self.dispatch_exposed_capability(invocation, descriptor.capability_id)
    }

    fn system_capability_response(
        &self,
        invocation: &CapabilityInvocation,
    ) -> Option<CapabilityResponse> {
        match invocation.capability_id.as_str() {
            "system/health" => {
                let health = self.health();
                Some(CapabilityResponse {
                    capability_id: invocation.capability_id.clone(),
                    correlation_id: invocation.correlation_id.clone(),
                    result: json!({
                        "status": health.status,
                        "registry_size": health.registry_size,
                        "full_parity_gaps": health.full_parity_gaps,
                        "network_ingress_loopback_only": health.network_ingress_loopback_only,
                        "network_ingress_token_configured": health.network_ingress_token_configured,
                        "workflow_registry_size": health.workflow_registry_size,
                        "appserver_registry_size": health.appserver_registry_size,
                    }),
                })
            }
            "system/capability_registry" => {
                let rows: Vec<Value> = capability_registry()
                    .iter()
                    .map(|descriptor| {
                        json!({
                            "capability_id": descriptor.capability_id,
                            "surface": descriptor.surface,
                            "summary": descriptor.summary,
                            "ingress": {
                                "stdio": descriptor.ingress.stdio.as_str(),
                                "http_localhost": descriptor.ingress.http_localhost.as_str(),
                                "ws_localhost": descriptor.ingress.ws_localhost.as_str(),
                            }
                        })
                    })
                    .collect();
                Some(CapabilityResponse {
                    capability_id: invocation.capability_id.clone(),
                    correlation_id: invocation.correlation_id.clone(),
                    result: Value::Array(rows),
                })
            }
            "system/capability_parity_report" => Some(CapabilityResponse {
                capability_id: invocation.capability_id.clone(),
                correlation_id: invocation.correlation_id.clone(),
                result: json!({
                    "report_markdown": render_capability_parity_report(),
                }),
            }),
            _ => None,
        }
    }
}

impl HealthPort for CoclaiAgent {
    fn health(&self) -> AgentHealth {
        CoclaiAgent::health(self)
    }
}

impl InvokePort for CoclaiAgent {
    fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        self.dispatch(invocation)
    }
}
