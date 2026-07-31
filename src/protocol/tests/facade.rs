use crate::{
    input::{ImuFrame, InputState, Stick},
    model::Pro,
    profile::{ControllerColors, Rgb24},
    protocol::{
        error::ProtocolError,
        facade::{OutputPreparation, PreparedOutputAction, SwitchHidProtocol},
        imu::{ImuEncodingState, ImuMode},
        session::ProtocolSession,
        subcommand::PreparedSubcommandReply,
    },
};

const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
const CUSTOM_RUMBLE: [u8; 8] = [0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17];
const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1B, 0xDC, 0xF9, 0x9F, 0x7D];
const ACCEPTED_IMU_MODES: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05];
const ORIENTATION_REPLAY_TOLERANCE: f64 = f64::EPSILON * 8.0;

fn assert_orientation_replay_close(left: [f64; 4], right: [f64; 4]) {
    for (index, (left, right)) in left.into_iter().zip(right).enumerate() {
        let difference = (left - right).abs();
        assert!(
            difference <= ORIENTATION_REPLAY_TOLERANCE,
            "orientation component {index} differs by {difference:e}, exceeding \
             {ORIENTATION_REPLAY_TOLERANCE:e}"
        );
    }
}

#[test]
fn input_preparation_returns_disabled_imu_candidates_without_mutating_current() {
    let protocol = protocol(None);
    let current_imu = ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123));
    let current = ProtocolSession::default().with_imu_encoding_state(current_imu);

    let prepared = protocol.prepare_input_report(&InputState::<Pro>::neutral(), 0xff, current, 456);

    assert_eq!(
        &prepared.bytes()[..13],
        &[0x30, 0xff, 0x80, 0, 0, 0, 0, 8, 0x80, 0, 8, 0x80, 0]
    );
    assert_eq!(&prepared.bytes()[13..], &[0; 36]);
    assert_eq!(prepared.next_timer(), 0);
    assert_eq!(
        prepared.next_imu_encoding_state(),
        ImuEncodingState::default()
    );
    assert_eq!(current.imu_encoding_state(), current_imu);
}

#[test]
fn input_preparation_uses_state_session_and_explicit_time_for_quaternion_candidates() {
    let protocol = protocol(None);
    let frames = [
        ImuFrame::raw([1, 2, 3], [0, 0, 1000]),
        ImuFrame::raw([4, 5, 6], [0, 0, 1000]),
        ImuFrame::raw([7, 8, 9], [0, 0, 1000]),
    ];
    let state = InputState::<Pro>::neutral()
        .with_sticks(
            Stick::raw(0x123, 0xabc).unwrap(),
            Stick::raw(0xfff, 0).unwrap(),
        )
        .with_imu(frames);
    let current_imu = ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], Some(1_000_000_000));
    let current = ProtocolSession::default()
        .with_imu_mode(ImuMode::Quaternion1)
        .with_imu_encoding_state(current_imu);

    let at_two_seconds = protocol.prepare_input_report(&state, 0x2a, current, 2_000_000_000);
    let repeated_at_two_seconds =
        protocol.prepare_input_report(&state, 0x2a, current, 2_000_000_000);
    let at_three_seconds = protocol.prepare_input_report(&state, 0x2a, current, 3_000_000_000);

    assert_eq!(repeated_at_two_seconds.bytes(), at_two_seconds.bytes());
    assert_eq!(
        repeated_at_two_seconds.next_timer(),
        at_two_seconds.next_timer()
    );
    assert_eq!(
        repeated_at_two_seconds
            .next_imu_encoding_state()
            .previous_report_ns(),
        at_two_seconds
            .next_imu_encoding_state()
            .previous_report_ns()
    );
    assert_orientation_replay_close(
        repeated_at_two_seconds
            .next_imu_encoding_state()
            .orientation(),
        at_two_seconds.next_imu_encoding_state().orientation(),
    );
    assert_eq!(
        &at_two_seconds.bytes()[..13],
        &[
            0x30, 0x2a, 0x80, 0, 0, 0, 0x23, 0xc1, 0xab, 0xff, 0x0f, 0, 0,
        ]
    );
    assert_eq!(at_two_seconds.next_timer(), 0x2b);
    assert_eq!(
        at_two_seconds
            .next_imu_encoding_state()
            .previous_report_ns(),
        Some(2_000_000_000)
    );
    assert_eq!(
        at_three_seconds
            .next_imu_encoding_state()
            .previous_report_ns(),
        Some(3_000_000_000)
    );
    assert_ne!(
        &at_two_seconds.bytes()[13..],
        &at_three_seconds.bytes()[13..]
    );
    assert_ne!(
        at_two_seconds.next_imu_encoding_state().orientation(),
        at_three_seconds.next_imu_encoding_state().orientation()
    );
    assert_eq!(current.imu_encoding_state(), current_imu);
}

