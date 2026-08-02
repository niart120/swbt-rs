use std::{env, fs, path::PathBuf};

use serde_json::{Value, json};
use swbt_core::{ErrorKind, PairingProfile, model};

const PROFILE_FIXTURE_IDS: [&str; 6] = [
    "adapter_default_with_classic_link_key",
    "joycon_l_adapter_default_with_classic_link_key",
    "joycon_r_adapter_default_with_classic_link_key",
    "local_address_with_classic_link_key",
    "joycon_l_local_address_with_classic_link_key",
    "joycon_r_local_address_with_classic_link_key",
];

#[test]
fn typed_profile_writes_deterministic_python_json() {
    let fixtures = python_profile_fixtures();
    assert_eq!(fixtures.len(), PROFILE_FIXTURE_IDS.len());

    assert_typed_round_trip::<model::Pro>(&fixtures[0]);
    assert_typed_round_trip::<model::JoyConL>(&fixtures[1]);
    assert_typed_round_trip::<model::JoyConR>(&fixtures[2]);
    assert_typed_round_trip::<model::Pro>(&fixtures[3]);
    assert_typed_round_trip::<model::JoyConL>(&fixtures[4]);
    assert_typed_round_trip::<model::JoyConR>(&fixtures[5]);
}

#[test]
fn python_local_address_profiles_redact_identity_and_keys_from_debug() {
    let fixtures = python_profile_fixtures();

    assert_typed_debug_redaction::<model::Pro>(&fixtures[3], "04".repeat(16));
    assert_typed_debug_redaction::<model::JoyConL>(&fixtures[4], "05".repeat(16));
    assert_typed_debug_redaction::<model::JoyConR>(&fixtures[5], "06".repeat(16));
}

#[test]
fn typed_profile_rejects_the_opposite_joycon_fixture() {
    let fixtures = python_profile_fixtures();
    assert_eq!(fixtures.len(), PROFILE_FIXTURE_IDS.len());

    let left = serde_json::to_vec(&fixtures[1]).expect("serialize left profile");
    let right = serde_json::to_vec(&fixtures[2]).expect("serialize right profile");
    assert_eq!(
        PairingProfile::<model::JoyConR>::from_json(&left)
            .expect_err("right typed profile must reject left fixture")
            .kind(),
        ErrorKind::ProfileControllerMismatch
    );
    assert_eq!(
        PairingProfile::<model::JoyConL>::from_json(&right)
            .expect_err("left typed profile must reject right fixture")
            .kind(),
        ErrorKind::ProfileControllerMismatch
    );
}

fn assert_typed_round_trip<M: model::ControllerModel>(input: &Value) {
    let profile = PairingProfile::<M>::from_json(
        &serde_json::to_vec(input).expect("serialize input profile"),
    )
    .expect("matching Python profile must parse");
    let first = profile
        .to_json_bytes()
        .expect("typed profile must serialize");
    let second = profile
        .to_json_bytes()
        .expect("repeated serialization must succeed");

    assert_eq!(first, second);
    assert_eq!(first.last(), Some(&b'\n'));
    assert_eq!(
        serde_json::from_slice::<Value>(&first).expect("serialized profile must remain JSON"),
        *input,
        "canonical profile fields must round-trip"
    );
    let expected = format!(
        "{}\n",
        serde_json::to_string_pretty(&input).expect("format expected canonical JSON")
    );
    assert_eq!(
        String::from_utf8(first).expect("profile output must be UTF-8"),
        expected,
        "Rust output must use sorted keys, two-space indent, and one trailing newline"
    );
}

#[test]
fn typed_profile_rejects_unknown_extensions() {
    let mut input = python_profile_fixtures()
        .into_iter()
        .next()
        .expect("fixture must retain the Pro case");
    input["future_extension"] = json!({"opaque": [3, 1, 2]});

    let error = PairingProfile::<model::Pro>::from_json(
        &serde_json::to_vec(&input).expect("serialize extended profile"),
    )
    .expect_err("unknown profile fields must be rejected");

    assert_eq!(error.kind(), ErrorKind::InvalidProfile);
    assert!(!error.to_string().contains("opaque"));
    assert!(!format!("{error:?}").contains("opaque"));
}

