use std::error::Error as _;

use serde_json::{Value, json};

use crate::{
    error::ErrorKind,
    model::{self, ControllerModel},
};

use super::{ControllerKind, PairingProfile, ProfileDocument};

const SECRET_SENTINEL: &str = "KNOWN_PROFILE_SECRET_7E3C1A";

#[test]
fn python_v2_fixture_converts_to_typed_pro_profile() {
    let fixture: Value = serde_json::from_slice(include_bytes!(
        "../../tests/fixtures/python-v0.6.0/profile/pairing-profile-fixtures.json"
    ))
    .expect("Python profile fixture must be valid JSON");
    assert_eq!(fixture["format"], "swbt.profile-fixtures");
    assert_eq!(fixture["source_version"], "0.6.0");
    assert_eq!(
        fixture["source_commit"],
        "84d2723b127f70fc78e12f4496f5c40af0ccfb0a"
    );
    let profile = fixture["cases"][0]["profile"].clone();

    let document = ProfileDocument::parse_json(
        &serde_json::to_vec(&profile).expect("serialize fixture profile"),
    )
    .expect("Python schema v2 profile must parse");
    let typed = PairingProfile::<model::Pro>::try_from(document)
        .expect("Python Pro profile must convert to typed Rust profile");

    assert_eq!(typed.controller_kind(), ControllerKind::Pro);
}

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
fn namespace_shape_and_known_key_fields_are_validated_without_secret_echo() {
    let invalid_namespace_cases = [
        json!([]),
        json!({"98:B6:E9:11:22:33/P": []}),
        json!({"not-an-address": {}}),
        json!({"00:11:22:33:44:55": {"not-an-address": {}}}),
        json!({
            "00:11:22:33:44:55": {
                "98:B6:E9:11:22:33/P": {},
                "98:B6:E9:44:55:66/P": {}
            }
        }),
    ];
    for namespaces in invalid_namespace_cases {
        let mut invalid = valid_profile("pro");
        invalid["key_store"]["namespaces"] = namespaces;
        assert_invalid_profile_without_secret(invalid);
    }

    let invalid_key_cases = [
        json!({"address_type": 256}),
        json!({"link_key_type": "4"}),
        json!({"link_key": SECRET_SENTINEL}),
        json!({"link_key": {"value": SECRET_SENTINEL}}),
        json!({"link_key": {"value": "123"}}),
        json!({"link_key": {"value": "00", "authenticated": "yes"}}),
        json!({
            "link_key": {"value": "00", "authenticated": true},
            "link_key_type": 4
        }),
        json!({
            "link_key": {
                "value": "01010101010101010101010101010101",
                "authenticated": true
            },
            "link_key_type": 256
        }),
        json!({"ltk": {"value": "00", "ediv": 65536}}),
        json!({"irk": {"value": "00", "rand": "xyz"}}),
        json!({"csrk": {"value": "00", "sign_counter": -1}}),
    ];
    for keys in invalid_key_cases {
        let mut invalid = valid_profile("pro");
        invalid["key_store"]["namespaces"]["02:12:34:56:78:9A"]["AA:BB:CC:DD:EE:FF/P"] = keys;
        assert_invalid_profile_without_secret(invalid);
    }
}

