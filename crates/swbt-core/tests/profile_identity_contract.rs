use swbt_core::{LocalAddress, ProfileIdentity};

#[test]
fn profile_identity_exposes_a_typed_redacted_local_address() {
    let address = LocalAddress::parse("02:12:34:56:78:9A").expect("valid public local address");
    let identity = ProfileIdentity::LocalAddress(address);

    assert_eq!(identity, ProfileIdentity::LocalAddress(address));
    let debug = format!("{identity:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("02:12:34:56:78:9A"));
}
