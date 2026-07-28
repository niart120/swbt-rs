use crate::{
    input::{Button, InputState, Stick},
    model::{ControllerModel, JoyConL, JoyConR, Pro},
    protocol::{
        error::ProtocolError,
        output_report::parse_output_report,
        spi::{MAX_READ_SIZE, VirtualSpiFlash},
        subcommand::{
            DeviceInfoBluetoothAddress, PreparedSubcommandReply, prepare_0x21,
            try_prepare_spi_reply, try_prepare_stateless_reply,
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

#[test]
fn spi_reply_matches_the_python_device_type_fixture() {
    let spi = VirtualSpiFlash::<Pro>::new(None);
    let state = InputState::<Pro>::neutral();

    let reply = spi_reply(&[0x12, 0x60, 0x00, 0x00, 0x01], &spi, &state, 0).unwrap();

    assert_eq!(
        reply.bytes(),
        &decode_50_byte_hex(
            "2100800000000008800008800090101260000001030000000000000000000000000000000000000000000000000000000000"
        )
    );
    assert_eq!(reply.next_timer(), 1);
}

#[test]
fn spi_reply_rejects_every_payload_shorter_than_the_request_prefix() {
    let spi = VirtualSpiFlash::<Pro>::new(None);
    let state = InputState::<Pro>::neutral();
    let payload = [0x12, 0x60, 0x00, 0x00];

    for actual in 0..=payload.len() {
        assert_eq!(
            spi_reply(&payload[..actual], &spi, &state, 0),
            Err(ProtocolError::TruncatedSpiReadRequest { minimum: 5, actual })
        );
    }
}

#[test]
fn spi_reply_uses_only_the_first_five_request_bytes() {
    let spi = VirtualSpiFlash::<Pro>::new(None);
    let state = InputState::<Pro>::neutral();

    let exact = spi_reply(&[0x50, 0x60, 0x00, 0x00, 0x0C], &spi, &state, 7).unwrap();
    let trailing = spi_reply(&[0x50, 0x60, 0x00, 0x00, 0x0C, 0xAA, 0xBB], &spi, &state, 7).unwrap();

    assert_eq!(trailing, exact);
    assert_eq!(&exact.bytes()[15..20], &[0x50, 0x60, 0x00, 0x00, 0x0C]);
    assert_eq!(
        &exact.bytes()[20..32],
        &[
            0x32, 0x32, 0x32, 0xFF, 0xFF, 0xFF, 0x00, 0xB2, 0xFF, 0xFF, 0x3B, 0x30
        ]
    );
}

#[test]
fn spi_reply_keeps_maximum_read_inside_the_envelope() {
    let spi = VirtualSpiFlash::<Pro>::new(None);
    let state = InputState::<Pro>::neutral();

    let maximum = spi_reply(
        &[0x12, 0x60, 0x00, 0x00, MAX_READ_SIZE as u8],
        &spi,
        &state,
        0xFF,
    )
    .unwrap();

    assert_eq!(
        &maximum.bytes()[15..49],
        &decode_34_byte_hex("126000001d03ffffffffffffffff01ffffffff000000000000004000400040000000")
    );
    assert_eq!(maximum.bytes()[49], 0);
    assert_eq!(maximum.next_timer(), 0);
}

#[test]
fn spi_reply_propagates_address_errors() {
    let spi = VirtualSpiFlash::<Pro>::new(None);
    let state = InputState::<Pro>::neutral();

    assert_eq!(
        spi_reply(&[0xFF, 0xFF, 0x07, 0x00, 0x02], &spi, &state, 0),
        Err(ProtocolError::SpiAddressOutOfRange {
            address: 0x7FFFF,
            size: 2,
        })
    );
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

fn spi_reply<M: ControllerModel>(
    payload: &[u8],
    spi: &VirtualSpiFlash<M>,
    state: &InputState<M>,
    timer: u8,
) -> Result<PreparedSubcommandReply, ProtocolError> {
    let mut raw = vec![0x01, 0x0A];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.push(0x10);
    raw.extend_from_slice(payload);
    let request = parse_output_report(&raw)
        .unwrap()
        .subcommand()
        .expect("0x01 output report has a subcommand");

    Ok(try_prepare_spi_reply(request, state, timer, spi)?
        .expect("0x10 subcommand is handled as SPI read"))
}

fn decode_50_byte_hex(value: &str) -> [u8; 50] {
    let mut decoded = [0; 50];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    decoded
}

fn decode_34_byte_hex(value: &str) -> [u8; 34] {
    let mut decoded = [0; 34];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    decoded
}