#[test]
fn public_peer_names_require_the_bumble_public_suffix() {
    parse(valid_profile("pro"));

    let mut raw_peer = valid_profile("pro");
    let peers = raw_peer["key_store"]["namespaces"]["02:12:34:56:78:9A"]
        .as_object_mut()
        .expect("test namespace is an object");
    let keys = peers
        .remove("AA:BB:CC:DD:EE:FF/P")
        .expect("canonical test peer exists");
    peers.insert("AA:BB:CC:DD:EE:FF".to_owned(), keys);
    assert_invalid_profile_without_secret(raw_peer);

    for invalid_peer in ["AA:BB:CC:DD:EE:FF/R", "AA:BB:CC:DD:EE:FF/Pextra"] {
        let mut invalid = valid_profile("pro");
        let peers = invalid["key_store"]["namespaces"]["02:12:34:56:78:9A"]
            .as_object_mut()
            .expect("test namespace is an object");
        let keys = peers
            .remove("AA:BB:CC:DD:EE:FF/P")
            .expect("canonical test peer exists");
        peers.insert(invalid_peer.to_owned(), keys);
        assert_invalid_profile_without_secret(invalid);
    }

    let mut typed_namespace = valid_profile("pro");
    let namespaces = typed_namespace["key_store"]["namespaces"]
        .as_object_mut()
        .expect("test namespaces is an object");
    let peers = namespaces
        .remove("02:12:34:56:78:9A")
        .expect("raw test namespace exists");
    namespaces.insert("02:12:34:56:78:9A/P".to_owned(), peers);
    assert_invalid_profile_without_secret(typed_namespace);
}

#[test]
fn canonical_classic_profile_rejects_unknown_legacy_and_non_classic_fields() {
    let mut root_extension = valid_profile("pro");
    root_extension["future_extension"] = json!({"secret": SECRET_SENTINEL});

    let mut identity_extension = valid_profile("pro");
    identity_extension["identity"]["future_identity"] = json!(SECRET_SENTINEL);

    let mut key_store_extension = valid_profile("pro");
    key_store_extension["key_store"]["future_store"] = json!(SECRET_SENTINEL);

    let mut peer_extension = valid_profile("pro");
    peer_extension["key_store"]["namespaces"]["02:12:34:56:78:9A"]["AA:BB:CC:DD:EE:FF/P"]["future_peer"] =
        json!(SECRET_SENTINEL);

    let mut key_extension = valid_profile("pro");
    key_extension["key_store"]["namespaces"]["02:12:34:56:78:9A"]["AA:BB:CC:DD:EE:FF/P"]["link_key"]
        ["future_key"] = json!(SECRET_SENTINEL);

    let mut address_type = valid_profile("pro");
    address_type["key_store"]["namespaces"]["02:12:34:56:78:9A"]["AA:BB:CC:DD:EE:FF/P"]["address_type"] =
        json!(0);

    let mut le_key = valid_profile("pro");
    le_key["key_store"]["namespaces"]["02:12:34:56:78:9A"]["AA:BB:CC:DD:EE:FF/P"]["ltk"] =
        json!({"value": "00"});

    for invalid in [
        root_extension,
        identity_extension,
        key_store_extension,
        peer_extension,
        key_extension,
        address_type,
        le_key,
    ] {
        assert_invalid_profile_without_secret(invalid);
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
fn raw_and_typed_profile_debug_redact_key_material() {
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

#[test]
fn local_address_identity_uses_the_public_semantic_contract() {
    for address in [
        "03:12:34:56:78:9A",
        "00:12:34:56:78:9A",
        "02:12:34:9E:8B:00",
        "02:12:34:9E:8B:3F",
    ] {
        let mut document = valid_profile("pro");
        document["identity"] = json!({
            "kind": "exp-local-address",
            "address": address
        });

        let error = ProfileDocument::parse_json(document.to_string().as_bytes())
            .expect_err("invalid local address identity must fail profile parsing");

        assert_eq!(error.kind(), ErrorKind::InvalidProfile);
        assert!(!error.to_string().contains(address));
        assert!(!format!("{error:?}").contains(address));
    }
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
                    "AA:BB:CC:DD:EE:FF/P": {
                        "link_key": {
                            "authenticated": true,
                            "value": "01010101010101010101010101010101"
                        },
                        "link_key_type": 4
                    }
                }
            }
        }
    })
}

fn assert_invalid_profile_without_secret(value: Value) {
    let error = ProfileDocument::parse_json(value.to_string().as_bytes())
        .expect_err("invalid key-store shape must fail profile parsing");

    assert_eq!(error.kind(), ErrorKind::InvalidProfile);
    assert!(!error.to_string().contains(SECRET_SENTINEL));
    assert!(!format!("{error:?}").contains(SECRET_SENTINEL));
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
