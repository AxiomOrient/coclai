use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use crate::plugin::{
    BlockReason, HookAction, HookContext, HookIssue, HookPhase, HookReport, PostHook, PreHook,
};

#[derive(Clone, Default)]
pub struct RuntimeHookConfig {
    pub pre_hooks: Vec<Arc<dyn PreHook>>,
    pub post_hooks: Vec<Arc<dyn PostHook>>,
    /// Hooks that fire specifically for PreToolUse phase via the internal approval loop.
    /// When non-empty, the runtime manages the approval channel internally and auto-escalates
    /// ApprovalPolicy from Never → Untrusted so codex sends approval requests.
    pub pre_tool_use_hooks: Vec<Arc<dyn PreHook>>,
}

impl std::fmt::Debug for RuntimeHookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeHookConfig")
            .field("pre_hooks", &hook_names(&self.pre_hooks))
            .field("post_hooks", &hook_names(&self.post_hooks))
            .field("pre_tool_use_hooks", &hook_names(&self.pre_tool_use_hooks))
            .finish()
    }
}

impl PartialEq for RuntimeHookConfig {
    fn eq(&self, other: &Self) -> bool {
        hook_names(&self.pre_hooks) == hook_names(&other.pre_hooks)
            && hook_names(&self.post_hooks) == hook_names(&other.post_hooks)
            && hook_names(&self.pre_tool_use_hooks) == hook_names(&other.pre_tool_use_hooks)
    }
}

impl Eq for RuntimeHookConfig {}

impl RuntimeHookConfig {
    /// Create empty hook config.
    /// Allocation: none. Complexity: O(1).
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one pre hook.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.pre_hooks.push(hook);
        self
    }

    /// Register one post hook.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.post_hooks.push(hook);
        self
    }

    /// Register one pre-tool-use hook (fires in PreToolUse phase via the approval loop).
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_pre_tool_use_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.pre_tool_use_hooks.push(hook);
        self
    }

    /// True when at least one tool-use hook is registered.
    /// Allocation: none. Complexity: O(1).
    pub fn has_pre_tool_use_hooks(&self) -> bool {
        !self.pre_tool_use_hooks.is_empty()
    }

    /// True when at least one hook of any kind is configured.
    /// Allocation: none. Complexity: O(1).
    pub fn is_empty(&self) -> bool {
        self.pre_hooks.is_empty()
            && self.post_hooks.is_empty()
            && self.pre_tool_use_hooks.is_empty()
    }
}

/// Merge default hooks with overlay hooks.
/// Ordering is overlay-first so duplicate names prefer overlay entries.
pub(crate) fn merge_hook_configs(
    defaults: &RuntimeHookConfig,
    overlay: &RuntimeHookConfig,
) -> RuntimeHookConfig {
    if defaults.is_empty() {
        return overlay.clone();
    }
    if overlay.is_empty() {
        return defaults.clone();
    }
    RuntimeHookConfig {
        pre_hooks: merge_preferred_hooks(&overlay.pre_hooks, &defaults.pre_hooks),
        post_hooks: merge_preferred_hooks(&overlay.post_hooks, &defaults.post_hooks),
        pre_tool_use_hooks: merge_preferred_hooks(
            &overlay.pre_tool_use_hooks,
            &defaults.pre_tool_use_hooks,
        ),
    }
}

pub(crate) struct HookKernel {
    pre_hooks: RwLock<Vec<Arc<dyn PreHook>>>,
    post_hooks: RwLock<Vec<Arc<dyn PostHook>>>,
    pre_tool_use_hooks: RwLock<Vec<Arc<dyn PreHook>>>,
    thread_scoped_pre_tool_use_hooks: RwLock<HashMap<String, Vec<Arc<dyn PreHook>>>>,
    latest_report: RwLock<HookReport>,
}

#[derive(Clone, Debug)]
pub(crate) struct PreHookDecision {
    pub hook_name: String,
    pub action: HookAction,
}

