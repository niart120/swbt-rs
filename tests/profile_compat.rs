use std::{env, fs, path::PathBuf};

use serde_json::{Value, json};
use swbt::{ErrorKind, PairingProfile, model};

const PROFILE_FIXTURE_IDS: [&str; 6] = [
    "adapter_default_with_classic_link_key",
    "joycon_l_adapter_default_with_classic_link_key",
    "joycon_r_adapter_default_with_classic_link_key",
    "local_address_with_classic_link_key",
    "joycon_l_local_address_with_classic_link_key",
    "joycon_r_local_address_with_classic_link_key",
];

#[test]
fn typed_profile_preserves_unknown_fields_and_writes_deterministic_python_json() {
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
    let mut input = input.clone();
    input["future_extension"] = json!({
        "opaque": [3, 1, 2]
    });
    let namespace = input["key_store"]["namespaces"]
        .as_object()
        .and_then(|namespaces| namespaces.keys().next())
        .expect("fixture has one local namespace")
        .clone();
    input["key_store"]["namespaces"][&namespace]["98:B6:E9:11:22:33"]["future_key_metadata"] = json!({
        "marker": "preserve-me"
    });

    let profile = PairingProfile::<M>::from_json(
        &serde_json::to_vec(&input).expect("serialize input profile"),
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
        input,
        "known and unknown profile fields must be lossless"
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
