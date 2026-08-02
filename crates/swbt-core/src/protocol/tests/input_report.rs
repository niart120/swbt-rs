use crate::{
    input::{Button, InputState, Stick},
    model::{ControllerModel, JoyConL, JoyConR, Pro},
    protocol::input_report::{encode_0x30, encode_neutral_0x30},
};

const NEUTRAL_REPORT_HEX: &str = "30008000000000088000088000000000000000000000000000000000000000000000000000000000000000000000000000";

#[test]
fn neutral_0x30_is_the_same_deterministic_49_bytes_for_every_model() {
    assert_neutral_report::<Pro>();
    assert_neutral_report::<JoyConL>();
    assert_neutral_report::<JoyConR>();
}

#[test]
fn prepared_0x30_returns_the_candidate_wrapping_timer_without_shared_mutation() {
    let prepared = encode_neutral_0x30::<Pro>(u8::MAX);
    let mut expected = decode_49_byte_hex(NEUTRAL_REPORT_HEX);
    expected[1] = u8::MAX;

    assert_eq!(prepared.bytes(), &expected);
    assert_eq!(prepared.next_timer(), 0);
    assert_eq!(encode_neutral_0x30::<Pro>(0).bytes()[1], 0);
}

#[test]
fn all_model_valid_buttons_match_the_python_wire_fixtures() {
    assert_all_button_report::<Pro>(
        "304280cf3fcf00088000088000000000000000000000000000000000000000000000000000000000000000000000000000",
    );
    assert_all_button_report::<JoyConL>(
        "3042800029ff00088000088000000000000000000000000000000000000000000000000000000000000000000000000000",
    );
    assert_all_button_report::<JoyConR>(
        "304280ff160000088000088000000000000000000000000000000000000000000000000000000000000000000000000000",
    );
}

#[test]
fn stick_packing_preserves_joycon_unavailable_side_as_neutral() {
    let pro = InputState::<Pro>::neutral().with_sticks(
        Stick::raw(0x123, 0xABC).unwrap(),
        Stick::raw(0xFFF, 0x000).unwrap(),
    );
    assert_eq!(
        encode_0x30(&pro, 0, &[0; 36]).bytes(),
        &decode_49_byte_hex(
            "30008000000023c1abff0f0000000000000000000000000000000000000000000000000000000000000000000000000000"
        )
    );

    let left = InputState::<JoyConL>::neutral()
        .with_left_stick(Stick::raw(Stick::CENTER, Stick::MAX).unwrap());
    let left_report = encode_0x30(&left, 0, &[0; 36]);
    assert_eq!(&left_report.bytes()[6..9], [0x00, 0xF8, 0xFF]);
    assert_eq!(&left_report.bytes()[9..12], [0x00, 0x08, 0x80]);

    let right = InputState::<JoyConR>::neutral()
        .with_right_stick(Stick::raw(Stick::MAX, Stick::CENTER).unwrap());
    let right_report = encode_0x30(&right, 0, &[0; 36]);
    assert_eq!(&right_report.bytes()[6..9], [0x00, 0x08, 0x80]);
    assert_eq!(&right_report.bytes()[9..12], [0xFF, 0x0F, 0x80]);
}

fn assert_neutral_report<M: ControllerModel>() {
    let prepared = encode_neutral_0x30::<M>(0);

    assert_eq!(prepared.bytes(), &decode_49_byte_hex(NEUTRAL_REPORT_HEX));
    assert_eq!(prepared.next_timer(), 1);
}

fn assert_all_button_report<M: ControllerModel>(expected_hex: &str) {
    let buttons = M::SPEC
        .supported_buttons()
        .iter()
        .copied()
        .map(|kind| Button::<M>::try_from(kind).unwrap());
    let state = InputState::<M>::neutral().with_buttons(buttons);

    assert_eq!(
        encode_0x30(&state, 0x42, &[0; 36]).bytes(),
        &decode_49_byte_hex(expected_hex)
    );
}

fn decode_49_byte_hex(value: &str) -> [u8; 49] {
    let mut decoded = [0; 49];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    decoded
}
