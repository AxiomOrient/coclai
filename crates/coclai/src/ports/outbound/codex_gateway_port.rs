#![allow(dead_code)]

use crate::api::PromptRunResult;
use crate::client::RunProfile;

pub trait CodexGatewayPort {
    fn quick_run(&self, cwd: &str, prompt: &str) -> Result<PromptRunResult, String>;
    fn quick_run_with_profile(
        &self,
        cwd: &str,
        prompt: &str,
        profile: RunProfile,
    ) -> Result<PromptRunResult, String>;
}
