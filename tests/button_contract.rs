use swbt::input::ButtonSet;
use swbt::model::{ControllerModel, JoyConL, JoyConR, Pro};
use swbt::{Button, ButtonKind, DirectJoyConL, ErrorKind, JoyConLButton, JoyConRButton, ProButton};

const PRO_BUTTONS: [ButtonKind; 18] = [
    ButtonKind::A,
    ButtonKind::B,
    ButtonKind::X,
    ButtonKind::Y,
    ButtonKind::L,
    ButtonKind::R,
    ButtonKind::ZL,
    ButtonKind::ZR,
    ButtonKind::Plus,
    ButtonKind::Minus,
    ButtonKind::Home,
    ButtonKind::Capture,
    ButtonKind::LeftStick,
    ButtonKind::RightStick,
    ButtonKind::DpadUp,
    ButtonKind::DpadDown,
    ButtonKind::DpadLeft,
    ButtonKind::DpadRight,
];

const JOYCON_L_BUTTONS: [ButtonKind; 11] = [
    ButtonKind::L,
    ButtonKind::ZL,
    ButtonKind::Minus,
    ButtonKind::Capture,
    ButtonKind::LeftStick,
    ButtonKind::SL,
    ButtonKind::SR,
    ButtonKind::DpadUp,
    ButtonKind::DpadDown,
    ButtonKind::DpadLeft,
    ButtonKind::DpadRight,
];

const JOYCON_R_BUTTONS: [ButtonKind; 11] = [
    ButtonKind::A,
    ButtonKind::B,
    ButtonKind::X,
    ButtonKind::Y,
    ButtonKind::R,
    ButtonKind::ZR,
    ButtonKind::Plus,
    ButtonKind::Home,
    ButtonKind::RightStick,
    ButtonKind::SL,
    ButtonKind::SR,
];

fn assert_dynamic_button_set<M: ControllerModel>(expected: &[ButtonKind]) {
    let accepted = ButtonKind::ALL
        .iter()
        .copied()
        .filter(|kind| Button::<M>::try_from(*kind).is_ok())
        .collect::<Vec<_>>();

    assert_eq!(accepted, expected);

    for kind in ButtonKind::ALL
        .iter()
        .copied()
        .filter(|kind| !expected.contains(kind))
    {
        let error = Button::<M>::try_from(kind).expect_err("button must be rejected");
        assert_eq!(error.kind(), ErrorKind::UnsupportedInput);
    }
}

#[test]
fn logical_button_codes_are_stable_and_model_conversion_is_exact() {
    let codes = ButtonKind::ALL
        .iter()
        .map(|kind| *kind as u8)
        .collect::<Vec<_>>();
    assert_eq!(codes, (0x00..=0x13).collect::<Vec<_>>());

    assert_dynamic_button_set::<Pro>(&PRO_BUTTONS);
    assert_dynamic_button_set::<JoyConL>(&JOYCON_L_BUTTONS);
    assert_dynamic_button_set::<JoyConR>(&JOYCON_R_BUTTONS);
}

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

#[test]
fn model_specific_constants_project_their_logical_button() {
    let pro = [
        ProButton::A,
        ProButton::B,
        ProButton::X,
        ProButton::Y,
        ProButton::L,
        ProButton::R,
        ProButton::ZL,
        ProButton::ZR,
        ProButton::PLUS,
        ProButton::MINUS,
        ProButton::HOME,
        ProButton::CAPTURE,
        ProButton::LEFT_STICK,
        ProButton::RIGHT_STICK,
        ProButton::DPAD_UP,
        ProButton::DPAD_DOWN,
        ProButton::DPAD_LEFT,
        ProButton::DPAD_RIGHT,
    ];
    let joycon_l = [
        JoyConLButton::L,
        JoyConLButton::ZL,
        JoyConLButton::MINUS,
        JoyConLButton::CAPTURE,
        JoyConLButton::LEFT_STICK,
        JoyConLButton::SL,
        JoyConLButton::SR,
        JoyConLButton::DPAD_UP,
        JoyConLButton::DPAD_DOWN,
        JoyConLButton::DPAD_LEFT,
        JoyConLButton::DPAD_RIGHT,
    ];
    let joycon_r = [
        JoyConRButton::A,
        JoyConRButton::B,
        JoyConRButton::X,
        JoyConRButton::Y,
        JoyConRButton::R,
        JoyConRButton::ZR,
        JoyConRButton::PLUS,
        JoyConRButton::HOME,
        JoyConRButton::RIGHT_STICK,
        JoyConRButton::SL,
        JoyConRButton::SR,
    ];

    assert_eq!(pro.map(|button| button.kind()), PRO_BUTTONS);
    assert_eq!(joycon_l.map(|button| button.kind()), JOYCON_L_BUTTONS);
    assert_eq!(joycon_r.map(|button| button.kind()), JOYCON_R_BUTTONS);
}

#[test]
fn button_set_removes_duplicates_and_iterates_in_logical_order() {
    let buttons = [ProButton::B, ProButton::A, ProButton::B]
        .into_iter()
        .collect::<ButtonSet<Pro>>();

    assert_eq!(buttons.len(), 2);
    assert!(buttons.contains(ProButton::A));
    assert!(buttons.contains(ProButton::B));
    assert_eq!(
        buttons
            .iter()
            .map(|button| button.kind())
            .collect::<Vec<_>>(),
        [ButtonKind::A, ButtonKind::B]
    );
}
