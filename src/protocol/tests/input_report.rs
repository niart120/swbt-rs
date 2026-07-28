use crate::{
    model::{ControllerModel, JoyConL, JoyConR, Pro},
    protocol::input_report::encode_neutral_0x30,
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

fn assert_neutral_report<M: ControllerModel>() {
    let prepared = encode_neutral_0x30::<M>(0);

    assert_eq!(prepared.bytes(), &decode_49_byte_hex(NEUTRAL_REPORT_HEX));
    assert_eq!(prepared.next_timer(), 1);
}

fn decode_49_byte_hex(value: &str) -> [u8; 49] {
    let mut decoded = [0; 49];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    decoded
}
