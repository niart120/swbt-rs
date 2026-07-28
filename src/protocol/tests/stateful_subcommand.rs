use crate::{
    input::{InputState, Stick},
    model::{ControllerModel, JoyConL, JoyConR, Pro},
    protocol::{
        error::ProtocolError,
        imu::{ImuEncodingState, ImuMode},
        output_report::parse_output_report,
        session::ProtocolSession,
        subcommand::{PreparedSessionReply, try_prepare_stateful_reply},
    },
};

const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
const ACCEPTED_IMU_MODES: &[u8] = &[0x00, 0x01, 0x02, 0x03, 0x04, 0x05];

#[test]
fn report_mode_candidates_match_supported_and_unsupported_fixtures() {
    let current = ProtocolSession::default();

    let supported = stateful_reply::<Pro>(0x03, &[0x30], current, 0).unwrap();
    let unsupported = stateful_reply::<Pro>(0x03, &[0x3F], current, 0).unwrap();

    assert_eq!(supported.bytes(), &expected_empty_data_reply(0x03));
    assert_eq!(supported.next_timer(), 1);
    assert_eq!(supported.next_session().report_mode(), Some(0x30));
    assert!(supported.next_session().report_mode_supported());
    assert_eq!(supported.next_session().unsupported_report_mode(), None);
    assert!(!supported.next_session().protocol_ready());
    assert_eq!(unsupported.bytes(), &expected_empty_data_reply(0x03));
    assert_eq!(unsupported.next_session().report_mode(), Some(0x3F));
    assert!(!unsupported.next_session().report_mode_supported());
    assert_eq!(
        unsupported.next_session().unsupported_report_mode(),
        Some(0x3F)
    );
    assert!(!unsupported.next_session().protocol_ready());
    assert_eq!(current, ProtocolSession::default());
}

#[test]
fn player_lights_candidate_matches_fixture_and_completes_readiness() {
    let fixture = stateful_reply::<Pro>(0x30, &[0x01], ProtocolSession::default(), 0).unwrap();
    let report_mode = ProtocolSession::default().with_report_mode(0x30);

    let ready = stateful_reply::<Pro>(0x30, &[0x01], report_mode, 0).unwrap();
    let zero_lights = stateful_reply::<Pro>(0x30, &[0x00], report_mode, 0).unwrap();

    assert_eq!(fixture.bytes(), &expected_empty_data_reply(0x30));
    assert_eq!(fixture.next_session().player_lights(), Some(0x01));
    assert!(!fixture.next_session().protocol_ready());
    assert!(ready.next_session().protocol_ready());
    assert_eq!(zero_lights.next_session().player_lights(), Some(0x00));
    assert!(!zero_lights.next_session().protocol_ready());
    assert_eq!(report_mode.player_lights(), None);
    assert!(!report_mode.protocol_ready());
}

#[test]
fn imu_candidate_matches_fixture_and_same_mode_starts_a_new_epoch() {
    let fixture = stateful_reply::<Pro>(0x40, &[0x02], ProtocolSession::default(), 0).unwrap();
    let current = ProtocolSession::default()
        .with_report_mode(0x30)
        .with_vibration_enabled(true)
        .with_imu_mode(ImuMode::Quaternion1)
        .with_imu_encoding_state(ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123)));

    let reset = stateful_reply::<Pro>(0x40, &[0x02], current, 0).unwrap();

    assert_eq!(fixture.bytes(), &expected_empty_data_reply(0x40));
    assert_eq!(fixture.next_session().imu_mode(), ImuMode::Quaternion1);
    assert_eq!(
        fixture.next_session().imu_encoding_state(),
        ImuEncodingState::default()
    );
    assert_eq!(reset.next_session().imu_mode(), ImuMode::Quaternion1);
    assert_eq!(
        reset.next_session().imu_encoding_state(),
        ImuEncodingState::default()
    );
    assert_eq!(reset.next_session().report_mode(), Some(0x30));
    assert!(reset.next_session().vibration_enabled());
    assert_eq!(
        current.imu_encoding_state(),
        ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123))
    );
}