#[test]
fn rumble_only_report_preserves_packet_and_raw_effect_without_a_reply() {
    let protocol = protocol(None);
    let mut raw = vec![0x10, 0x2A];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.extend_from_slice(&[0xAA, 0xBB]);

    let prepared = protocol
        .prepare_output_report(
            &raw,
            &InputState::<Pro>::neutral(),
            0xFE,
            ProtocolSession::default(),
        )
        .unwrap();

    let OutputPreparation::RumbleOnly { packet_id, rumble } = prepared else {
        panic!("0x10 output report must be rumble-only");
    };
    assert_eq!(packet_id, 0x2A);
    assert_eq!(rumble.bytes(), &NEUTRAL_RUMBLE);
}

#[test]
fn stateful_route_matches_the_python_fixture_and_returns_candidates() {
    let protocol = protocol(None);
    let raw = subcommand_raw(0xAB, NEUTRAL_RUMBLE, 0x03, &[0x30]);
    let current = ProtocolSession::default();

    let first = protocol
        .prepare_output_report(&raw, &InputState::<Pro>::neutral(), 0, current)
        .unwrap();
    let second = protocol
        .prepare_output_report(&raw, &InputState::<Pro>::neutral(), 0, current)
        .unwrap();

    assert_eq!(second, first);
    let OutputPreparation::Subcommand {
        packet_id,
        rumble,
        request,
        outcome,
    } = first
    else {
        panic!("0x01 output report must carry a subcommand");
    };
    assert_eq!(packet_id, 0xAB);
    assert_eq!(rumble.bytes(), &NEUTRAL_RUMBLE);
    assert_eq!(request.id(), 0x03);
    assert_eq!(request.payload(), &[0x30]);
    let PreparedOutputAction::SessionReply(reply) = outcome.unwrap() else {
        panic!("0x03 must return a session candidate");
    };
    assert_eq!(
        reply.bytes(),
        &decode_50_byte_hex(
            "2100800000000008800008800080030000000000000000000000000000000000000000000000000000000000000000000000"
        )
    );
    assert_eq!(reply.next_timer(), 1);
    assert_eq!(reply.next_session().report_mode(), Some(0x30));
    assert_eq!(current, ProtocolSession::default());
}

