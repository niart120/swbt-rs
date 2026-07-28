use std::{error::Error as StdError, fmt};

use crate::{
    input::InputState,
    model::ControllerModel,
    protocol::{OutputReport, ProtocolError, RawRumble, SwitchHidProtocol, parse_output_report},
    runtime::{
        connection::ObservedSubcommands,
        sender::ReportSender,
        transport::{HidChannel, SendAcceptance, TransportError, TransportPort},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputHandling {
    RumbleOnly,
    ReplyAccepted(SendAcceptance),
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T07 preserves output context before T21 worker integration"
    )
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputObservation {
    pub(crate) channel: HidChannel,
    pub(crate) report_id: u8,
    pub(crate) packet_id: u8,
    pub(crate) rumble: RawRumble,
}

pub(crate) struct OutputHandlingContext<'a, M: ControllerModel> {
    pub(crate) observe_output: &'a mut dyn FnMut(OutputObservation),
    pub(crate) protocol: &'a SwitchHidProtocol<M>,
    pub(crate) current: &'a InputState<M>,
    pub(crate) observed: &'a mut ObservedSubcommands,
    pub(crate) sender: &'a mut ReportSender<M>,
    pub(crate) transport: &'a mut dyn TransportPort,
}

#[derive(Debug)]
pub(crate) enum OutputHandlingError {
    Protocol(ProtocolError),
    Transport(TransportError),
}

impl From<ProtocolError> for OutputHandlingError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl From<TransportError> for OutputHandlingError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

impl fmt::Display for OutputHandlingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(formatter, "output protocol error: {error}"),
            Self::Transport(error) => write!(formatter, "output transport error: {error}"),
        }
    }
}

