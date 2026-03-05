use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use crate::plugin::{HookAction, HookContext, HookIssue, HookPhase, HookReport, PostHook, PreHook};

#[derive(Clone, Default)]
pub struct RuntimeHookConfig {
    pub pre_hooks: Vec<Arc<dyn PreHook>>,
    pub post_hooks: Vec<Arc<dyn PostHook>>,
}

impl std::fmt::Debug for RuntimeHookConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeHookConfig")
            .field("pre_hooks", &hook_names(&self.pre_hooks))
            .field("post_hooks", &hook_names(&self.post_hooks))
            .finish()
    }
}

impl PartialEq for RuntimeHookConfig {
    fn eq(&self, other: &Self) -> bool {
        hook_names(&self.pre_hooks) == hook_names(&other.pre_hooks)
            && hook_names(&self.post_hooks) == hook_names(&other.post_hooks)
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

    /// True when at least one hook is configured.
    /// Allocation: none. Complexity: O(1).
    pub fn is_empty(&self) -> bool {
        self.pre_hooks.is_empty() && self.post_hooks.is_empty()
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
    }
}

pub(crate) struct HookKernel {
    pre_hooks: RwLock<Vec<Arc<dyn PreHook>>>,
    post_hooks: RwLock<Vec<Arc<dyn PostHook>>>,
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
            latest_report: RwLock::new(HookReport::default()),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        let pre_count = match self.pre_hooks.read() {
            Ok(guard) => guard.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        };
        if pre_count > 0 {
            return true;
        }
        let post_count = match self.post_hooks.read() {
            Ok(guard) => guard.len(),
            Err(poisoned) => poisoned.into_inner().len(),
        };
        post_count > 0
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
    pub(crate) async fn run_pre_with(
        &self,
        ctx: &HookContext,
        report: &mut HookReport,
        scoped: Option<&RuntimeHookConfig>,
    ) -> Vec<PreHookDecision> {
        let hooks = merge_owned_with_overlay(
            read_rwlock_vec(&self.pre_hooks),
            scoped.map(|cfg| cfg.pre_hooks.as_slice()),
        );
        let mut decisions = Vec::with_capacity(hooks.len());
        for hook in hooks {
            match hook.call(ctx).await {
                Ok(action) => decisions.push(PreHookDecision {
                    hook_name: hook.name().to_owned(),
                    action,
                }),
                Err(issue) => report.push(normalize_issue(issue, hook.name(), ctx.phase)),
            }
        }
        decisions
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