#[test]
fn every_model_accepts_all_declared_imu_modes() {
    assert_all_imu_modes::<Pro>();
    assert_all_imu_modes::<JoyConL>();
    assert_all_imu_modes::<JoyConR>();
}

#[test]
fn vibration_candidate_matches_fixture_and_preserves_other_state() {
    let fixture = stateful_reply::<Pro>(0x48, &[0x01], ProtocolSession::default(), 0).unwrap();
    let current = ProtocolSession::default()
        .with_report_mode(0x30)
        .with_player_lights(0x01)
        .with_vibration_enabled(true);

    let disabled = stateful_reply::<Pro>(0x48, &[0x00], current, 0).unwrap();

    assert_eq!(fixture.bytes(), &expected_empty_data_reply(0x48));
    assert!(fixture.next_session().vibration_enabled());
    assert!(!disabled.next_session().vibration_enabled());
    assert!(disabled.next_session().protocol_ready());
    assert!(current.vibration_enabled());
}

#[test]
fn stateful_subcommands_require_one_argument_without_changing_current() {
    let current = ProtocolSession::default()
        .with_report_mode(0x30)
        .with_player_lights(0x01)
        .with_vibration_enabled(true);

    for subcommand_id in [0x03, 0x30, 0x40, 0x48] {
        assert_eq!(
            stateful_result::<Pro>(subcommand_id, &[], current, 0),
            Err(ProtocolError::MissingSubcommandArgument { subcommand_id })
        );
    }
    assert!(current.protocol_ready());
    assert!(current.vibration_enabled());
}

#[test]
fn invalid_imu_and_vibration_values_return_structured_errors() {
    let current = ProtocolSession::default()
        .with_imu_mode(ImuMode::Quaternion1)
        .with_imu_encoding_state(ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123)));

    assert_eq!(
        stateful_result::<Pro>(0x40, &[0x06], current, 0),
        Err(ProtocolError::UnsupportedImuMode {
            requested: 0x06,
            accepted: ACCEPTED_IMU_MODES,
        })
    );
    assert_eq!(
        stateful_result::<Pro>(0x48, &[0x02], current, 0),
        Err(ProtocolError::InvalidVibrationValue { requested: 0x02 })
    );
    assert_eq!(
        current.imu_encoding_state(),
        ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123))
    );
}

#[test]
fn stateful_reply_uses_the_provided_input_state_and_timer() {
    let state = InputState::<Pro>::neutral().with_sticks(
        Stick::raw(0x123, 0xABC).unwrap(),
        Stick::raw(0xFFF, 0x000).unwrap(),
    );

    let prepared =
        stateful_result_with_state(0x48, &[0x01], ProtocolSession::default(), 0xFE, &state)
            .unwrap()
            .unwrap();

    assert_eq!(
        &prepared.bytes()[..13],
        &[
            0x21, 0xFE, 0x80, 0x00, 0x00, 0x00, 0x23, 0xC1, 0xAB, 0xFF, 0x0F, 0x00, 0x00
        ]
    );
    assert_eq!(prepared.next_timer(), 0xFF);
}

#[test]
fn stateful_errors_match_the_python_display_contract() {
    let cases = [
        (
            ProtocolError::MissingSubcommandArgument {
                subcommand_id: 0x03,
            },
            "set input report mode subcommand must include one argument byte",
        ),
        (
            ProtocolError::MissingSubcommandArgument {
                subcommand_id: 0x30,
            },
            "set player lights subcommand must include one argument byte",
        ),
        (
            ProtocolError::MissingSubcommandArgument {
                subcommand_id: 0x40,
            },
            "enable IMU subcommand must include one argument byte",
        ),
        (
            ProtocolError::MissingSubcommandArgument {
                subcommand_id: 0x48,
            },
            "enable vibration subcommand must include one argument byte",
        ),
        (
            ProtocolError::UnsupportedImuMode {
                requested: 0x06,
                accepted: ACCEPTED_IMU_MODES,
            },
            "enable IMU subcommand argument must be one of: 0x00, 0x01, 0x02, 0x03, 0x04, 0x05",
        ),
        (
            ProtocolError::InvalidVibrationValue { requested: 0x02 },
            "enable vibration subcommand argument must be 0x00 or 0x01",
        ),
    ];

    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
    }
}