#[test]
fn every_supported_subcommand_routes_to_the_expected_action_kind() {
    let protocol = protocol(None);
    let state = InputState::<Pro>::neutral();
    let current = ProtocolSession::default();
    let reply_cases: &[(u8, &[u8])] = &[
        (0x02, &[]),
        (0x04, &[]),
        (0x08, &[]),
        (0x10, &[0x12, 0x60, 0x00, 0x00, 0x01]),
        (0x21, &[0x01]),
        (0x22, &[0x01]),
    ];
    let session_cases: &[(u8, &[u8])] = &[
        (0x03, &[0x30]),
        (0x30, &[0x01]),
        (0x40, &[0x02]),
        (0x48, &[0x01]),
    ];

    for &(subcommand_id, payload) in reply_cases {
        let raw = subcommand_raw(0x12, NEUTRAL_RUMBLE, subcommand_id, payload);
        let prepared = protocol
            .prepare_output_report(&raw, &state, 0x42, current)
            .unwrap();
        let OutputPreparation::Subcommand { outcome, .. } = prepared else {
            panic!("supported subcommand must be parsed");
        };
        let PreparedOutputAction::Reply(reply) = outcome.unwrap() else {
            panic!("stateless and SPI subcommands must not return a session candidate");
        };
        assert_eq!(reply.bytes()[14], subcommand_id);
        assert_eq!(reply.next_timer(), 0x43);
    }

    for &(subcommand_id, payload) in session_cases {
        let raw = subcommand_raw(0x12, NEUTRAL_RUMBLE, subcommand_id, payload);
        let prepared = protocol
            .prepare_output_report(&raw, &state, 0x42, current)
            .unwrap();
        let OutputPreparation::Subcommand { outcome, .. } = prepared else {
            panic!("supported subcommand must be parsed");
        };
        let PreparedOutputAction::SessionReply(reply) = outcome.unwrap() else {
            panic!("stateful subcommands must return a session candidate");
        };
        assert_eq!(reply.bytes()[14], subcommand_id);
        assert_eq!(reply.next_timer(), 0x43);
    }
}

#[test]
fn facade_owned_address_and_colors_feed_the_existing_handlers() {
    let colors = ControllerColors::new(
        Rgb24::new(0x01, 0x02, 0x03),
        Rgb24::new(0x04, 0x05, 0x06),
        Rgb24::new(0x07, 0x08, 0x09),
        Rgb24::new(0x0A, 0x0B, 0x0C),
    );
    let protocol = protocol(Some(colors));
    let state = InputState::<Pro>::neutral();
    let current = ProtocolSession::default();

    let device_info_raw = subcommand_raw(0, NEUTRAL_RUMBLE, 0x02, &[]);
    let device_info = reply_from(
        protocol
            .prepare_output_report(&device_info_raw, &state, 0, current)
            .unwrap(),
    );
    assert_eq!(&device_info.bytes()[19..25], DEVICE_INFO_ADDRESS);

    let colors_raw = subcommand_raw(0, NEUTRAL_RUMBLE, 0x10, &[0x50, 0x60, 0x00, 0x00, 0x0C]);
    let color_reply = reply_from(
        protocol
            .prepare_output_report(&colors_raw, &state, 0, current)
            .unwrap(),
    );
    assert_eq!(&color_reply.bytes()[20..32], colors.to_spi_bytes());
}

#[test]
fn unsupported_subcommands_keep_parsed_context_in_the_inner_error() {
    let protocol = protocol(None);
    for subcommand_id in [0x23, 0x99] {
        let raw = subcommand_raw(0x77, CUSTOM_RUMBLE, subcommand_id, &[0xAA, 0xBB]);
        let prepared = protocol
            .prepare_output_report(
                &raw,
                &InputState::<Pro>::neutral(),
                0,
                ProtocolSession::default(),
            )
            .unwrap();

        let OutputPreparation::Subcommand {
            packet_id,
            rumble,
            request,
            outcome,
        } = prepared
        else {
            panic!("unsupported subcommand still has parsed context");
        };
        assert_eq!(packet_id, 0x77);
        assert_eq!(rumble.bytes(), &CUSTOM_RUMBLE);
        assert_eq!(request.id(), subcommand_id);
        assert_eq!(request.payload(), &[0xAA, 0xBB]);
        let error = outcome.unwrap_err();
        assert_eq!(
            error,
            ProtocolError::UnsupportedSubcommand { subcommand_id }
        );
        assert_eq!(
            error.to_string(),
            format!("unsupported subcommand: 0x{subcommand_id:02x}")
        );
    }
}

