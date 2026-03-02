use std::collections::BTreeSet;

use crate::capability::{
    capability_by_id, capability_parity_gaps, capability_registry,
    missing_capabilities_for_ingress, render_capability_parity_report, CapabilityExposure,
    CapabilityIngress,
};
use crate::rpc_contract::methods::KNOWN;

#[test]
fn registry_has_unique_capability_ids() {
    let mut unique = BTreeSet::new();
    for capability in capability_registry() {
        assert!(
            unique.insert(capability.capability_id),
            "duplicate capability id: {}",
            capability.capability_id
        );
    }
    assert!(!unique.is_empty(), "capability registry must not be empty");
}

#[test]
fn registry_covers_all_known_rpc_methods() {
    for method in KNOWN {
        let capability = capability_by_id(method);
        assert!(
            capability.is_some(),
            "known rpc method is missing from capability registry: {method}"
        );
    }
}

#[test]
fn stdio_path_is_marked_available_for_every_capability() {
    let missing_stdio = missing_capabilities_for_ingress(CapabilityIngress::Stdio);
    assert!(
        missing_stdio.is_empty(),
        "stdio should expose every capability; missing={}",
        missing_stdio.len()
    );
}

#[test]
fn parity_gap_list_matches_report_rows() {
    let report = render_capability_parity_report();
    let row_count = report
        .lines()
        .filter(|line| line.starts_with("- capability_id: "))
        .count();
    assert_eq!(row_count, capability_registry().len());

    let gap_count = capability_parity_gaps().len();
    assert!(
        report.contains(&format!("- full_parity_gaps: {gap_count}")),
        "report must include full_parity_gaps summary"
    );
}

#[test]
fn http_and_ws_ingress_are_available_for_every_capability() {
    for capability in capability_registry() {
        assert_eq!(
            capability.ingress.http_localhost,
            CapabilityExposure::Available,
            "http ingress status must be available for {}",
            capability.capability_id
        );
        assert_eq!(
            capability.ingress.ws_localhost,
            CapabilityExposure::Available,
            "ws ingress status must be available for {}",
            capability.capability_id
        );
    }
}
