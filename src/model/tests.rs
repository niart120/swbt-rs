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

#[test]
fn protocol_metadata_is_projected_from_each_model_declaration() {
    let cases = [
        (
            ControllerKind::Pro,
            "Pro Controller",
            0x03,
            [0x03, 0x02],
            [ButtonKind::L, ButtonKind::R].as_slice(),
            "323232ffffff00b2ffff3b30",
        ),
        (
            ControllerKind::JoyConL,
            "Joy-Con (L)",
            0x01,
            [0x01, 0x01],
            [ButtonKind::SL, ButtonKind::SR].as_slice(),
            "00b2ff32323200b2ff00b2ff",
        ),
        (
            ControllerKind::JoyConR,
            "Joy-Con (R)",
            0x02,
            [0x01, 0x01],
            [ButtonKind::SL, ButtonKind::SR].as_slice(),
            "ff3b30323232ff3b30ff3b30",
        ),
    ];

    for (kind, local_name, device_type, tail, pairing, colors_hex) in cases {
        let protocol = kind.spec().protocol;
        assert_eq!(protocol.local_name, local_name);
        assert_eq!(protocol.class_of_device, 0x002508);
        assert_eq!(protocol.device_type, device_type);
        assert_eq!(protocol.device_info_tail, tail);
        assert_eq!(protocol.battery_connection, 0x80);
        assert_eq!(protocol.vibrator_input, 0x00);
        assert_eq!(protocol.pairing_trigger_buttons, pairing);
        assert_eq!(protocol.accepted_imu_modes, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(
            protocol.default_colors.to_spi_bytes(),
            decode_12_byte_hex(colors_hex)
        );
        assert_eq!(protocol.accelerometer_calibration.zero_raw, [0; 3]);
        assert_eq!(
            protocol.accelerometer_calibration.reference_raw,
            [0x4000; 3]
        );
        assert_eq!(protocol.gyroscope_calibration.zero_raw, [0; 3]);
        assert_eq!(protocol.gyroscope_calibration.reference_raw, [0x343B; 3]);
    }
}

fn decode_12_byte_hex(value: &str) -> [u8; 12] {
    let mut decoded = [0; 12];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    decoded
}