impl HookKernel {
    pub(crate) fn new(config: RuntimeHookConfig) -> Self {
        Self {
            pre_hooks: RwLock::new(config.pre_hooks),
            post_hooks: RwLock::new(config.post_hooks),
            pre_tool_use_hooks: RwLock::new(config.pre_tool_use_hooks),
            thread_scoped_pre_tool_use_hooks: RwLock::new(HashMap::new()),
            latest_report: RwLock::new(HookReport::default()),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        rwlock_len(&self.pre_hooks) > 0
            || rwlock_len(&self.post_hooks) > 0
            || rwlock_len(&self.pre_tool_use_hooks) > 0
    }

    /// True when at least one pre-tool-use hook is registered.
    /// Allocation: none (read lock only). Complexity: O(1).
    pub(crate) fn has_pre_tool_use_hooks(&self) -> bool {
        rwlock_len(&self.pre_tool_use_hooks) > 0
            || match self.thread_scoped_pre_tool_use_hooks.read() {
                Ok(guard) => guard.values().any(|hooks| !hooks.is_empty()),
                Err(poisoned) => poisoned
                    .into_inner()
                    .values()
                    .any(|hooks| !hooks.is_empty()),
            }
    }

    pub(crate) fn register_thread_scoped_pre_tool_use_hooks(
        &self,
        thread_id: &str,
        hooks: &[Arc<dyn PreHook>],
    ) {
        if hooks.is_empty() {
            return;
        }
        let mut guard = match self.thread_scoped_pre_tool_use_hooks.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let entry = guard.entry(thread_id.to_owned()).or_default();
        let mut names: HashSet<&'static str> = entry.iter().map(|hook| hook.hook_name()).collect();
        for hook in hooks {
            if names.insert(hook.hook_name()) {
                entry.push(Arc::clone(hook));
            }
        }
    }

    pub(crate) fn clear_thread_scoped_pre_tool_use_hooks(&self, thread_id: &str) {
        let mut guard = match self.thread_scoped_pre_tool_use_hooks.write() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.remove(thread_id);
    }

    /// Register additional hooks into runtime kernel.
    /// Duplicate names are ignored to keep execution deterministic.
    /// Allocation: O(n) for name set snapshot. Complexity: O(n + m), n=existing, m=incoming.
    pub(crate) fn register(&self, config: RuntimeHookConfig) {
        if config.is_empty() {
            return;
        }
        register_dedup_hooks(&self.pre_hooks, config.pre_hooks);
        register_dedup_hooks(&self.post_hooks, config.post_hooks);
        register_dedup_hooks(&self.pre_tool_use_hooks, config.pre_tool_use_hooks);
    }

    pub(crate) fn report_snapshot(&self) -> HookReport {
        match self.latest_report.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub(crate) fn set_latest_report(&self, report: HookReport) {
        match self.latest_report.write() {
            Ok(mut guard) => *guard = report,
            Err(poisoned) => *poisoned.into_inner() = report,
        }
    }

    /// Execute global pre hooks plus optional scoped hooks for one call.
    /// Scoped hooks are appended after globals and deduplicated by hook name.
    /// Returns `Err(BlockReason)` on the first hook that returns `HookAction::Block`.
    /// Subsequent hooks are not executed. Allocation: O(n) decisions vec.
    pub(crate) async fn run_pre_with(
        &self,
        ctx: &HookContext,
        report: &mut HookReport,
        scoped: Option<&RuntimeHookConfig>,
    ) -> Result<Vec<PreHookDecision>, BlockReason> {
        let hooks = merge_owned_with_overlay(
            read_rwlock_vec(&self.pre_hooks),
            scoped.map(|cfg| cfg.pre_hooks.as_slice()),
        );
        let mut decisions = Vec::with_capacity(hooks.len());
        for hook in hooks {
            match hook.call(ctx).await {
                Ok(HookAction::Block(reason)) => return Err(reason),
                Ok(action) => decisions.push(PreHookDecision {
                    hook_name: hook.name().to_owned(),
                    action,
                }),
                Err(issue) => report.push(normalize_issue(issue, hook.name(), ctx.phase)),
            }
        }
        Ok(decisions)
    }

    /// Execute pre-tool-use hooks for one approval request.
    /// Returns `Err(BlockReason)` on the first hook that blocks (→ deny approval).
    /// Returns `Ok(())` when all hooks pass (→ approve).
    /// Allocation: O(n) hook vec clone. Complexity: O(n), n = hook count.
    pub(crate) async fn run_pre_tool_use_with(
        &self,
        ctx: &HookContext,
        report: &mut HookReport,
    ) -> Result<(), BlockReason> {
        let mut hooks = read_rwlock_vec(&self.pre_tool_use_hooks);
        if let Some(thread_id) = ctx.thread_id.as_deref() {
            let scoped = self.thread_scoped_pre_tool_use_hooks_for(thread_id);
            hooks = merge_owned_with_overlay(hooks, scoped.as_deref());
        }
        for hook in hooks {
            match hook.call(ctx).await {
                Ok(HookAction::Block(reason)) => return Err(reason),
                Ok(_) => {}
                Err(issue) => report.push(normalize_issue(issue, hook.name(), ctx.phase)),
            }
        }
        Ok(())
    }

    fn thread_scoped_pre_tool_use_hooks_for(
        &self,
        thread_id: &str,
    ) -> Option<Vec<Arc<dyn PreHook>>> {
        let guard = match self.thread_scoped_pre_tool_use_hooks.read() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        guard.get(thread_id).cloned()
    }

    /// Execute global post hooks plus optional scoped hooks for one call.
    /// Scoped hooks are appended after globals and deduplicated by hook name.
    pub(crate) async fn run_post_with(
        &self,
        ctx: &HookContext,
        report: &mut HookReport,
        scoped: Option<&RuntimeHookConfig>,
    ) {
        let hooks = merge_owned_with_overlay(
            read_rwlock_vec(&self.post_hooks),
            scoped.map(|cfg| cfg.post_hooks.as_slice()),
        );
        for hook in hooks {
            if let Err(issue) = hook.call(ctx).await {
                report.push(normalize_issue(issue, hook.name(), ctx.phase));
            }
        }
    }
}

fn normalize_issue(mut issue: HookIssue, fallback_name: &str, phase: HookPhase) -> HookIssue {
    if issue.hook_name.trim().is_empty() {
        issue.hook_name = fallback_name.to_owned();
    }
    issue.phase = phase;
    issue
}

fn hook_names<T>(hooks: &[Arc<T>]) -> Vec<&'static str>
where
    T: ?Sized + HookName,
{
    hooks.iter().map(|hook| hook.hook_name()).collect()
}

trait HookName {
    fn hook_name(&self) -> &'static str;
}

impl HookName for dyn PreHook {
    fn hook_name(&self) -> &'static str {
        self.name()
    }
}

impl HookName for dyn PostHook {
    fn hook_name(&self) -> &'static str {
        self.name()
    }
}

