use crate::{
    input::{ImuFrame, InputState},
    model::{Pro, SensorCalibration},
    protocol::{
        imu::{ImuEncodingState, ImuMode, encode_imu_block},
        input_report::encode_0x30,
    },
};

#[test]
fn disabled_imu_is_zero_and_resets_only_the_candidate_state() {
    let current = ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123));
    let frames = [ImuFrame::raw([1, 2, 3], [4, 5, 6]); 3];

    let encoded = encode_imu_block(
        current,
        ImuMode::Disabled,
        &frames,
        gyro_calibration([0; 3]),
        456,
    );

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

    let encoded = encode_imu_block(
        current,
        ImuMode::Standard,
        &frames,
        gyro_calibration([0; 3]),
        456,
    );

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

#[test]
fn every_quaternion_mode_matches_the_python_mode_2_fixture() {
    let frames = [
        ImuFrame::raw([1, 2, 3], [0, 0, 1000]),
        ImuFrame::raw([4, 5, 6], [0, 0, 1000]),
        ImuFrame::raw([7, 8, 9], [0, 0, 1000]),
    ];
    let current = ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], Some(0));
    let expected = decode_36_byte_hex(
        "0100020003000e0000000080040005000600574b0200000007000800090000000000f40d",
    );

    for mode in [
        ImuMode::Quaternion1,
        ImuMode::Quaternion2,
        ImuMode::Quaternion3,
        ImuMode::Quaternion4,
    ] {
        let encoded = encode_imu_block(
            current,
            mode,
            &frames,
            gyro_calibration([0; 3]),
            1_000_000_000,
        );

        assert_eq!(encoded.block(), &expected);
        assert!(encoded.next_state().orientation()[2] > 0.0);
        assert_eq!(
            encoded.next_state().previous_report_ns(),
            Some(1_000_000_000)
        );
        assert_eq!(
            current,
            ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], Some(0))
        );
    }
}

#[test]
fn quaternion_wire_value_distinguishes_rotation_sign_and_uses_active_calibration() {
    let positive_frames = [
        ImuFrame::raw([1, 2, 3], [0, 0, 1000]),
        ImuFrame::raw([4, 5, 6], [0, 0, 1000]),
        ImuFrame::raw([7, 8, 9], [0, 0, 1000]),
    ];
    let negative_frames = positive_frames.map(|frame| frame.with_gyro([0, 0, -1000]));
    let current = ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], Some(0));

    let positive = encode_imu_block(
        current,
        ImuMode::Quaternion1,
        &positive_frames,
        gyro_calibration([0; 3]),
        1_000_000_000,
    );
    let negative = encode_imu_block(
        current,
        ImuMode::Quaternion1,
        &negative_frames,
        gyro_calibration([0; 3]),
        1_000_000_000,
    );
    let calibrated_to_zero = encode_imu_block(
        current,
        ImuMode::Quaternion1,
        &positive_frames,
        gyro_calibration([0, 0, 1000]),
        1_000_000_000,
    );

    assert_eq!(
        positive.block(),
        &decode_36_byte_hex(
            "0100020003000e0000000080040005000600574b0200000007000800090000000000f40d"
        )
    );
    assert_eq!(
        negative.block(),
        &decode_36_byte_hex(
            "0100020003000e0000000040040005000600a8b40500000007000800090000000000f40d"
        )
    );
    assert!(mode_2_component_2(positive.block()) > 0);
    assert!(mode_2_component_2(negative.block()) < 0);
    assert_eq!(
        calibrated_to_zero.block(),
        &decode_36_byte_hex(
            "0100020003000e000000000004000500060000000000000007000800090000000000f40d"
        )
    );
}

#[test]
fn quaternion_integration_uses_each_of_the_three_samples() {
    let expected = decode_36_byte_hex(
        "0100020003000e000000000002000300040012cf0000000003000400050000000000f40d",
    );

    for active_sample in 0..3 {
        let frames = std::array::from_fn(|index| {
            ImuFrame::raw(
                [index as i16 + 1, index as i16 + 2, index as i16 + 3],
                [0, 0, if index == active_sample { 1000 } else { 0 }],
            )
        });
        let encoded = encode_imu_block(
            ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], Some(0)),
            ImuMode::Quaternion1,
            &frames,
            gyro_calibration([0; 3]),
            1_000_000_000,
        );

        assert_eq!(encoded.block(), &expected);
        assert!(encoded.next_state().orientation()[2] > 0.0);
    }
}

