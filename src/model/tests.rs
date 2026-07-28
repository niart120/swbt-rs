use super::{ButtonKind, ButtonWirePosition, ControllerKind, button_wire_position};

const PRO_MAPPING: &[(ButtonKind, ButtonWirePosition)] = &[
    (ButtonKind::A, ButtonWirePosition::new(3, 0x08)),
    (ButtonKind::B, ButtonWirePosition::new(3, 0x04)),
    (ButtonKind::X, ButtonWirePosition::new(3, 0x02)),
    (ButtonKind::Y, ButtonWirePosition::new(3, 0x01)),
    (ButtonKind::L, ButtonWirePosition::new(5, 0x40)),
    (ButtonKind::R, ButtonWirePosition::new(3, 0x40)),
    (ButtonKind::ZL, ButtonWirePosition::new(5, 0x80)),
    (ButtonKind::ZR, ButtonWirePosition::new(3, 0x80)),
    (ButtonKind::Plus, ButtonWirePosition::new(4, 0x02)),
    (ButtonKind::Minus, ButtonWirePosition::new(4, 0x01)),
    (ButtonKind::Home, ButtonWirePosition::new(4, 0x10)),
    (ButtonKind::Capture, ButtonWirePosition::new(4, 0x20)),
    (ButtonKind::LeftStick, ButtonWirePosition::new(4, 0x08)),
    (ButtonKind::RightStick, ButtonWirePosition::new(4, 0x04)),
    (ButtonKind::DpadUp, ButtonWirePosition::new(5, 0x02)),
    (ButtonKind::DpadDown, ButtonWirePosition::new(5, 0x01)),
    (ButtonKind::DpadLeft, ButtonWirePosition::new(5, 0x08)),
    (ButtonKind::DpadRight, ButtonWirePosition::new(5, 0x04)),
];

const JOYCON_L_MAPPING: &[(ButtonKind, ButtonWirePosition)] = &[
    (ButtonKind::L, ButtonWirePosition::new(5, 0x40)),
    (ButtonKind::ZL, ButtonWirePosition::new(5, 0x80)),
    (ButtonKind::Minus, ButtonWirePosition::new(4, 0x01)),
    (ButtonKind::Capture, ButtonWirePosition::new(4, 0x20)),
    (ButtonKind::LeftStick, ButtonWirePosition::new(4, 0x08)),
    (ButtonKind::SL, ButtonWirePosition::new(5, 0x20)),
    (ButtonKind::SR, ButtonWirePosition::new(5, 0x10)),
    (ButtonKind::DpadUp, ButtonWirePosition::new(5, 0x02)),
    (ButtonKind::DpadDown, ButtonWirePosition::new(5, 0x01)),
    (ButtonKind::DpadLeft, ButtonWirePosition::new(5, 0x08)),
    (ButtonKind::DpadRight, ButtonWirePosition::new(5, 0x04)),
];

const JOYCON_R_MAPPING: &[(ButtonKind, ButtonWirePosition)] = &[
    (ButtonKind::A, ButtonWirePosition::new(3, 0x08)),
    (ButtonKind::B, ButtonWirePosition::new(3, 0x04)),
    (ButtonKind::X, ButtonWirePosition::new(3, 0x02)),
    (ButtonKind::Y, ButtonWirePosition::new(3, 0x01)),
    (ButtonKind::R, ButtonWirePosition::new(3, 0x40)),
    (ButtonKind::ZR, ButtonWirePosition::new(3, 0x80)),
    (ButtonKind::Plus, ButtonWirePosition::new(4, 0x02)),
    (ButtonKind::Home, ButtonWirePosition::new(4, 0x10)),
    (ButtonKind::RightStick, ButtonWirePosition::new(4, 0x04)),
    (ButtonKind::SL, ButtonWirePosition::new(3, 0x20)),
    (ButtonKind::SR, ButtonWirePosition::new(3, 0x10)),
];

fn assert_mapping(controller: ControllerKind, expected: &[(ButtonKind, ButtonWirePosition)]) {
    for kind in ButtonKind::ALL {
        let expected_position = expected
            .iter()
            .find_map(|(expected_kind, position)| (*expected_kind == *kind).then_some(*position));
        assert_eq!(
            button_wire_position(controller, *kind),
            expected_position,
            "{controller:?} {kind:?}"
        );
    }
}

#[test]
fn all_supported_buttons_use_the_python_baseline_wire_mapping() {
    assert_mapping(ControllerKind::Pro, PRO_MAPPING);
    assert_mapping(ControllerKind::JoyConL, JOYCON_L_MAPPING);
    assert_mapping(ControllerKind::JoyConR, JOYCON_R_MAPPING);
}

#[test]
fn wire_positions_use_one_bit_inside_the_button_bytes() {
    for controller in ControllerKind::ALL {
        for button in ButtonKind::ALL {
            if let Some(position) = button_wire_position(*controller, *button) {
                assert!((3..=5).contains(&position.byte_index));
                assert_eq!(position.mask.count_ones(), 1);
            }
        }
    }
}
