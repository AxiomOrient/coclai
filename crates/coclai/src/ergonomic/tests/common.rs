use super::super::*;
use coclai_runtime::{HookAction, HookContext, HookIssue, HookPatch};
use std::future::Future;
use std::pin::Pin;

pub(super) struct TestPreHook;
pub(super) struct TestPostHook;

impl PreHook for TestPreHook {
    fn name(&self) -> &'static str {
        "test_pre"
    }

    fn call<'a>(
        &'a self,
        _ctx: &'a HookContext,
    ) -> Pin<Box<dyn Future<Output = Result<HookAction, HookIssue>> + Send + 'a>> {
        Box::pin(async { Ok(HookAction::Mutate(HookPatch::default())) })
    }
}

impl PostHook for TestPostHook {
    fn name(&self) -> &'static str {
        "test_post"
    }

    fn call<'a>(
        &'a self,
        _ctx: &'a HookContext,
    ) -> Pin<Box<dyn Future<Output = Result<(), HookIssue>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}
