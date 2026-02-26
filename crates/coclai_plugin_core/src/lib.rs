use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type HookFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginContractVersion {
    pub major: u16,
    pub minor: u16,
}

impl PluginContractVersion {
    pub const CURRENT: Self = Self { major: 1, minor: 0 };

    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    pub const fn is_compatible_with(self, other: Self) -> bool {
        self.major == other.major
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookPhase {
    PreRun,
    PostRun,
    PreSessionStart,
    PostSessionStart,
    PreTurn,
    PostTurn,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookContext {
    pub phase: HookPhase,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub main_status: Option<String>,
    pub correlation_id: String,
    pub ts_ms: i64,
    pub metadata: Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookAttachment {
    AtPath {
        path: String,
        placeholder: Option<String>,
    },
    ImageUrl {
        url: String,
    },
    LocalImage {
        path: String,
    },
    Skill {
        name: String,
        path: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookPatch {
    pub prompt_override: Option<String>,
    pub model_override: Option<String>,
    pub add_attachments: Vec<HookAttachment>,
    pub metadata_delta: Value,
}

impl Default for HookPatch {
    fn default() -> Self {
        Self {
            prompt_override: None,
            model_override: None,
            add_attachments: Vec::new(),
            metadata_delta: Value::Null,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HookAction {
    Noop,
    Mutate(HookPatch),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookIssueClass {
    Validation,
    Execution,
    Timeout,
    Internal,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HookIssue {
    pub hook_name: String,
    pub phase: HookPhase,
    pub class: HookIssueClass,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct HookReport {
    pub issues: Vec<HookIssue>,
}

impl HookReport {
    pub fn push(&mut self, issue: HookIssue) {
        self.issues.push(issue);
    }

    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

pub trait PreHook: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn call<'a>(&'a self, ctx: &'a HookContext) -> HookFuture<'a, Result<HookAction, HookIssue>>;
}

pub trait PostHook: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn call<'a>(&'a self, ctx: &'a HookContext) -> HookFuture<'a, Result<(), HookIssue>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_contract_major_must_match() {
        let current = PluginContractVersion::CURRENT;
        assert!(current.is_compatible_with(PluginContractVersion::new(1, 99)));
        assert!(!current.is_compatible_with(PluginContractVersion::new(2, 0)));
    }

    #[test]
    fn hook_report_tracks_issues() {
        let mut report = HookReport::default();
        assert!(report.is_clean());
        report.push(HookIssue {
            hook_name: "pre_sanitize".to_owned(),
            phase: HookPhase::PreRun,
            class: HookIssueClass::Validation,
            message: "invalid metadata".to_owned(),
        });
        assert!(!report.is_clean());
        assert_eq!(report.issues.len(), 1);
    }
}
