use swbt::{
    ButtonKind, ImuFrame, JoyConLButton, JoyConLInputState, ProButton, ProInputState, Stick,
};

fn main() -> swbt::Result<()> {
    let pro = ProInputState::neutral()
        .with_buttons([ProButton::A, ProButton::L])
        .with_sticks(Stick::left(0.5)?, Stick::right(0.5)?);
    assert_eq!(
        pro.buttons()
            .map(|button| button.kind())
            .collect::<Vec<_>>(),
        [ButtonKind::A, ButtonKind::L]
    );

    let left = JoyConLInputState::neutral()
        .with_buttons([JoyConLButton::SL, JoyConLButton::SR])
        .with_left_stick(Stick::up(1.0)?)
        .with_imu(ImuFrame::neutral());
    assert_eq!(left.imu_frames(), &[ImuFrame::neutral(); 3]);

    Ok(())
}
