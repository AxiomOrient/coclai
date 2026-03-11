use std::sync::{Arc, Mutex};

use crate::plugin::{
    HookAction, HookContext, HookIssue, HookIssueClass, HookPhase, PostHook, PreHook,
};
use serde_json::Value;

#[derive(Clone)]
pub(crate) struct RecordingPreHook {
    pub(crate) name: &'static str,
    pub(crate) events: Arc<Mutex<Vec<String>>>,
    pub(crate) fail_phase: Option<HookPhase>,
}

impl PreHook for RecordingPreHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> crate::plugin::HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async move {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("pre:{:?}", ctx.phase));
            if self.fail_phase == Some(ctx.phase) {
                return Err(HookIssue {
                    hook_name: self.name.to_owned(),
                    phase: ctx.phase,
                    class: HookIssueClass::Execution,
                    message: "forced pre hook failure".to_owned(),
                });
            }
            Ok(HookAction::Noop)
        })
    }
}

#[derive(Clone)]
pub(crate) struct RecordingPostHook {
    pub(crate) name: &'static str,
    pub(crate) events: Arc<Mutex<Vec<String>>>,
    pub(crate) fail_phase: Option<HookPhase>,
}

impl PostHook for RecordingPostHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> crate::plugin::HookFuture<'a, Result<(), HookIssue>> {
        Box::pin(async move {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("post:{:?}", ctx.phase));
            if self.fail_phase == Some(ctx.phase) {
                return Err(HookIssue {
                    hook_name: self.name.to_owned(),
                    phase: ctx.phase,
                    class: HookIssueClass::Execution,
                    message: "forced post hook failure".to_owned(),
                });
            }
            Ok(())
        })
    }
}

#[derive(Clone)]
pub(crate) struct PhasePatchPreHook {
    pub(crate) name: &'static str,
    pub(crate) patches: Vec<(HookPhase, crate::plugin::HookPatch)>,
}

impl PreHook for PhasePatchPreHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> crate::plugin::HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async move {
            if let Some((_, patch)) = self.patches.iter().find(|(phase, _)| *phase == ctx.phase) {
                Ok(HookAction::Mutate(patch.clone()))
            } else {
                Ok(HookAction::Noop)
            }
        })
    }
}

#[derive(Clone)]
pub(crate) struct MetadataCapturePostHook {
    pub(crate) name: &'static str,
    pub(crate) metadata: Arc<Mutex<Vec<(HookPhase, Value)>>>,
}

impl PostHook for MetadataCapturePostHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> crate::plugin::HookFuture<'a, Result<(), HookIssue>> {
        Box::pin(async move {
            self.metadata
                .lock()
                .expect("metadata lock")
                .push((ctx.phase, ctx.metadata.clone()));
            Ok(())
        })
    }
}
