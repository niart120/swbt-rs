use std::error::Error as _;

use serde_json::{Value, json};

use crate::{
    error::ErrorKind,
    model::{self, ControllerModel},
};

use super::{ControllerKind, PairingProfile, ProfileDocument};

const SECRET_SENTINEL: &str = "KNOWN_PROFILE_SECRET_7E3C1A";

#[test]
fn matching_v2_envelope_converts_to_the_requested_model() {
    assert_matching_model::<model::Pro>("pro");
    assert_matching_model::<model::JoyConL>("joycon_l");
    assert_matching_model::<model::JoyConR>("joycon_r");

    let mut local_identity = valid_profile("pro");
    local_identity["identity"] = json!({
        "kind": "exp-local-address",
        "address": "02:12:34:56:78:9A"
    });
    let profile = PairingProfile::<model::Pro>::try_from(parse(local_identity))
        .expect("recognized local-address identity must parse");

    assert_eq!(profile.controller_kind(), ControllerKind::Pro);
}

#[test]
fn raw_envelope_rejects_invalid_schema_and_shape_without_guessing() {
    let malformed = ProfileDocument::parse_json(b"{");
    let malformed_error = malformed.expect_err("malformed JSON must fail");
    assert_eq!(malformed_error.kind(), ErrorKind::InvalidProfile);
    assert!(malformed_error.source().is_some());

    let invalid_utf8 = ProfileDocument::parse_json(&[0xFF]);
    let invalid_utf8_error = invalid_utf8.expect_err("invalid UTF-8 must fail");
    assert_eq!(invalid_utf8_error.kind(), ErrorKind::InvalidProfile);
    assert!(invalid_utf8_error.source().is_some());

    let mut wrong_format = valid_profile("pro");
    wrong_format["format"] = json!("other.profile");
    let mut wrong_version = valid_profile("pro");
    wrong_version["schema_version"] = json!(1);
    let mut unknown_kind = valid_profile("pro");
    unknown_kind["controller_kind"] = json!("unknown");
    let mut invalid_identity = valid_profile("pro");
    invalid_identity["identity"] = json!([]);
    let mut invalid_identity_kind = valid_profile("pro");
    invalid_identity_kind["identity"] = json!({"kind": "unknown"});
    let mut adapter_identity_with_address = valid_profile("pro");
    adapter_identity_with_address["identity"] = json!({
        "kind": "adapter-default",
        "address": "02:12:34:56:78:9A"
    });
    let mut local_identity_without_address = valid_profile("pro");
    local_identity_without_address["identity"] = json!({"kind": "exp-local-address"});
    let mut invalid_key_store = valid_profile("pro");
    invalid_key_store["key_store"] = json!([]);
    let mut invalid_namespaces = valid_profile("pro");
    invalid_namespaces["key_store"]["namespaces"] = json!([]);
    let mut missing_namespaces = valid_profile("pro");
    missing_namespaces["key_store"]
        .as_object_mut()
        .expect("test key store is an object")
        .remove("namespaces");
    let mut missing_key_store = valid_profile("pro");
    missing_key_store
        .as_object_mut()
        .expect("test profile is an object")
        .remove("key_store");

    for invalid in [
        json!([]),
        wrong_format,
        wrong_version,
        unknown_kind,
        invalid_identity,
        invalid_identity_kind,
        adapter_identity_with_address,
        local_identity_without_address,
        invalid_key_store,
        invalid_namespaces,
        missing_namespaces,
        missing_key_store,
    ] {
        let error = ProfileDocument::parse_json(invalid.to_string().as_bytes())
            .expect_err("invalid envelope must fail");
        assert_eq!(error.kind(), ErrorKind::InvalidProfile);
    }
}

#[test]
fn typed_conversion_returns_structured_model_mismatch_without_secret_echo() {
    let document = parse(valid_profile("pro"));

    let error = PairingProfile::<model::JoyConL>::try_from(document)
        .expect_err("Pro profile must not validate as Joy-Con L");

    assert_eq!(error.kind(), ErrorKind::ProfileControllerMismatch);
    assert!(!error.to_string().contains(SECRET_SENTINEL));
    assert!(!format!("{error:?}").contains(SECRET_SENTINEL));
}

#[test]
fn raw_and_typed_profile_debug_redact_key_material_and_unknown_fields() {
    let document = parse(valid_profile("pro"));
    let raw_debug = format!("{document:?}");

    assert!(raw_debug.contains("ProfileDocument"));
    assert!(raw_debug.contains("<redacted>"));
    assert!(!raw_debug.contains(SECRET_SENTINEL));
    assert!(!raw_debug.contains("link_key"));

    let profile =
        PairingProfile::<model::Pro>::try_from(document).expect("matching model must validate");
    let typed_debug = format!("{profile:?}");

    assert!(typed_debug.contains("PairingProfile"));
    assert!(typed_debug.contains("<redacted>"));
    assert!(!typed_debug.contains(SECRET_SENTINEL));
    assert!(!typed_debug.contains("link_key"));
}

fn valid_profile(controller_kind: &str) -> Value {
    json!({
        "format": "swbt.profile",
        "schema_version": 2,
        "controller_kind": controller_kind,
        "identity": {
            "kind": "adapter-default"
        },
        "key_store": {
            "namespaces": {
                "02:12:34:56:78:9A": {
                    "AA:BB:CC:DD:EE:FF": {
                        "link_key": SECRET_SENTINEL
                    }
                }
            }
        },
        "future_extension": {
            "opaque_secret": SECRET_SENTINEL
        }
    })
}

fn parse(value: Value) -> ProfileDocument {
    ProfileDocument::parse_json(value.to_string().as_bytes())
        .expect("test profile must be a valid v2 envelope")
}

fn assert_matching_model<M: ControllerModel>(kind_name: &str) {
    let document = parse(valid_profile(kind_name));

    assert_eq!(document.controller_kind(), M::KIND);

    let profile = PairingProfile::<M>::try_from(document).expect("matching model must validate");

    assert_eq!(profile.controller_kind(), M::KIND);
}
