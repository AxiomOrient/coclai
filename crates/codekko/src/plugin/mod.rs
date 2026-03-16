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
    /// Called before a tool (command/file-change) executes, via the approval loop.
    PreToolUse,
    /// Reserved for post-execution tool events (not yet wired).
    PostToolUse,
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
    /// Tool or command name, set for PreToolUse/PostToolUse phases.
    pub tool_name: Option<String>,
    /// Raw tool input params, set for PreToolUse/PostToolUse phases.
    pub tool_input: Option<Value>,
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

/// The reason a [`PreHook`] decided to block execution.
/// Allocation: two Strings. Complexity: O(1) to construct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockReason {
    pub hook_name: String,
    pub phase: HookPhase,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HookAction {
    Noop,
    Mutate(HookPatch),
    /// Stop execution immediately. No subsequent hooks run. No state is mutated.
    Block(BlockReason),
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

/// Pure filter that gates a hook on phase, tool name, and/or cwd prefix.
/// `phases` empty = all phases match. `tool_name` / `cwd_prefix` None = no constraint.
/// Allocation: O(phases count + name/prefix length) at construction.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookMatcher {
    /// Phases this hook applies to. Empty = all phases.
    pub phases: Vec<HookPhase>,
    /// Exact tool name filter (for PreToolUse). None = any tool.
    pub tool_name: Option<String>,
    /// Working-directory prefix filter. None = any cwd.
    pub cwd_prefix: Option<String>,
}

impl HookMatcher {
    /// Match only specific phases.
    /// Allocation: one Vec. Complexity: O(phases count).
    pub fn phases(phases: impl Into<Vec<HookPhase>>) -> Self {
        Self {
            phases: phases.into(),
            ..Self::default()
        }
    }

    /// Add exact tool_name constraint (meaningful for PreToolUse).
    /// Allocation: one String. Complexity: O(name length).
    pub fn with_tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = Some(name.into());
        self
    }

    /// Add cwd_prefix constraint. Uses `str::starts_with` matching.
    /// Allocation: one String. Complexity: O(prefix length).
    pub fn with_cwd_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.cwd_prefix = Some(prefix.into());
        self
    }

    /// True when `ctx` satisfies all non-empty constraints.
    /// `cwd_prefix` matches `cwd == prefix` or `cwd` starts with `prefix + "/"`.
    /// This avoids treating `/project` as a prefix of `/project2`.
    /// Pure function; no heap allocation. Complexity: O(phases count + prefix length).
    pub fn matches(&self, ctx: &HookContext) -> bool {
        let phase_ok = self.phases.is_empty() || self.phases.contains(&ctx.phase);
        let tool_ok = self
            .tool_name
            .as_deref()
            .is_none_or(|name| ctx.tool_name.as_deref() == Some(name));
        let cwd_ok = self.cwd_prefix.as_deref().is_none_or(|prefix| {
            ctx.cwd.as_deref().is_some_and(|cwd| {
                // Exact match or child path. `starts_with(prefix)` guarantees
                // `prefix.len()` is a char boundary in `cwd`, making the slice safe.
                cwd == prefix || (cwd.starts_with(prefix) && cwd[prefix.len()..].starts_with('/'))
            })
        });
        phase_ok && tool_ok && cwd_ok
    }
}

/// A [`PreHook`] wrapper that runs the inner hook only when `matcher` passes.
/// On mismatch, returns `HookAction::Noop` without invoking the inner hook.
/// Allocation: none per call when matcher fails. Complexity: O(matcher check).
pub struct FilteredPreHook<H: PreHook> {
    inner: H,
    matcher: HookMatcher,
}

impl<H: PreHook> FilteredPreHook<H> {
    /// Wrap `hook` so it only fires when `matcher` passes.
    /// Allocation: one HookMatcher clone. Complexity: O(1).
    pub fn new(hook: H, matcher: HookMatcher) -> Self {
        Self {
            inner: hook,
            matcher,
        }
    }
}

impl<H: PreHook> PreHook for FilteredPreHook<H> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn call<'a>(&'a self, ctx: &'a HookContext) -> HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async move {
            if self.matcher.matches(ctx) {
                self.inner.call(ctx).await
            } else {
                Ok(HookAction::Noop)
            }
        })
    }
}

/// A [`PostHook`] wrapper that runs the inner hook only when `matcher` passes.
/// On mismatch, returns `Ok(())` without invoking the inner hook.
/// Allocation: none per call when matcher fails. Complexity: O(matcher check).
pub struct FilteredPostHook<H: PostHook> {
    inner: H,
    matcher: HookMatcher,
}

impl<H: PostHook> FilteredPostHook<H> {
    /// Wrap `hook` so it only fires when `matcher` passes.
    /// Allocation: one HookMatcher clone. Complexity: O(1).
    pub fn new(hook: H, matcher: HookMatcher) -> Self {
        Self {
            inner: hook,
            matcher,
        }
    }
}

impl<H: PostHook> PostHook for FilteredPostHook<H> {
    fn name(&self) -> &'static str {
        self.inner.name()
    }

    fn call<'a>(&'a self, ctx: &'a HookContext) -> HookFuture<'a, Result<(), HookIssue>> {
        Box::pin(async move {
            if self.matcher.matches(ctx) {
                self.inner.call(ctx).await
            } else {
                Ok(())
            }
        })
    }
}

#[cfg(test)]
mod tests;
