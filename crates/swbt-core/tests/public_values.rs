use swbt_core::{
    ButtonKind, ErrorKind, PairingProfile, Pro, ProButton, ProInputState, ProfileIdentity, Stick,
};

#[test]
fn core_exposes_model_valid_input_without_a_runtime_backend() {
    let state = ProInputState::neutral()
        .with_buttons([ProButton::A])
        .with_left_stick(Stick::up(0.5).expect("valid normalized stick"));

    assert_eq!(
        state
            .buttons()
            .map(|button| button.kind())
            .collect::<Vec<_>>(),
        [ButtonKind::A]
    );
}

#[test]
fn core_profile_values_round_trip_without_a_runtime_backend() {
    let profile = PairingProfile::<Pro>::from_json(
        br#"{
          "format": "swbt.profile",
          "schema_version": 2,
          "controller_kind": "pro",
          "identity": { "kind": "adapter-default" },
          "key_store": { "namespaces": {} }
        }"#,
    )
    .expect("valid backend-independent profile");

    let round_trip = profile.to_json_bytes().expect("serialize profile");
    let reparsed = PairingProfile::<Pro>::from_json(&round_trip).expect("reparse profile");
    assert_eq!(reparsed.controller_kind(), swbt_core::ControllerKind::Pro);
    assert_eq!(
        ProfileIdentity::AdapterDefault,
        ProfileIdentity::AdapterDefault
    );
    assert_eq!(ErrorKind::InvalidProfile, ErrorKind::InvalidProfile);
}
