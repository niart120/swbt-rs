use std::collections::BTreeSet;

use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/python-v0.6.0/hid/hid-service-fixtures.json");
const SOURCE_COMMIT: &str = "84d2723b127f70fc78e12f4496f5c40af0ccfb0a";
const DESCRIPTOR_SHA256: &str = "25f0b3b7e59bdfec05e8cced16e43a8878509865a0cb223f05025c556f3bedba";

fn fixture_document() -> Value {
    serde_json::from_str(FIXTURE).expect("generated HID service fixture must be valid JSON")
}

#[test]
fn hid_service_fixture_has_pinned_reproducible_provenance() {
    let fixture = fixture_document();

    assert_eq!(fixture["format"], "swbt.hid-service-fixtures");
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["source_repository"],
        "https://github.com/niart120/swbt-python"
    );
    assert_eq!(fixture["source_commit"], SOURCE_COMMIT);
    assert_eq!(fixture["source_version"], "0.6.0");
    assert_eq!(fixture["python_version"], "3.13");
    assert_eq!(
        fixture["generator"],
        "tools/generate_python_hid_service_fixtures.py"
    );
    assert_eq!(fixture["descriptor"]["length"], 203);
    assert_eq!(fixture["descriptor"]["sha256"], DESCRIPTOR_SHA256);
    assert_eq!(
        fixture["descriptor"]["hex"]
            .as_str()
            .expect("descriptor hex")
            .len(),
        203 * 2
    );

    let source_paths = fixture["source_paths"]
        .as_array()
        .expect("source_paths must be an array");
    assert_eq!(source_paths.len(), 5);
    assert!(source_paths.iter().all(|path| {
        path.as_str()
            .is_some_and(|path| path.starts_with("src/swbt/"))
    }));
}

#[test]
fn hid_service_fixture_contains_exactly_the_supported_models_and_complete_policy() {
    let fixture = fixture_document();
    let models = fixture["models"]
        .as_object()
        .expect("models must be an object");
    assert_eq!(
        models.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["joycon_l", "joycon_r", "pro"])
    );

    let expected_policy_fields = BTreeSet::from([
        "bluetooth_profile_version",
        "boot_device",
        "country_code",
        "device_release_number",
        "device_subclass",
        "normally_connectable",
        "parser_version",
        "profile_version",
        "provider_name",
        "reconnect_initiate",
        "remote_wake",
        "service_description",
        "service_name",
        "ssr_host_max_latency",
        "ssr_host_min_timeout",
        "supervision_timeout",
        "virtual_cable",
    ]);
    for model in models.values() {
        assert!(model["local_name"].is_string());
        let policy = model["sdp_policy"]
            .as_object()
            .expect("SDP policy must be an object");
        assert_eq!(
            policy.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            expected_policy_fields
        );
    }
}
