#![allow(dead_code)]

use crate::agent::AgentHealth;

pub trait HealthPort {
    fn health(&self) -> AgentHealth;
}