#[test]
fn typed_profile_normalizes_lowercase_namespace_to_uppercase() {
    let mut input = python_profile_fixtures()
        .into_iter()
        .next()
        .expect("fixture must retain the Pro case");
    let namespaces = input["key_store"]["namespaces"]
        .as_object_mut()
        .expect("fixture namespaces must be an object");
    let peers = namespaces
        .remove("00:11:22:33:44:55")
        .expect("fixture namespace must exist");
    namespaces.insert("aa:bb:cc:dd:ee:ff".to_owned(), peers);

    let profile = PairingProfile::<model::Pro>::from_json(
        &serde_json::to_vec(&input).expect("serialize lowercase namespace profile"),
    )
    .expect("lowercase Bluetooth namespace must parse");
    let output: Value = serde_json::from_slice(
        &profile
            .to_json_bytes()
            .expect("normalized profile must serialize"),
    )
    .expect("normalized profile must remain JSON");

    assert!(
        output["key_store"]["namespaces"]["AA:BB:CC:DD:EE:FF"]["98:B6:E9:11:22:33/P"].is_object()
    );
}

#[test]
fn typed_profile_normalizes_lowercase_peer_to_uppercase() {
    let mut input = python_profile_fixtures()
        .into_iter()
        .next()
        .expect("fixture must retain the Pro case");
    let peers = input["key_store"]["namespaces"]["00:11:22:33:44:55"]
        .as_object_mut()
        .expect("fixture peers must be an object");
    let keys = peers
        .remove("98:B6:E9:11:22:33/P")
        .expect("fixture peer must exist");
    peers.insert("98:b6:e9:11:22:33/P".to_owned(), keys);

    let profile = PairingProfile::<model::Pro>::from_json(
        &serde_json::to_vec(&input).expect("serialize lowercase peer profile"),
    )
    .expect("lowercase Bluetooth peer must parse");
    let output: Value = serde_json::from_slice(
        &profile
            .to_json_bytes()
            .expect("normalized profile must serialize"),
    )
    .expect("normalized profile must remain JSON");

    assert!(
        output["key_store"]["namespaces"]["00:11:22:33:44:55"]["98:B6:E9:11:22:33/P"]
            .is_object()
    );
}

fn assert_typed_debug_redaction<M: model::ControllerModel>(input: &Value, key: String) {
    let profile = PairingProfile::<M>::from_json(
        &serde_json::to_vec(input).expect("serialize local-address fixture"),
    )
    .expect("matching local-address profile must parse");
    let rendered = format!("{profile:?}");

    assert!(!rendered.contains("02:12:34:56:78:9A"));
    assert!(!rendered.contains(&key));
    assert!(rendered.contains("<redacted>"));
}

#[test]
#[ignore = "manual cross-language gate; set SWBT_PROFILE_COMPAT_OUTPUT"]
fn write_rust_profile_for_pinned_python_reader() {
    let output = PathBuf::from(
        env::var_os("SWBT_PROFILE_COMPAT_OUTPUT")
            .expect("SWBT_PROFILE_COMPAT_OUTPUT must name the output file"),
    );
    let input = python_profile_fixtures()
        .into_iter()
        .next()
        .expect("fixture must retain the Pro case");
    let profile = PairingProfile::<model::Pro>::from_json(
        &serde_json::to_vec(&input).expect("serialize input profile"),
    )
    .expect("matching Python profile must parse");

    fs::write(
        output,
        profile
            .to_json_bytes()
            .expect("typed profile must serialize"),
    )
    .expect("write Rust profile for the Python reader");
}

fn python_profile_fixtures() -> Vec<Value> {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "fixtures/python-v0.6.0/profile/pairing-profile-fixtures.json"
    ))
    .expect("Python profile fixture must be valid JSON");
    let cases = fixture["cases"]
        .as_array()
        .expect("Python profile fixture cases must be an array");
    assert_eq!(
        cases
            .iter()
            .map(|case| case["id"].as_str().expect("fixture ID must be text"))
            .collect::<Vec<_>>(),
        PROFILE_FIXTURE_IDS
    );
    cases.iter().map(|case| case["profile"].clone()).collect()
}
