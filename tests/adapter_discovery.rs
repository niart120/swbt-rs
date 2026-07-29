#[cfg(not(feature = "bumble"))]
#[test]
fn discovery_without_bumble_feature_reports_unsupported_without_side_effects() {
    let error = swbt::list_adapters().expect_err("default build has no USB discovery backend");

    assert_eq!(error.kind(), swbt::ErrorKind::UnsupportedCapability);
}
