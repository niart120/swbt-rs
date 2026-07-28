use crate::{
    input::{Button, InputState, Stick},
    model::{ControllerModel, JoyConL, JoyConR, Pro},
    protocol::{
        error::ProtocolError,
        output_report::parse_output_report,
        subcommand::{
            DeviceInfoBluetoothAddress, PreparedSubcommandReply, prepare_0x21,
            try_prepare_stateless_reply,
        },
    },
};

const DEVICE_INFO_ADDRESS: DeviceInfoBluetoothAddress =
    DeviceInfoBluetoothAddress::from_wire_bytes([0x00, 0x1B, 0xDC, 0xF9, 0x9F, 0x7D]);
const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

#[test]
fn device_info_replies_match_all_model_fixtures_without_reversing_the_address() {
    assert_neutral_fixture::<Pro>(
        0x02,
        &[],
        "21008000000000088000088000820204000302001bdcf99f7d03020000000000000000000000000000000000000000000000",
    );
    assert_neutral_fixture::<JoyConL>(
        0x02,
        &[],
        "21008000000000088000088000820204000102001bdcf99f7d01010000000000000000000000000000000000000000000000",
    );
    assert_neutral_fixture::<JoyConR>(
        0x02,
        &[],
        "21008000000000088000088000820204000202001bdcf99f7d01010000000000000000000000000000000000000000000000",
    );
}

#[test]
fn trigger_elapsed_replies_place_pairing_buttons_in_python_slot_order() {
    assert_neutral_fixture::<Pro>(
        0x04,
        &[],
        "2100800000000008800008800083042c012c0100000000000000000000000000000000000000000000000000000000000000",
    );
    assert_neutral_fixture::<JoyConL>(
        0x04,
        &[],
        "21008000000000088000088000830400000000000000002c012c010000000000000000000000000000000000000000000000",
    );
}

#[test]
fn simple_ack_reply_matches_the_python_fixture() {
    assert_neutral_fixture::<Pro>(
        0x08,
        &[],
        "2100800000000008800008800080080000000000000000000000000000000000000000000000000000000000000000000000",
    );
}

#[test]
fn mcu_config_reply_matches_the_python_fixture_and_leaves_one_padding_byte() {
    assert_neutral_fixture::<Pro>(
        0x21,
        &[0x01],
        "21008000000000088000088000a0210100ff0008001b0100000000000000000000000000000000000000000000000000c800",
    );
}

#[test]
fn reply_prefix_uses_the_typed_current_input_and_explicit_timer() {
    let state = InputState::<Pro>::neutral()
        .with_buttons([Button::<Pro>::A])
        .with_sticks(
            Stick::raw(0x123, 0x456).unwrap(),
            Stick::raw(0x789, 0xABC).unwrap(),
        );

    let reply = stateless_reply(0x08, &[], &state, 0xFE);

    assert_eq!(
        &reply.bytes()[..15],
        &[
            0x21, 0xFE, 0x80, 0x08, 0x00, 0x00, 0x23, 0x61, 0x45, 0x89, 0xC7, 0xAB, 0x00, 0x80,
            0x08
        ]
    );
    assert_eq!(&reply.bytes()[15..], &[0; 35]);
    assert_eq!(reply.next_timer(), 0xFF);
}

#[test]
fn reply_envelope_rejects_data_that_cannot_fit_without_panicking() {
    let state = InputState::<Pro>::neutral();

    let result = prepare_0x21(0x08, 0x80, &[0; 36], &state, 0);

    assert_eq!(
        result,
        Err(ProtocolError::SubcommandReplyDataTooLarge {
            size: 36,
            maximum: 35,
        })
    );
}

#[test]
fn stateless_handler_defers_other_subcommands_to_later_handlers() {
    let state = InputState::<Pro>::neutral();

    for subcommand_id in [0x03, 0x10, 0x30, 0x40, 0x48, 0x99] {
        let mut raw = vec![0x01, 0x0A];
        raw.extend_from_slice(&NEUTRAL_RUMBLE);
        raw.push(subcommand_id);
        let request = parse_output_report(&raw)
            .unwrap()
            .subcommand()
            .expect("0x01 output report has a subcommand");

        assert!(
            try_prepare_stateless_reply(request, &state, 0, DEVICE_INFO_ADDRESS)
                .unwrap()
                .is_none()
        );
    }
}

fn assert_neutral_fixture<M: ControllerModel>(
    subcommand_id: u8,
    payload: &[u8],
    expected_hex: &str,
) {
    let state = InputState::<M>::neutral();
    let reply = stateless_reply(subcommand_id, payload, &state, 0);

    assert_eq!(reply.bytes(), &decode_50_byte_hex(expected_hex));
    assert_eq!(reply.next_timer(), 1);
}

fn stateless_reply<M: ControllerModel>(
    subcommand_id: u8,
    payload: &[u8],
    state: &InputState<M>,
    timer: u8,
) -> PreparedSubcommandReply {
    let mut raw = vec![0x01, 0x0A];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.push(subcommand_id);
    raw.extend_from_slice(payload);
    let request = parse_output_report(&raw)
        .unwrap()
        .subcommand()
        .expect("0x01 output report has a subcommand");

    try_prepare_stateless_reply(request, state, timer, DEVICE_INFO_ADDRESS)
        .unwrap()
        .expect("fixture subcommand is stateless")
}

fn decode_50_byte_hex(value: &str) -> [u8; 50] {
    let mut decoded = [0; 50];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    decoded
}