#[test]
fn stateful_subcommands_ignore_payload_after_the_first_argument() {
    let current = ProtocolSession::default();

    for (subcommand_id, argument) in [(0x03, 0x30), (0x30, 0x01), (0x40, 0x02), (0x48, 0x01)] {
        let exact = stateful_reply::<Pro>(subcommand_id, &[argument], current, 9).unwrap();
        let trailing =
            stateful_reply::<Pro>(subcommand_id, &[argument, 0xAA, 0xBB], current, 9).unwrap();

        assert_eq!(trailing, exact);
    }
}

#[test]
fn stateful_handler_defers_other_subcommands() {
    let current = ProtocolSession::default();

    for subcommand_id in [0x02, 0x04, 0x08, 0x10, 0x21, 0x99] {
        assert_eq!(
            stateful_result::<Pro>(subcommand_id, &[], current, 0).unwrap(),
            None
        );
    }
}

fn assert_all_imu_modes<M: ControllerModel>() {
    let cases = [
        (0x00, ImuMode::Disabled),
        (0x01, ImuMode::Standard),
        (0x02, ImuMode::Quaternion1),
        (0x03, ImuMode::Quaternion2),
        (0x04, ImuMode::Quaternion3),
        (0x05, ImuMode::Quaternion4),
    ];

    for (requested, expected) in cases {
        let prepared =
            stateful_reply::<M>(0x40, &[requested], ProtocolSession::default(), requested).unwrap();

        assert_eq!(prepared.next_session().imu_mode(), expected);
        assert_eq!(
            prepared.next_session().imu_encoding_state(),
            ImuEncodingState::default()
        );
        assert_eq!(prepared.next_timer(), requested.wrapping_add(1));
    }
}

fn stateful_reply<M: ControllerModel>(
    subcommand_id: u8,
    payload: &[u8],
    current: ProtocolSession,
    timer: u8,
) -> Result<PreparedSessionReply, ProtocolError> {
    Ok(
        stateful_result::<M>(subcommand_id, payload, current, timer)?
            .expect("fixture subcommand is stateful"),
    )
}

fn stateful_result<M: ControllerModel>(
    subcommand_id: u8,
    payload: &[u8],
    current: ProtocolSession,
    timer: u8,
) -> Result<Option<PreparedSessionReply>, ProtocolError> {
    let state = InputState::<M>::neutral();
    stateful_result_with_state(subcommand_id, payload, current, timer, &state)
}

fn stateful_result_with_state<M: ControllerModel>(
    subcommand_id: u8,
    payload: &[u8],
    current: ProtocolSession,
    timer: u8,
    state: &InputState<M>,
) -> Result<Option<PreparedSessionReply>, ProtocolError> {
    let mut raw = vec![0x01, 0x0A];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.push(subcommand_id);
    raw.extend_from_slice(payload);
    let request = parse_output_report(&raw)
        .unwrap()
        .subcommand()
        .expect("0x01 output report has a subcommand");

    try_prepare_stateful_reply(request, state, timer, current)
}

fn expected_empty_data_reply(subcommand_id: u8) -> [u8; 50] {
    let mut expected = [0; 50];
    expected[..13].copy_from_slice(&[
        0x21, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x08, 0x80, 0x00, 0x08, 0x80, 0x00,
    ]);
    expected[13] = 0x80;
    expected[14] = subcommand_id;
    expected
}
