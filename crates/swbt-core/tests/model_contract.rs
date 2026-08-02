use std::collections::HashSet;

use swbt_core::ControllerKind;
use swbt_core::model::{ControllerModel, JoyConL, JoyConR, Pro};

fn assert_model<M: ControllerModel>(
    kind: ControllerKind,
    profile_name: &str,
    has_left_stick: bool,
    has_right_stick: bool,
) {
    assert_eq!(M::KIND, kind);
    assert_eq!(M::PROFILE_NAME, profile_name);
    assert_eq!(M::SPEC.kind(), kind);
    assert_eq!(M::SPEC.profile_name(), profile_name);
    assert_eq!(M::SPEC.has_left_stick(), has_left_stick);
    assert_eq!(M::SPEC.has_right_stick(), has_right_stick);
}

#[test]
fn controller_models_project_unique_runtime_identity_and_capabilities() {
    assert_model::<Pro>(ControllerKind::Pro, "pro", true, true);
    assert_model::<JoyConL>(ControllerKind::JoyConL, "joycon_l", true, false);
    assert_model::<JoyConR>(ControllerKind::JoyConR, "joycon_r", false, true);

    let kinds = ControllerKind::ALL.iter().copied().collect::<HashSet<_>>();
    let profile_names = ControllerKind::ALL
        .iter()
        .map(|kind| kind.profile_name())
        .collect::<HashSet<_>>();

    assert_eq!(kinds.len(), ControllerKind::ALL.len());
    assert_eq!(profile_names.len(), ControllerKind::ALL.len());
}