impl StdError for OutputHandlingError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Protocol(error) => Some(error),
            Self::Transport(error) => Some(error),
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T07 defines output handling before T21 worker integration"
    )
)]
pub(crate) fn handle_output<M: ControllerModel>(
    channel: HidChannel,
    raw: &[u8],
    context: OutputHandlingContext<'_, M>,
) -> Result<OutputHandling, OutputHandlingError> {
    let OutputHandlingContext {
        observe_output,
        protocol,
        current,
        observed,
        sender,
        transport,
    } = context;
    let output = parse_output_report(raw)?;
    observe_output(OutputObservation {
        channel,
        report_id: output.report_id(),
        packet_id: output.packet_id(),
        rumble: *output.rumble(),
    });
    match output {
        OutputReport::Rumble { .. } => Ok(OutputHandling::RumbleOnly),
        OutputReport::Subcommand { request, .. } => {
            observed.observe(request.id());
            let prepared = sender.prepare_reply(protocol, request, current)?;
            let accepted = sender.send_reply(prepared, transport)?;
            Ok(OutputHandling::ReplyAccepted(accepted))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        input::{ImuFrame, InputState},
        model::Pro,
        protocol::{DeviceInfoBluetoothAddress, ProtocolError, SwitchHidProtocol},
        runtime::{
            connection::ObservedSubcommands,
            output::{
                OutputHandling, OutputHandlingContext, OutputHandlingError, OutputObservation,
                handle_output,
            },
            sender::ReportSender,
            transport::{
                HidChannel, TransportErrorKind, TransportEvent, TransportPort, activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const PACKET_ID: u8 = 0x5a;
    const RAW_RUMBLE: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];

    #[test]
    fn both_channels_observe_before_preparation_and_keep_observations_after_send_rejection() {
        for channel in [HidChannel::Control, HidChannel::Interrupt] {
            let mut harness = Harness::new();
            harness
                .control
                .script_sends([ScriptedSendOutcome::Rejected, ScriptedSendOutcome::Accepted]);
            let initial_session = harness.sender.session();

            let semantic_error = harness
                .handle(channel, &subcommand_report(0x40, &[]))
                .expect_err("missing IMU argument must fail after observation");
            assert_eq!(
                protocol_error(semantic_error),
                ProtocolError::MissingSubcommandArgument {
                    subcommand_id: 0x40
                }
            );
            assert_observed(&harness.observed, 0x40);
            assert_output_observation(&harness.output_observations[0], channel, 0x01);
            assert_eq!(harness.sender.timer(), 0);
            assert_eq!(harness.sender.session(), initial_session);

            let unsupported = harness
                .handle(channel, &subcommand_report(0xaa, &[]))
                .expect_err("unsupported subcommand must fail after observation");
            assert_eq!(
                protocol_error(unsupported),
                ProtocolError::UnsupportedSubcommand {
                    subcommand_id: 0xaa
                }
            );
            assert_observed(&harness.observed, 0xaa);
            assert_output_observation(&harness.output_observations[1], channel, 0x01);
            assert_eq!(harness.sender.timer(), 0);
            assert_eq!(harness.sender.session(), initial_session);

            let rejected = harness
                .handle(channel, &subcommand_report(0x08, &[]))
                .expect_err("scripted reply must be rejected after observation");
            assert_eq!(
                transport_error_kind(rejected),
                TransportErrorKind::SendRejected
            );
            assert_observed(&harness.observed, 0x08);
            assert_output_observation(&harness.output_observations[2], channel, 0x01);
            assert_eq!(harness.sender.timer(), 0);
            assert_eq!(harness.sender.session(), initial_session);

            let accepted = harness
                .handle(channel, &subcommand_report(0x08, &[]))
                .expect("retry reply accepted");
            assert!(matches!(accepted, OutputHandling::ReplyAccepted(_)));
            assert_observed(&harness.observed, 0x08);
            assert_output_observation(&harness.output_observations[3], channel, 0x01);
            assert_eq!(harness.sender.timer(), 1);
            assert_eq!(harness.control.accepted_interrupts().len(), 1);
        }
    }

    #[test]
    fn malformed_and_rumble_only_outputs_do_not_fabricate_runtime_state_on_either_channel() {
        for channel in [HidChannel::Control, HidChannel::Interrupt] {
            let mut harness = Harness::new();
            let initial_session = harness.sender.session();

            let malformed = harness
                .handle(channel, &truncated_subcommand_report())
                .expect_err("truncated subcommand report must fail during parsing");
            assert_eq!(
                protocol_error(malformed),
                ProtocolError::TruncatedOutputReport {
                    report_id: 0x01,
                    minimum: 11,
                    actual: 10,
                }
            );
            assert!(harness.observed.is_empty());
            assert!(harness.output_observations.is_empty());
            assert_eq!(harness.sender.timer(), 0);
            assert_eq!(harness.sender.session(), initial_session);
            assert!(harness.control.accepted_interrupts().is_empty());

            let rumble = harness
                .handle(channel, &rumble_report())
                .expect("valid rumble-only report");
            assert_eq!(rumble, OutputHandling::RumbleOnly);
            assert!(harness.observed.is_empty());
            assert_eq!(harness.output_observations.len(), 1);
            assert_output_observation(&harness.output_observations[0], channel, 0x10);
            assert_eq!(harness.sender.timer(), 0);
            assert_eq!(harness.sender.session(), initial_session);
            assert!(harness.control.accepted_interrupts().is_empty());
        }
    }

    #[test]
    fn in_flight_old_mode_input_finishes_before_accepted_imu_ack_and_new_mode_input() {
        let mut harness = Harness::new();
        let imu_request = subcommand_report(0x40, &[0x02]);
        harness.control.script_sends([
            ScriptedSendOutcome::AcceptedThenEvent(TransportEvent::HidOutput {
                channel: HidChannel::Interrupt,
                payload: imu_request.into_boxed_slice(),
            }),
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
        ]);
        let imu_frame = ImuFrame::raw([0, 0, 4096], [0, 0, 0]);
        harness.state = InputState::neutral().with_imu([imu_frame; 3]);

        harness
            .sender
            .send_input(&harness.protocol, &harness.state, 0, &mut harness.transport)
            .expect("in-flight old-mode input accepted");
        assert_eq!(harness.sender.timer(), 1);
        assert!(!harness.sender.session().imu_enabled());

        let events = harness
            .transport
            .poll(Duration::ZERO)
            .expect("output queued while old-mode input was in flight");
        let [TransportEvent::HidOutput { channel, payload }] = events.as_slice() else {
            panic!("one HID output event must follow the accepted input");
        };
        let handled = harness
            .handle(*channel, payload)
            .expect("IMU mode acknowledgement accepted");
        assert!(matches!(handled, OutputHandling::ReplyAccepted(_)));
        assert_eq!(harness.sender.timer(), 2);
        assert!(harness.sender.session().imu_enabled());

        harness
            .sender
            .send_input(&harness.protocol, &harness.state, 0, &mut harness.transport)
            .expect("first new-mode input accepted");

        let accepted = harness.control.accepted_interrupts();
        assert_eq!(
            accepted
                .iter()
                .map(|report| (report[0], report[1]))
                .collect::<Vec<_>>(),
            [(0x30, 0), (0x21, 1), (0x30, 2)]
        );
        assert_eq!(&accepted[0][13..], &[0; 36]);
        assert_eq!(accepted[1][14], 0x40);
        assert_ne!(&accepted[2][13..], &accepted[0][13..]);
        assert_eq!(harness.sender.timer(), 3);
        assert_eq!(
            harness
                .sender
                .session()
                .imu_encoding_state()
                .previous_report_ns(),
            Some(0)
        );
    }

    struct Harness {
        protocol: SwitchHidProtocol<Pro>,
        sender: ReportSender<Pro>,
        observed: ObservedSubcommands,
        transport: FakeTransport,
        control: FakeTransportControl,
        state: InputState<Pro>,
        output_observations: Vec<OutputObservation>,
    }

    impl Harness {
        fn new() -> Self {
            let (mut transport, control) = FakeTransport::with_limits(8, 8);
            let (notifier, _wake_receiver) = activity_channel();
            transport.open(notifier).expect("open fake transport");
            Self {
                protocol: SwitchHidProtocol::new(
                    None,
                    DeviceInfoBluetoothAddress::from_wire_bytes(DEVICE_INFO_ADDRESS),
                ),
                sender: ReportSender::new(),
                observed: ObservedSubcommands::default(),
                transport,
                control,
                state: InputState::neutral(),
                output_observations: Vec::new(),
            }
        }

        fn handle(
            &mut self,
            channel: HidChannel,
            raw: &[u8],
        ) -> Result<OutputHandling, OutputHandlingError> {
            let output_observations = &mut self.output_observations;
            let mut observe_output = |observation| output_observations.push(observation);
            handle_output(
                channel,
                raw,
                OutputHandlingContext {
                    observe_output: &mut observe_output,
                    protocol: &self.protocol,
                    current: &self.state,
                    observed: &mut self.observed,
                    sender: &mut self.sender,
                    transport: &mut self.transport,
                },
            )
        }
    }

    fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut raw = vec![0x01, PACKET_ID];
        raw.extend_from_slice(&RAW_RUMBLE);
        raw.push(subcommand_id);
        raw.extend_from_slice(payload);
        raw
    }

    fn truncated_subcommand_report() -> Vec<u8> {
        let mut raw = vec![0x01, PACKET_ID];
        raw.extend_from_slice(&RAW_RUMBLE);
        raw
    }

    fn rumble_report() -> Vec<u8> {
        let mut raw = vec![0x10, PACKET_ID];
        raw.extend_from_slice(&RAW_RUMBLE);
        raw
    }

    fn assert_output_observation(
        observation: &OutputObservation,
        channel: HidChannel,
        report_id: u8,
    ) {
        assert_eq!(observation.channel, channel);
        assert_eq!(observation.report_id, report_id);
        assert_eq!(observation.packet_id, PACKET_ID);
        assert_eq!(observation.rumble.bytes(), &RAW_RUMBLE);
    }

    fn assert_observed(observed: &ObservedSubcommands, subcommand_id: u8) {
        let mut snapshot = observed.clone();
        assert!(!snapshot.observe(subcommand_id));
    }

    fn protocol_error(error: OutputHandlingError) -> ProtocolError {
        let OutputHandlingError::Protocol(error) = error else {
            panic!("expected protocol error");
        };
        error
    }

    fn transport_error_kind(error: OutputHandlingError) -> TransportErrorKind {
        let OutputHandlingError::Transport(error) = error else {
            panic!("expected transport error");
        };
        error.kind()
    }
}