#[test]
fn semantic_errors_keep_parsed_context_and_the_current_session() {
    let protocol = protocol(None);
    let state = InputState::<Pro>::neutral();
    let current = ProtocolSession::default()
        .with_report_mode(0x30)
        .with_player_lights(0x01);
    let cases: &[(u8, &[u8], ProtocolError)] = &[
        (
            0x03,
            &[],
            ProtocolError::MissingSubcommandArgument {
                subcommand_id: 0x03,
            },
        ),
        (
            0x10,
            &[0x12, 0x60, 0x00, 0x00],
            ProtocolError::TruncatedSpiReadRequest {
                minimum: 5,
                actual: 4,
            },
        ),
        (
            0x40,
            &[0x06],
            ProtocolError::UnsupportedImuMode {
                requested: 0x06,
                accepted: ACCEPTED_IMU_MODES,
            },
        ),
    ];

    for &(subcommand_id, payload, expected) in cases {
        let raw = subcommand_raw(0x34, CUSTOM_RUMBLE, subcommand_id, payload);
        let prepared = protocol
            .prepare_output_report(&raw, &state, 9, current)
            .unwrap();
        let OutputPreparation::Subcommand {
            packet_id,
            rumble,
            request,
            outcome,
        } = prepared
        else {
            panic!("invalid subcommand still has parsed context");
        };

        assert_eq!(packet_id, 0x34);
        assert_eq!(rumble.bytes(), &CUSTOM_RUMBLE);
        assert_eq!(request.id(), subcommand_id);
        assert_eq!(request.payload(), payload);
        assert_eq!(outcome, Err(expected));
        assert!(current.protocol_ready());
    }
}

#[test]
fn parse_failures_remain_outer_errors_without_a_fabricated_effect() {
    let protocol = protocol(None);
    let cases = [
        (vec![], ProtocolError::OutputReportEmpty),
        (
            vec![0x99],
            ProtocolError::UnsupportedOutputReport { report_id: 0x99 },
        ),
        (
            vec![0x01; 10],
            ProtocolError::TruncatedOutputReport {
                report_id: 0x01,
                minimum: 11,
                actual: 10,
            },
        ),
        (
            vec![0x10; 9],
            ProtocolError::TruncatedOutputReport {
                report_id: 0x10,
                minimum: 10,
                actual: 9,
            },
        ),
    ];

    for (raw, expected) in cases {
        assert_eq!(
            protocol.prepare_output_report(
                &raw,
                &InputState::<Pro>::neutral(),
                0,
                ProtocolSession::default(),
            ),
            Err(expected)
        );
    }
}

#[test]
fn facade_uses_the_explicit_input_state_even_when_the_session_is_not_ready() {
    let protocol = protocol(None);
    let state = InputState::<Pro>::neutral().with_sticks(
        Stick::raw(0x123, 0xABC).unwrap(),
        Stick::raw(0xFFF, 0x000).unwrap(),
    );
    let raw = subcommand_raw(0, NEUTRAL_RUMBLE, 0x08, &[]);

    let reply = reply_from(
        protocol
            .prepare_output_report(&raw, &state, 0xFE, ProtocolSession::default())
            .unwrap(),
    );

    assert_eq!(
        &reply.bytes()[..13],
        &[
            0x21, 0xFE, 0x80, 0x00, 0x00, 0x00, 0x23, 0xC1, 0xAB, 0xFF, 0x0F, 0x00, 0x00,
        ]
    );
    assert_eq!(reply.next_timer(), 0xFF);
}

fn protocol(colors: Option<ControllerColors>) -> SwitchHidProtocol<Pro> {
    SwitchHidProtocol::new(colors, DEVICE_INFO_ADDRESS)
}

fn reply_from(prepared: OutputPreparation<'_>) -> PreparedSubcommandReply {
    let OutputPreparation::Subcommand { outcome, .. } = prepared else {
        panic!("expected subcommand preparation");
    };
    let PreparedOutputAction::Reply(reply) = outcome.unwrap() else {
        panic!("expected reply without a session candidate");
    };
    reply
}

fn subcommand_raw(packet_id: u8, rumble: [u8; 8], subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut raw = vec![0x01, packet_id];
    raw.extend_from_slice(&rumble);
    raw.push(subcommand_id);
    raw.extend_from_slice(payload);
    raw
}

fn decode_50_byte_hex(value: &str) -> [u8; 50] {
    let mut decoded = [0; 50];
    for (index, byte) in decoded.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    decoded
}