#[test]
fn quaternion_integration_preserves_non_commuting_sample_order() {
    let frames = [
        ImuFrame::raw([1, 2, 3], [1000, 0, 0]),
        ImuFrame::raw([4, 5, 6], [0, 1000, 0]),
        ImuFrame::raw([7, 8, 9], [0, 0, 1000]),
    ];
    let encoded = encode_imu_block(
        ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], Some(0)),
        ImuMode::Quaternion1,
        &frames,
        gyro_calibration([0; 3]),
        1_000_000_000,
    );

    assert_eq!(
        encoded.block(),
        &decode_36_byte_hex(
            "0100020003002ee73bc2ec840400050006009cef0000000007000800090000000000f40d"
        )
    );
}

#[test]
fn quaternion_initial_receding_and_reset_epochs_use_zero_elapsed_time() {
    let frames = [ImuFrame::raw([1, 2, 3], [0, 0, 1000]); 3];
    let expected_receded = decode_36_byte_hex(
        "0100020003000e000000000001000200030000000000000001000200030000000000000c",
    );

    for current in [
        ImuEncodingState::default(),
        ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], Some(200)),
    ] {
        let encoded = encode_imu_block(
            current,
            ImuMode::Quaternion1,
            &frames,
            gyro_calibration([0; 3]),
            100,
        );

        assert_eq!(encoded.block(), &expected_receded);
        assert_eq!(encoded.next_state().orientation(), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(encoded.next_state().previous_report_ns(), Some(100));
    }

    let distinct_frames = [
        ImuFrame::raw([1, 2, 3], [0, 0, 1000]),
        ImuFrame::raw([4, 5, 6], [0, 0, 1000]),
        ImuFrame::raw([7, 8, 9], [0, 0, 1000]),
    ];
    let reset = ImuEncodingState::default();
    let encoded = encode_imu_block(
        reset,
        ImuMode::Quaternion1,
        &distinct_frames,
        gyro_calibration([0; 3]),
        1_000_000_000,
    );
    assert_eq!(
        encoded.block(),
        &decode_36_byte_hex(
            "0100020003000e000000000004000500060000000000000007000800090000000000f40d"
        )
    );
    assert_eq!(reset, ImuEncodingState::default());
}

#[test]
fn quaternion_packing_uses_first_maximum_canonical_sign_and_truncated_timestamp() {
    let now_ns = 2_053_999_999;
    let frames = [ImuFrame::neutral(); 3];
    let tied = ImuEncodingState::new([0.5, -0.5, 0.5, -0.5], Some(now_ns));
    let tied_result = encode_imu_block(
        tied,
        ImuMode::Quaternion1,
        &frames,
        gyro_calibration([0; 3]),
        now_ns,
    );
    assert_eq!(
        tied_result.block(),
        &decode_36_byte_hex(
            "00000000000002008001001000000000000000000600000000000000000000000080020c"
        )
    );

    let positive = ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(0));
    let negative = ImuEncodingState::new([-0.1, -0.2, -0.3, -0.9], Some(0));
    let positive_result = encode_imu_block(
        positive,
        ImuMode::Quaternion1,
        &frames,
        gyro_calibration([0; 3]),
        0,
    );
    let negative_result = encode_imu_block(
        negative,
        ImuMode::Quaternion1,
        &frames,
        gyro_calibration([0; 3]),
        0,
    );
    assert_eq!(positive_result.block(), negative_result.block());
}

fn gyro_calibration(zero_raw: [i16; 3]) -> SensorCalibration {
    SensorCalibration {
        zero_raw,
        reference_raw: [0x343B; 3],
    }
}

fn mode_2_component_2(block: &[u8; 36]) -> i32 {
    let high = u32::from(block[18]) | (u32::from(block[19]) << 8) | (u32::from(block[20]) << 16);
    let encoded = ((high & 0x7FFFF) << 2) | u32::from(block[11] >> 6);
    if encoded & (1 << 20) == 0 {
        encoded as i32
    } else {
        encoded as i32 - (1 << 21)
    }
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
