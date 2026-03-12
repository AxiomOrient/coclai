use serde_json::json;

use crate::plugin::{
    FilteredPostHook, FilteredPreHook, HookAction, HookContext, HookFuture, HookIssue, HookMatcher,
    HookPhase, PostHook, PreHook,
};

// ── HookContext test fixture ──────────────────────────────────────────────────

fn ctx(phase: HookPhase, tool_name: Option<&str>, cwd: Option<&str>) -> HookContext {
    HookContext {
        phase,
        thread_id: None,
        turn_id: None,
        cwd: cwd.map(ToOwned::to_owned),
        model: None,
        main_status: None,
        correlation_id: "hk-test".to_owned(),
        ts_ms: 0,
        metadata: json!({}),
        tool_name: tool_name.map(ToOwned::to_owned),
        tool_input: None,
    }
}

// ── HookMatcher::matches ──────────────────────────────────────────────────────

#[test]
fn default_matcher_matches_all() {
    let m = HookMatcher::default();
    assert!(m.matches(&ctx(HookPhase::PreRun, None, None)));
    assert!(m.matches(&ctx(HookPhase::PreToolUse, Some("cargo"), Some("/proj"))));
}

#[test]
fn phases_filter_matches_included_phase() {
    let m = HookMatcher::phases(vec![HookPhase::PreRun, HookPhase::PostRun]);
    assert!(m.matches(&ctx(HookPhase::PreRun, None, None)));
    assert!(m.matches(&ctx(HookPhase::PostRun, None, None)));
}

#[test]
fn phases_filter_rejects_excluded_phase() {
    let m = HookMatcher::phases(vec![HookPhase::PreRun]);
    assert!(!m.matches(&ctx(HookPhase::PreTurn, None, None)));
    assert!(!m.matches(&ctx(HookPhase::PreToolUse, Some("cargo"), None)));
}

#[test]
fn tool_name_filter_matches_exact() {
    let m = HookMatcher::default().with_tool_name("cargo");
    assert!(m.matches(&ctx(HookPhase::PreToolUse, Some("cargo"), None)));
}

#[test]
fn tool_name_filter_rejects_different_name() {
    let m = HookMatcher::default().with_tool_name("cargo");
    assert!(!m.matches(&ctx(HookPhase::PreToolUse, Some("npm"), None)));
}

#[test]
fn tool_name_filter_rejects_none_tool() {
    let m = HookMatcher::default().with_tool_name("cargo");
    assert!(!m.matches(&ctx(HookPhase::PreRun, None, None)));
}

#[test]
fn cwd_prefix_matches_child_path() {
    let m = HookMatcher::default().with_cwd_prefix("/project");
    assert!(m.matches(&ctx(HookPhase::PreRun, None, Some("/project/src"))));
}

#[test]
fn cwd_prefix_matches_exact_path() {
    let m = HookMatcher::default().with_cwd_prefix("/project");
    assert!(m.matches(&ctx(HookPhase::PreRun, None, Some("/project"))));
}

#[test]
fn cwd_prefix_rejects_sibling_with_shared_prefix() {
    // "/project2" must NOT match prefix "/project"
    let m = HookMatcher::default().with_cwd_prefix("/project");
    assert!(!m.matches(&ctx(HookPhase::PreRun, None, Some("/project2"))));
}

#[test]
fn cwd_prefix_rejects_non_prefix() {
    let m = HookMatcher::default().with_cwd_prefix("/project");
    assert!(!m.matches(&ctx(HookPhase::PreRun, None, Some("/other/path"))));
}

#[test]
fn cwd_prefix_rejects_none_cwd() {
    let m = HookMatcher::default().with_cwd_prefix("/project");
    assert!(!m.matches(&ctx(HookPhase::PreRun, None, None)));
}

#[test]
fn combined_filters_all_must_pass() {
    let m = HookMatcher::phases(vec![HookPhase::PreToolUse])
        .with_tool_name("cargo")
        .with_cwd_prefix("/repo");
    // all pass
    assert!(m.matches(&ctx(
        HookPhase::PreToolUse,
        Some("cargo"),
        Some("/repo/crate")
    )));
    // wrong phase
    assert!(!m.matches(&ctx(HookPhase::PreRun, Some("cargo"), Some("/repo/crate"))));
    // wrong tool
    assert!(!m.matches(&ctx(
        HookPhase::PreToolUse,
        Some("npm"),
        Some("/repo/crate")
    )));
    // wrong cwd
    assert!(!m.matches(&ctx(HookPhase::PreToolUse, Some("cargo"), Some("/other"))));
}

// ── FilteredPreHook ───────────────────────────────────────────────────────────

struct SpyPreHook;

impl PreHook for SpyPreHook {
    fn name(&self) -> &'static str {
        "spy"
    }

    fn call<'a>(&'a self, _ctx: &'a HookContext) -> HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async { Ok(HookAction::Noop) })
    }
}

// FilteredPreHook delegates name from inner hook.
#[test]
fn filtered_pre_hook_name_delegates() {
    let hook = FilteredPreHook::new(SpyPreHook, HookMatcher::default());
    assert_eq!(hook.name(), "spy");
}

#[tokio::test]
async fn filtered_pre_hook_passes_when_matcher_matches() {
    let hook = FilteredPreHook::new(SpyPreHook, HookMatcher::phases(vec![HookPhase::PreRun]));
    let c = ctx(HookPhase::PreRun, None, None);
    let result = hook.call(&c).await;
    assert_eq!(result, Ok(HookAction::Noop));
}

#[tokio::test]
async fn filtered_pre_hook_noop_when_matcher_fails() {
    // Only PreToolUse, but calling with PreRun
    let hook = FilteredPreHook::new(SpyPreHook, HookMatcher::phases(vec![HookPhase::PreToolUse]));
    let c = ctx(HookPhase::PreRun, None, None);
    let result = hook.call(&c).await;
    assert_eq!(result, Ok(HookAction::Noop));
}

// ── FilteredPostHook ──────────────────────────────────────────────────────────

struct SpyPostHook;

impl PostHook for SpyPostHook {
    fn name(&self) -> &'static str {
        "spy-post"
    }

    fn call<'a>(&'a self, _ctx: &'a HookContext) -> HookFuture<'a, Result<(), HookIssue>> {
        Box::pin(async { Ok(()) })
    }
}

#[test]
fn filtered_post_hook_name_delegates() {
    let hook = FilteredPostHook::new(SpyPostHook, HookMatcher::default());
    assert_eq!(hook.name(), "spy-post");
}

#[tokio::test]
async fn filtered_post_hook_ok_when_matcher_fails() {
    let hook = FilteredPostHook::new(SpyPostHook, HookMatcher::phases(vec![HookPhase::PostRun]));
    let c = ctx(HookPhase::PreRun, None, None);
    let result = hook.call(&c).await;
    assert_eq!(result, Ok(()));
}

#[tokio::test]
async fn filtered_post_hook_delegates_when_matcher_passes() {
    let hook = FilteredPostHook::new(SpyPostHook, HookMatcher::phases(vec![HookPhase::PostRun]));
    let c = ctx(HookPhase::PostRun, None, None);
    let result = hook.call(&c).await;
    assert_eq!(result, Ok(()));
}
