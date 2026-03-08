use super::*;

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

#[test]
fn hook_patch_default_is_noop() {
    let patch = HookPatch::default();
    assert!(patch.prompt_override.is_none());
    assert!(patch.model_override.is_none());
    assert!(patch.add_attachments.is_empty());
    assert_eq!(patch.metadata_delta, serde_json::Value::Null);
}
