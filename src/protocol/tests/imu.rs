use crate::{
    input::{ImuFrame, InputState},
    model::Pro,
    protocol::{
        imu::{ImuEncodingState, ImuMode, encode_imu_block},
        input_report::encode_0x30,
    },
};

#[test]
fn disabled_imu_is_zero_and_resets_only_the_candidate_state() {
    let current = ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123));
    let frames = [ImuFrame::raw([1, 2, 3], [4, 5, 6]); 3];

    let encoded = encode_imu_block(current, ImuMode::Disabled, &frames, 456);

    assert_eq!(encoded.block(), &[0; 36]);
    assert_eq!(encoded.next_state(), ImuEncodingState::default());
    assert_eq!(
        current,
        ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123))
    );
}

#[test]
fn standard_imu_preserves_three_signed_little_endian_frames_and_resets_candidate() {
    let frames = [
        ImuFrame::raw([1, -2, 3], [-4, 5, -6]),
        ImuFrame::raw([7, -8, 9], [-10, 11, -12]),
        ImuFrame::raw([13, -14, 15], [-16, 17, -18]),
    ];
    let current = ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123));

    let encoded = encode_imu_block(current, ImuMode::Standard, &frames, 456);

    assert_eq!(
        encoded.block(),
        &decode_36_byte_hex(
            "0100feff0300fcff0500faff0700f8ff0900f6ff0b00f4ff0d00f2ff0f00f0ff1100eeff"
        )
    );
    assert_eq!(encoded.next_state(), ImuEncodingState::default());
    assert_eq!(
        current,
        ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123))
    );

    let state = InputState::<Pro>::neutral().with_imu(frames);
    assert_eq!(
        encode_0x30(&state, 0, encoded.block()).bytes(),
        &decode_49_byte_hex(
            "300080000000000880000880000100feff0300fcff0500faff0700f8ff0900f6ff0b00f4ff0d00f2ff0f00f0ff1100eeff"
        )
    );
}

fn decode_36_byte_hex(value: &str) -> [u8; 36] {
    let mut decoded = [0; 36];
    decode_hex_into(value, &mut decoded);
    decoded
}

fn decode_49_byte_hex(value: &str) -> [u8; 49] {
    let mut decoded = [0; 49];
    decode_hex_into(value, &mut decoded);
    decoded
}

fn decode_hex_into(value: &str, decoded: &mut [u8]) {
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
}