/// Read the length of a poisoning-safe RwLock hook vec without cloning.
/// Allocation: none. Complexity: O(1).
fn rwlock_len<T: ?Sized>(target: &RwLock<Vec<Arc<T>>>) -> usize {
    match target.read() {
        Ok(guard) => guard.len(),
        Err(poisoned) => poisoned.into_inner().len(),
    }
}

/// Read a poisoning-safe RwLock clone of the hook vec.
/// Allocation: clones Vec + its Arc entries. Complexity: O(n), n=hook count.
fn read_rwlock_vec<T: ?Sized>(target: &RwLock<Vec<Arc<T>>>) -> Vec<Arc<T>> {
    match target.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn merge_preferred_hooks<T>(preferred: &[Arc<T>], fallback: &[Arc<T>]) -> Vec<Arc<T>>
where
    T: ?Sized + HookName,
{
    let mut merged = Vec::with_capacity(preferred.len() + fallback.len());
    let mut names: HashSet<&'static str> = HashSet::with_capacity(preferred.len() + fallback.len());
    for hook in preferred {
        if names.insert(hook.hook_name()) {
            merged.push(Arc::clone(hook));
        }
    }
    for hook in fallback {
        if names.insert(hook.hook_name()) {
            merged.push(Arc::clone(hook));
        }
    }
    merged
}

fn merge_owned_with_overlay<T>(mut base: Vec<Arc<T>>, overlay: Option<&[Arc<T>]>) -> Vec<Arc<T>>
where
    T: ?Sized + HookName,
{
    let Some(overlay) = overlay else {
        return base;
    };
    if overlay.is_empty() {
        return base;
    }
    let mut names: HashSet<&'static str> = base.iter().map(|hook| hook.hook_name()).collect();
    for hook in overlay {
        if names.insert(hook.hook_name()) {
            base.push(Arc::clone(hook));
        }
    }
    base
}

/// Register incoming hooks deduplicating by name. Poison-safe.
/// Allocation: one HashSet per call. Complexity: O(n + m), n=existing, m=incoming.
fn register_dedup_hooks<T>(target: &RwLock<Vec<Arc<T>>>, incoming: Vec<Arc<T>>)
where
    T: ?Sized + HookName,
{
    let mut guard = match target.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut names: HashSet<&'static str> = guard.iter().map(|hook| hook.hook_name()).collect();
    for hook in incoming {
        if names.insert(hook.hook_name()) {
            guard.push(hook);
        }
    }
}
