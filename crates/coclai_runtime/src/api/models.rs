use std::time::Duration;

use thiserror::Error;

use crate::errors::{RpcError, RuntimeError};

use super::{
    ApprovalPolicy, PromptAttachment, ReasoningEffort, SandboxPolicy, ThreadId, TurnId,
    DEFAULT_REASONING_EFFORT,
};

#[derive(Clone, Debug, PartialEq)]
pub struct PromptRunParams {
    pub cwd: String,
    pub prompt: String,
    pub model: Option<String>,
    pub effort: Option<ReasoningEffort>,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_policy: SandboxPolicy,
    /// Explicit opt-in gate for privileged sandbox usage (SEC-004).
    /// Default stays false to preserve safe-by-default posture.
    pub privileged_escalation_approved: bool,
    pub attachments: Vec<PromptAttachment>,
    pub timeout: Duration,
}

impl PromptRunParams {
    /// Create prompt-run params with safe defaults.
    /// Allocation: two String allocations for cwd/prompt. Complexity: O(n), n = input lengths.
    pub fn new(cwd: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            cwd: cwd.into(),
            prompt: prompt.into(),
            model: None,
            effort: Some(DEFAULT_REASONING_EFFORT),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(super::SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: Vec::new(),
            timeout: Duration::from_secs(120),
        }
    }

    /// Set explicit model override.
    /// Allocation: one String. Complexity: O(model length).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set explicit reasoning effort.
    /// Allocation: none. Complexity: O(1).
    pub fn with_effort(mut self, effort: ReasoningEffort) -> Self {
        self.effort = Some(effort);
        self
    }

    /// Set approval policy override.
    /// Allocation: none. Complexity: O(1).
    pub fn with_approval_policy(mut self, approval_policy: ApprovalPolicy) -> Self {
        self.approval_policy = approval_policy;
        self
    }

    /// Set sandbox policy override.
    /// Allocation: depends on sandbox payload move/clone at callsite. Complexity: O(1).
    pub fn with_sandbox_policy(mut self, sandbox_policy: SandboxPolicy) -> Self {
        self.sandbox_policy = sandbox_policy;
        self
    }

    /// Explicitly approve privileged sandbox escalation for this run.
    /// Callers are expected to set approval policy + scope alongside this flag.
    pub fn allow_privileged_escalation(mut self) -> Self {
        self.privileged_escalation_approved = true;
        self
    }

    /// Set prompt timeout.
    /// Allocation: none. Complexity: O(1).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add one generic attachment.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_attachment(mut self, attachment: PromptAttachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Add one `@path` attachment.
    /// Allocation: one String. Complexity: O(path length).
    pub fn attach_path(self, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::AtPath {
            path: path.into(),
            placeholder: None,
        })
    }

    /// Add one `@path` attachment with placeholder.
    /// Allocation: two Strings. Complexity: O(path + placeholder length).
    pub fn attach_path_with_placeholder(
        self,
        path: impl Into<String>,
        placeholder: impl Into<String>,
    ) -> Self {
        self.with_attachment(PromptAttachment::AtPath {
            path: path.into(),
            placeholder: Some(placeholder.into()),
        })
    }

    /// Add one remote image attachment.
    /// Allocation: one String. Complexity: O(url length).
    pub fn attach_image_url(self, url: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::ImageUrl { url: url.into() })
    }

    /// Add one local image attachment.
    /// Allocation: one String. Complexity: O(path length).
    pub fn attach_local_image(self, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::LocalImage { path: path.into() })
    }

    /// Add one skill attachment.
    /// Allocation: two Strings. Complexity: O(name + path length).
    pub fn attach_skill(self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::Skill {
            name: name.into(),
            path: path.into(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRunResult {
    pub thread_id: ThreadId,
    pub turn_id: TurnId,
    pub assistant_text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptTurnTerminalState {
    Failed,
    CompletedWithoutAssistantText,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptTurnFailure {
    pub terminal_state: PromptTurnTerminalState,
    pub source_method: String,
    pub code: Option<i64>,
    pub message: String,
}

impl std::fmt::Display for PromptTurnFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let terminal = match self.terminal_state {
            PromptTurnTerminalState::Failed => "failed",
            PromptTurnTerminalState::CompletedWithoutAssistantText => {
                "completed_without_assistant_text"
            }
        };
        if let Some(code) = self.code {
            write!(
                f,
                "terminal={terminal} source_method={} code={code} message={}",
                self.source_method, self.message
            )
        } else {
            write!(
                f,
                "terminal={terminal} source_method={} message={}",
                self.source_method, self.message
            )
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PromptRunError {
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError),
    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),
    #[error("turn failed: {0}")]
    TurnFailedWithContext(PromptTurnFailure),
    #[error("turn failed")]
    TurnFailed,
    #[error("turn interrupted")]
    TurnInterrupted,
    #[error("turn timed out after {0:?}")]
    Timeout(Duration),
    #[error("turn completed without assistant text: {0}")]
    TurnCompletedWithoutAssistantText(PromptTurnFailure),
    #[error("assistant text is empty")]
    EmptyAssistantText,
    #[error("attachment not found: {0}")]
    AttachmentNotFound(String),
}
