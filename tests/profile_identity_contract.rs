use std::time::Duration;

use swbt::{CreateProfileOptions, LocalAddress, ProfileIdentity};

#[test]
fn create_profile_options_expose_typed_identity_and_pair_timeout() {
    let address = LocalAddress::parse("02:12:34:56:78:9A").expect("valid public local address");
    let options = CreateProfileOptions {
        identity: ProfileIdentity::LocalAddress(address),
        pair_timeout: Duration::from_secs(60),
    };

    assert_eq!(options.identity, ProfileIdentity::LocalAddress(address));
    assert_eq!(options.pair_timeout, Duration::from_secs(60));
    let debug = format!("{:?}", options.identity);
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("02:12:34:56:78:9A"));
}
