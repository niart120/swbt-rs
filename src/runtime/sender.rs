#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 defines report sending before T21 worker integration"
    )
)]

use std::marker::PhantomData;

use crate::{
    input::InputState,
    model::ControllerModel,
    protocol::{
        ImuEncodingState, InputPreparation, PreparedOutputAction, PreparedSessionReply,
        PreparedSubcommandReply, ProtocolSession, SwitchHidProtocol,
    },
    runtime::transport::{SendAcceptance, TransportPort, TransportResult},
};

#[derive(Clone, Copy, Debug, PartialEq)]
struct SenderCommit {
    next_timer: u8,
    session: ProtocolSession,
}

pub(crate) struct ReportSender<M: ControllerModel> {
    committed: SenderCommit,
    model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> ReportSender<M> {
    pub(crate) fn new() -> Self {
        Self {
            committed: SenderCommit {
                next_timer: 0,
                session: ProtocolSession::default(),
            },
            model: PhantomData,
        }
    }

    #[must_use]
    pub(crate) const fn timer(&self) -> u8 {
        self.committed.next_timer
    }

    #[must_use]
    pub(crate) const fn session(&self) -> ProtocolSession {
        self.committed.session
    }

    pub(crate) fn send_input(
        &mut self,
        protocol: &SwitchHidProtocol<M>,
        state: &InputState<M>,
        now_ns: u64,
        transport: &mut dyn TransportPort,
    ) -> TransportResult<SendAcceptance> {
        let prepared: InputPreparation = protocol.prepare_input_report(
            state,
            self.committed.next_timer,
            self.committed.session,
            now_ns,
        );
        let next_imu_encoding_state: ImuEncodingState = prepared.next_imu_encoding_state();
        let candidate = SenderCommit {
            next_timer: prepared.next_timer(),
            session: self
                .committed
                .session
                .with_imu_encoding_state(next_imu_encoding_state),
        };
        self.send_candidate(prepared.bytes(), candidate, transport)
    }

    pub(crate) fn send_reply(
        &mut self,
        prepared: PreparedOutputAction,
        transport: &mut dyn TransportPort,
    ) -> TransportResult<SendAcceptance> {
        match prepared {
            PreparedOutputAction::Reply(reply) => self.send_plain_reply(reply, transport),
            PreparedOutputAction::SessionReply(reply) => self.send_session_reply(reply, transport),
        }
    }

    fn send_plain_reply(
        &mut self,
        prepared: PreparedSubcommandReply,
        transport: &mut dyn TransportPort,
    ) -> TransportResult<SendAcceptance> {
        let candidate = SenderCommit {
            next_timer: prepared.next_timer(),
            session: self.committed.session,
        };
        self.send_candidate(prepared.bytes(), candidate, transport)
    }

    fn send_session_reply(
        &mut self,
        prepared: PreparedSessionReply,
        transport: &mut dyn TransportPort,
    ) -> TransportResult<SendAcceptance> {
        let candidate = SenderCommit {
            next_timer: prepared.next_timer(),
            session: prepared.next_session(),
        };
        self.send_candidate(prepared.bytes(), candidate, transport)
    }

    fn send_candidate(
        &mut self,
        bytes: &[u8],
        candidate: SenderCommit,
        transport: &mut dyn TransportPort,
    ) -> TransportResult<SendAcceptance> {
        let acceptance = transport.send_interrupt(bytes)?;
        self.committed = candidate;
        Ok(acceptance)
    }
}

impl<M: ControllerModel> Default for ReportSender<M> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        input::{ImuFrame, InputState},
        model::Pro,
        protocol::{
            DeviceInfoBluetoothAddress, OutputPreparation, PreparedOutputAction, SwitchHidProtocol,
        },
        runtime::{
            sender::ReportSender,
            transport::{
                TransportErrorKind, TransportEvent, TransportPort, activity_channel,
                fake::{FakeTransport, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

    #[test]
    fn accepted_input_reply_input_share_one_timer() {
        let protocol = protocol();
        let (mut transport, control) = open_transport();
        let mut sender = ReportSender::<Pro>::new();
        let state = InputState::<Pro>::neutral();

        sender
            .send_input(&protocol, &state, 10, &mut transport)
            .expect("first input accepted");
        let reply = output_action(&protocol, &sender, &state, 0x08, &[]);
        sender
            .send_reply(reply, &mut transport)
            .expect("reply accepted");
        sender
            .send_input(&protocol, &state, 20, &mut transport)
            .expect("second input accepted");

        let accepted = control.accepted_interrupts();
        assert_eq!(
            accepted
                .iter()
                .map(|report| (report[0], report[1]))
                .collect::<Vec<_>>(),
            [(0x30, 0), (0x21, 1), (0x30, 2)]
        );
        assert_eq!(sender.timer(), 3);
    }

    #[test]
    fn rejected_candidates_leave_committed_state_and_retry_input_at_the_new_time() {
        let protocol = protocol();
        let (mut transport, control) = open_transport();
        control.script_sends([
            ScriptedSendOutcome::Rejected,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Rejected,
            ScriptedSendOutcome::AcceptedThenDisconnect { reason: Some(0x13) },
        ]);
        let mut sender = ReportSender::<Pro>::new();
        let initial_session = sender.session();
        let stateful_raw = output_action(
            &protocol,
            &sender,
            &InputState::<Pro>::neutral(),
            0x40,
            &[0x02],
        );

        let error = sender
            .send_reply(stateful_raw, &mut transport)
            .expect_err("first session reply rejected");
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(sender.timer(), 0);
        assert_eq!(sender.session(), initial_session);

        let retry = output_action(
            &protocol,
            &sender,
            &InputState::<Pro>::neutral(),
            0x40,
            &[0x02],
        );
        sender
            .send_reply(retry, &mut transport)
            .expect("session reply retry accepted");
        assert_eq!(sender.timer(), 1);
        assert!(sender.session().imu_enabled());

        let state = InputState::<Pro>::neutral().with_imu([
            ImuFrame::raw([1, 2, 3], [0, 0, 1000]),
            ImuFrame::raw([4, 5, 6], [0, 0, 1000]),
            ImuFrame::raw([7, 8, 9], [0, 0, 1000]),
        ]);
        sender
            .send_input(&protocol, &state, 1_000_000_000, &mut transport)
            .expect("baseline quaternion input accepted");
        assert_eq!(sender.timer(), 2);
        assert_eq!(
            sender.session().imu_encoding_state().previous_report_ns(),
            Some(1_000_000_000)
        );

        let committed_before_rejection = sender.session();
        let rejected_candidate =
            protocol.prepare_input_report(&state, sender.timer(), sender.session(), 2_000_000_000);
        let error = sender
            .send_input(&protocol, &state, 2_000_000_000, &mut transport)
            .expect_err("first input rejected");
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(sender.timer(), 2);
        assert_eq!(sender.session(), committed_before_rejection);

        let retry_candidate =
            protocol.prepare_input_report(&state, sender.timer(), sender.session(), 3_000_000_000);
        assert_ne!(rejected_candidate, retry_candidate);
        sender
            .send_input(&protocol, &state, 3_000_000_000, &mut transport)
            .expect("input retry accepted");
        assert_eq!(sender.timer(), 3);
        assert_eq!(
            sender.session().imu_encoding_state().previous_report_ns(),
            Some(3_000_000_000)
        );
        let accepted = control.accepted_interrupts();
        assert_eq!(
            accepted
                .iter()
                .map(|report| (report[0], report[1]))
                .collect::<Vec<_>>(),
            [(0x21, 0), (0x30, 1), (0x30, 2)]
        );
        assert_eq!(accepted[2].as_ref(), retry_candidate.bytes());

        let committed_before_disconnect = sender.session();
        assert_eq!(
            transport
                .poll(Duration::ZERO)
                .expect("accepted send is followed by disconnect"),
            [TransportEvent::Disconnected { reason: Some(0x13) }]
        );
        assert_eq!(sender.timer(), 3);
        assert_eq!(sender.session(), committed_before_disconnect);
    }

    fn protocol() -> SwitchHidProtocol<Pro> {
        SwitchHidProtocol::new(
            None,
            DeviceInfoBluetoothAddress::from_wire_bytes(DEVICE_INFO_ADDRESS),
        )
    }

    fn open_transport() -> (
        FakeTransport,
        crate::runtime::transport::fake::FakeTransportControl,
    ) {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _wake_receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        (transport, control)
    }

    fn output_action(
        protocol: &SwitchHidProtocol<Pro>,
        sender: &ReportSender<Pro>,
        state: &InputState<Pro>,
        subcommand_id: u8,
        payload: &[u8],
    ) -> PreparedOutputAction {
        let mut raw = vec![0x01, 0];
        raw.extend_from_slice(&NEUTRAL_RUMBLE);
        raw.push(subcommand_id);
        raw.extend_from_slice(payload);
        let prepared = protocol
            .prepare_output_report(&raw, state, sender.timer(), sender.session())
            .expect("valid output report");
        let OutputPreparation::Subcommand { outcome, .. } = prepared else {
            panic!("0x01 must prepare a subcommand");
        };
        outcome.expect("supported subcommand")
    }
}
