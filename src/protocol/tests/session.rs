use crate::protocol::{
    imu::{ImuEncodingState, ImuMode},
    session::ProtocolSession,
};

#[test]
fn protocol_session_starts_with_the_python_baseline_state() {
    let session = ProtocolSession::default();

    assert_eq!(session.report_mode(), None);
    assert!(!session.report_mode_supported());
    assert_eq!(session.unsupported_report_mode(), None);
    assert_eq!(session.player_lights(), None);
    assert_eq!(session.imu_mode(), ImuMode::Disabled);
    assert!(!session.imu_enabled());
    assert_eq!(session.imu_encoding_state(), ImuEncodingState::default());
    assert!(!session.vibration_enabled());
    assert!(!session.protocol_ready());
}

#[test]
fn supported_report_mode_and_nonzero_lights_are_both_required_for_readiness() {
    let session = ProtocolSession::default();
    let report_mode_only = session.with_report_mode(0x30);
    let zero_lights = report_mode_only.with_player_lights(0x00);
    let ready = zero_lights.with_player_lights(0x01);
    let reverse_order = session.with_player_lights(0x10).with_report_mode(0x30);

    assert!(!session.protocol_ready());
    assert!(!report_mode_only.protocol_ready());
    assert!(!zero_lights.protocol_ready());
    assert!(ready.protocol_ready());
    assert!(reverse_order.protocol_ready());
    assert!(!ready.with_player_lights(0x00).protocol_ready());
}

#[test]
fn unsupported_report_mode_is_preserved_without_rounding_or_readiness() {
    let ready = ProtocolSession::default()
        .with_report_mode(0x30)
        .with_player_lights(0x01);

    let unsupported = ready.with_report_mode(0x3F);
    let supported_again = unsupported.with_report_mode(0x30);

    assert_eq!(unsupported.report_mode(), Some(0x3F));
    assert!(!unsupported.report_mode_supported());
    assert_eq!(unsupported.unsupported_report_mode(), Some(0x3F));
    assert!(!unsupported.protocol_ready());
    assert_eq!(ready.report_mode(), Some(0x30));
    assert!(ready.report_mode_supported());
    assert_eq!(ready.unsupported_report_mode(), None);
    assert!(ready.protocol_ready());
    assert_eq!(supported_again.report_mode(), Some(0x30));
    assert!(supported_again.report_mode_supported());
    assert_eq!(supported_again.unsupported_report_mode(), None);
    assert!(supported_again.protocol_ready());
}

#[test]
fn accepted_imu_mode_starts_a_new_epoch_and_preserves_other_state() {
    let current = ProtocolSession::default()
        .with_report_mode(0x30)
        .with_player_lights(0x01)
        .with_vibration_enabled(true)
        .with_imu_mode(ImuMode::Quaternion1)
        .with_imu_encoding_state(ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123)));

    let candidate = current.with_imu_mode(ImuMode::Quaternion1);

    assert_eq!(candidate.imu_mode(), ImuMode::Quaternion1);
    assert!(candidate.imu_enabled());
    assert_eq!(candidate.imu_encoding_state(), ImuEncodingState::default());
    assert_eq!(candidate.report_mode(), Some(0x30));
    assert_eq!(candidate.player_lights(), Some(0x01));
    assert!(candidate.vibration_enabled());
    assert!(candidate.protocol_ready());
    assert!(!candidate.with_vibration_enabled(false).vibration_enabled());
    assert_eq!(
        current.imu_encoding_state(),
        ImuEncodingState::new([0.1, 0.2, 0.3, 0.9], Some(123))
    );
}

#[test]
fn separate_connection_sessions_do_not_share_state() {
    let first_connection = ProtocolSession::default()
        .with_report_mode(0x30)
        .with_player_lights(0x01)
        .with_vibration_enabled(true);
    let second_connection = ProtocolSession::default();

    assert!(first_connection.protocol_ready());
    assert!(first_connection.vibration_enabled());
    assert!(!second_connection.protocol_ready());
    assert!(!second_connection.vibration_enabled());
}
