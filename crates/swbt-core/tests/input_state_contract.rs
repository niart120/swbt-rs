use swbt_core::{
    ButtonKind, ImuFrame, JoyConLButton, JoyConLInputState, JoyConRButton, JoyConRInputState,
    ProButton, ProInputState, Stick,
};

#[test]
fn pro_state_replaces_complete_model_valid_input() {
    let left = Stick::raw(0, Stick::MAX).expect("left stick is valid");
    let right = Stick::raw(Stick::MAX, 0).expect("right stick is valid");
    let frames = [
        ImuFrame::raw([1, 0, 0], [0; 3]),
        ImuFrame::raw([2, 0, 0], [0; 3]),
        ImuFrame::raw([3, 0, 0], [0; 3]),
    ];

    let state = ProInputState::neutral()
        .with_buttons([ProButton::B, ProButton::A, ProButton::B])
        .with_sticks(left, right)
        .with_imu(frames);

    assert_eq!(
        state
            .buttons()
            .map(|button| button.kind())
            .collect::<Vec<_>>(),
        [ButtonKind::A, ButtonKind::B]
    );
    assert_eq!(state.left_stick(), left);
    assert_eq!(state.right_stick(), right);
    assert_eq!(state.imu_frames(), &frames);
    assert_eq!(state.clone(), state);
}

#[test]
fn neutral_state_has_centered_available_sticks_and_neutral_imu() {
    let pro = ProInputState::neutral();
    let left = JoyConLInputState::neutral();
    let right = JoyConRInputState::neutral();

    assert_eq!(pro.buttons().count(), 0);
    assert_eq!(pro.left_stick(), Stick::center());
    assert_eq!(pro.right_stick(), Stick::center());
    assert_eq!(pro.imu_frames(), &[ImuFrame::neutral(); 3]);
    assert_eq!(left.left_stick(), Stick::center());
    assert_eq!(right.right_stick(), Stick::center());
}

#[test]
fn joycon_states_expose_model_specific_buttons_and_stick_replacement() {
    let left_stick = Stick::left(1.0).expect("left tilt is valid");
    let left = JoyConLInputState::neutral()
        .with_buttons([JoyConLButton::DPAD_LEFT, JoyConLButton::SL])
        .with_left_stick(left_stick)
        .with_imu(ImuFrame::raw([1, 2, 3], [4, 5, 6]));

    let right_stick = Stick::right(1.0).expect("right tilt is valid");
    let right = JoyConRInputState::neutral()
        .with_buttons([JoyConRButton::A, JoyConRButton::SR])
        .with_right_stick(right_stick);

    assert_eq!(left.left_stick(), left_stick);
    assert_eq!(
        left.buttons()
            .map(|button| button.kind())
            .collect::<Vec<_>>(),
        [ButtonKind::SL, ButtonKind::DpadLeft]
    );
    assert_eq!(left.imu_frames(), &[ImuFrame::raw([1, 2, 3], [4, 5, 6]); 3]);
    assert_eq!(right.right_stick(), right_stick);
    assert_eq!(
        right
            .buttons()
            .map(|button| button.kind())
            .collect::<Vec<_>>(),
        [ButtonKind::A, ButtonKind::SR]
    );
}
