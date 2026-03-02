#![allow(dead_code)]

use crate::agent::{AgentDispatchError, CapabilityInvocation, CapabilityResponse};

pub trait InvokePort {
    fn invoke(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError>;
}
