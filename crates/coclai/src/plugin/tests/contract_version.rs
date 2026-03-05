use super::*;

#[test]
fn plugin_contract_major_must_match() {
    let current = PluginContractVersion::CURRENT;
    assert!(current.is_compatible_with(PluginContractVersion::new(1, 99)));
    assert!(!current.is_compatible_with(PluginContractVersion::new(2, 0)));
}

#[test]
fn plugin_contract_current_constant_is_stable() {
    let current = PluginContractVersion::CURRENT;
    assert_eq!(current, PluginContractVersion::new(1, 0));
}
