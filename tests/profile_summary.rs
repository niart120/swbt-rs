use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use swbt::{ControllerKind, ErrorKind, ProfileIdentityKind, ProfileSummary, inspect_profile};

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

#[test]
fn dynamic_profile_summary_exposes_only_safe_validated_metadata() {
    let summary = ProfileSummary::from_json(profile_json().as_bytes()).expect("valid profile");

    assert_eq!(summary.schema_version(), 2);
    assert_eq!(summary.controller_kind(), ControllerKind::JoyConL);
    assert_eq!(summary.identity_kind(), ProfileIdentityKind::LocalAddress);
    assert_eq!(summary.namespace_count(), 1);
    assert_eq!(summary.bond_count(), 1);
    let debug = format!("{summary:?}");
    assert!(!debug.contains("02:12:34:56:78:9A"));
    assert!(!debug.contains("AA:BB:CC:DD:EE:FF"));
    assert!(!debug.contains("7045EC0E7A7045EC"));
}

#[test]
fn file_inspection_maps_read_and_validation_errors_without_disclosing_the_path() {
    let sequence = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "swbt-profile-summary-T04_SECRET_PATH-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).expect("create profile summary test directory");
    let path = directory.join("profile.json");
    fs::write(&path, profile_json()).expect("write profile summary test fixture");

    let summary = inspect_profile(&path).expect("inspect valid profile");
    assert_eq!(summary.controller_kind(), ControllerKind::JoyConL);

    fs::write(&path, b"{").expect("replace with malformed profile");
    let invalid = inspect_profile(&path).expect_err("malformed profile must fail");
    assert_eq!(invalid.kind(), ErrorKind::InvalidProfile);
    assert!(!invalid.to_string().contains("T04_SECRET_PATH"));
    assert!(!format!("{invalid:?}").contains("T04_SECRET_PATH"));

    fs::remove_file(&path).expect("remove profile summary test fixture");
    let missing = inspect_profile(&path).expect_err("missing profile must fail");
    assert_eq!(missing.kind(), ErrorKind::ProfileNotFound);
    assert!(!missing.to_string().contains("T04_SECRET_PATH"));
    assert!(!format!("{missing:?}").contains("T04_SECRET_PATH"));
    fs::remove_dir(&directory).expect("remove profile summary test directory");
}

fn profile_json() -> &'static str {
    r#"{
  "format": "swbt.profile",
  "schema_version": 2,
  "controller_kind": "joycon_l",
  "identity": {
    "kind": "exp-local-address",
    "address": "02:12:34:56:78:9A"
  },
  "key_store": {
    "namespaces": {
      "02:12:34:56:78:9A": {
        "AA:BB:CC:DD:EE:FF": {
          "link_key": {
            "value": "7045EC0E7A7045EC"
          }
        }
      }
    }
  }
}"#
}
