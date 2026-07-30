use std::{env, fs, path::PathBuf};

use serde_json::{Value, json};
use swbt::{PairingProfile, model};

#[test]
fn typed_profile_preserves_unknown_fields_and_writes_deterministic_python_json() {
    let mut input = python_profile_fixture();
    input["future_extension"] = json!({
        "opaque": [3, 1, 2]
    });
    input["key_store"]["namespaces"]["00:11:22:33:44:55"]["98:B6:E9:11:22:33"]["future_key_metadata"] = json!({
        "marker": "preserve-me"
    });

    let profile = PairingProfile::<model::Pro>::from_json(
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

#[test]
#[ignore = "manual cross-language gate; set SWBT_PROFILE_COMPAT_OUTPUT"]
fn write_rust_profile_for_pinned_python_reader() {
    let output = PathBuf::from(
        env::var_os("SWBT_PROFILE_COMPAT_OUTPUT")
            .expect("SWBT_PROFILE_COMPAT_OUTPUT must name the output file"),
    );
    let input = python_profile_fixture();
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

fn python_profile_fixture() -> Value {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "fixtures/python-v0.6.0/profile/pairing-profile-fixtures.json"
    ))
    .expect("Python profile fixture must be valid JSON");
    fixture["cases"][0]["profile"].clone()
}
