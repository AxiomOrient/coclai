use crate::agent::CoclaiAgent;

#[derive(Clone)]
pub struct ServiceContainer {
    agent: CoclaiAgent,
}

impl ServiceContainer {
    pub fn new() -> Self {
        Self {
            agent: CoclaiAgent::new(),
        }
    }

    pub fn agent(&self) -> CoclaiAgent {
        self.agent.clone()
    }
}

pub fn build_agent() -> CoclaiAgent {
    ServiceContainer::new().agent()
}
