use std::net::{IpAddr, SocketAddr};

use crate::agent::{AgentDispatchError, CapabilityInvocation, CoclaiAgent};
use crate::capability::CapabilityIngress;

fn is_loopback_caller(caller_addr: &str) -> bool {
    let caller_addr = caller_addr.trim();
    if caller_addr.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if let Ok(socket) = caller_addr.parse::<SocketAddr>() {
        return socket.ip().is_loopback();
    }
    if let Ok(ip) = caller_addr.parse::<IpAddr>() {
        return ip.is_loopback();
    }

    let host = caller_addr
        .rsplit_once(':')
        .map_or(caller_addr, |(host, _)| host)
        .trim_matches(&['[', ']'][..]);
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

impl CoclaiAgent {
    pub(super) fn authorize_network_invocation(
        &self,
        invocation: &CapabilityInvocation,
    ) -> Result<(), AgentDispatchError> {
        if matches!(invocation.ingress, CapabilityIngress::Stdio) {
            return Ok(());
        }

        if self.security_policy.network_loopback_only {
            let caller_addr = invocation.caller_addr.as_deref().ok_or_else(|| {
                AgentDispatchError::UnauthorizedInvocation {
                    capability_id: invocation.capability_id.clone(),
                    ingress: invocation.ingress.as_str(),
                    reason: "caller_addr_required",
                }
            })?;
            if !is_loopback_caller(caller_addr) {
                return Err(AgentDispatchError::UnauthorizedInvocation {
                    capability_id: invocation.capability_id.clone(),
                    ingress: invocation.ingress.as_str(),
                    reason: "non_loopback_caller",
                });
            }
        }

        let Some(required_token) = self.security_policy.required_network_token.as_deref() else {
            return Err(AgentDispatchError::UnauthorizedInvocation {
                capability_id: invocation.capability_id.clone(),
                ingress: invocation.ingress.as_str(),
                reason: "network_token_not_configured",
            });
        };

        if invocation.auth_token.as_deref() != Some(required_token) {
            return Err(AgentDispatchError::UnauthorizedInvocation {
                capability_id: invocation.capability_id.clone(),
                ingress: invocation.ingress.as_str(),
                reason: "invalid_or_missing_token",
            });
        }

        Ok(())
    }
}
