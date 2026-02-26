use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use coclai_plugin_core::{
    HookAction, HookContext, HookIssue, HookPhase, HookReport, PostHook, PreHook,
};

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
        register_pre_hooks(&self.pre_hooks, config.pre_hooks);
        register_post_hooks(&self.post_hooks, config.post_hooks);
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
        let hooks = merged_pre_hooks(read_pre_hooks(&self.pre_hooks), scoped);
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
        let hooks = merged_post_hooks(read_post_hooks(&self.post_hooks), scoped);
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

fn read_pre_hooks(target: &RwLock<Vec<Arc<dyn PreHook>>>) -> Vec<Arc<dyn PreHook>> {
    match target.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn read_post_hooks(target: &RwLock<Vec<Arc<dyn PostHook>>>) -> Vec<Arc<dyn PostHook>> {
    match target.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn merged_pre_hooks(
    mut hooks: Vec<Arc<dyn PreHook>>,
    scoped: Option<&RuntimeHookConfig>,
) -> Vec<Arc<dyn PreHook>> {
    let Some(scoped) = scoped else {
        return hooks;
    };
    if scoped.pre_hooks.is_empty() {
        return hooks;
    }
    let mut names: HashSet<&'static str> = hooks.iter().map(|hook| hook.name()).collect();
    for hook in &scoped.pre_hooks {
        if names.insert(hook.name()) {
            hooks.push(Arc::clone(hook));
        }
    }
    hooks
}

fn merged_post_hooks(
    mut hooks: Vec<Arc<dyn PostHook>>,
    scoped: Option<&RuntimeHookConfig>,
) -> Vec<Arc<dyn PostHook>> {
    let Some(scoped) = scoped else {
        return hooks;
    };
    if scoped.post_hooks.is_empty() {
        return hooks;
    }
    let mut names: HashSet<&'static str> = hooks.iter().map(|hook| hook.name()).collect();
    for hook in &scoped.post_hooks {
        if names.insert(hook.name()) {
            hooks.push(Arc::clone(hook));
        }
    }
    hooks
}

fn register_pre_hooks(target: &RwLock<Vec<Arc<dyn PreHook>>>, incoming: Vec<Arc<dyn PreHook>>) {
    let mut guard = match target.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut names: HashSet<&'static str> = guard.iter().map(|hook| hook.name()).collect();
    for hook in incoming {
        if names.insert(hook.name()) {
            guard.push(hook);
        }
    }
}

fn register_post_hooks(target: &RwLock<Vec<Arc<dyn PostHook>>>, incoming: Vec<Arc<dyn PostHook>>) {
    let mut guard = match target.write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut names: HashSet<&'static str> = guard.iter().map(|hook| hook.name()).collect();
    for hook in incoming {
        if names.insert(hook.name()) {
            guard.push(hook);
        }
    }
}
