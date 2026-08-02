use swbt_core::{PairingProfile as CorePairingProfile, Pro as CorePro};

#[test]
fn runtime_crate_reexports_core_values_as_the_same_types() {
    let state = swbt_core::ProInputState::neutral().with_buttons([swbt_core::ProButton::A]);
    let runtime_state: swbt::ProInputState = state;
    let core_state: swbt_core::ProInputState = runtime_state;
    assert_eq!(core_state.buttons().count(), 1);

    let profile = CorePairingProfile::<CorePro>::from_json(
        br#"{
          "format": "swbt.profile",
          "schema_version": 2,
          "controller_kind": "pro",
          "identity": { "kind": "adapter-default" },
          "key_store": { "namespaces": {} }
        }"#,
    )
    .expect("valid core profile");
    let runtime_profile: swbt::PairingProfile<swbt::model::Pro> = profile;
    assert_eq!(runtime_profile.controller_kind(), swbt::ControllerKind::Pro);
}

#[test]
fn runtime_module_paths_reexport_the_core_definitions() {
    let button: swbt::input::Button<swbt::model::Pro> = swbt_core::ProButton::A;
    let kind: swbt::error::ErrorKind = swbt_core::ErrorKind::InvalidInput;

    assert_eq!(button.kind(), swbt::ButtonKind::A);
    assert_eq!(kind, swbt::ErrorKind::InvalidInput);
}
