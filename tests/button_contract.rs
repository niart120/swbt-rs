use swbt::{ButtonKind, DirectJoyConL, ErrorKind, JoyConLButton};

#[test]
fn controller_dynamic_button_boundary_returns_typed_buttons_or_unsupported_input() {
    let controller = DirectJoyConL::builder("usb:0")
        .build()
        .expect("build side-effect-free controller");

    assert_eq!(
        controller
            .button(ButtonKind::DpadUp)
            .expect("left Joy-Con supports D-pad up"),
        JoyConLButton::DPAD_UP
    );
    assert_eq!(
        controller
            .button(ButtonKind::A)
            .expect_err("left Joy-Con does not support A")
            .kind(),
        ErrorKind::UnsupportedInput
    );
}
