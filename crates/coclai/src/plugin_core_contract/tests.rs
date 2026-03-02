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
