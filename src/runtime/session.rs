#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T17 defines connection-session coordination before M2 worker integration"
    )
)]

use std::{error::Error as StdError, fmt, num::NonZeroU64};

use crate::{
    model::ControllerModel,
    runtime::{
        connection::ObservedSubcommands, periodic::PeriodicPolicy, sender::ReportSender,
        state::InputStateStore, transport::TransportEvent,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct ConnectionSessionId(NonZeroU64);

impl ConnectionSessionId {
    #[must_use]
    pub(crate) const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SessionError {
    IdExhausted,
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IdExhausted => formatter.write_str("connection session ID exhausted"),
        }
    }
}

impl StdError for SessionError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SessionEvent {
    session_id: ConnectionSessionId,
    event: TransportEvent,
}

pub(crate) struct ConnectionSessions {
    last_issued: u64,
    current: Option<ConnectionSessionId>,
}

impl ConnectionSessions {
    pub(crate) const fn new() -> Self {
        Self {
            last_issued: 0,
            current: None,
        }
    }

    #[must_use]
    pub(crate) const fn current(&self) -> Option<ConnectionSessionId> {
        self.current
    }

    pub(crate) fn begin_periodic<M: ControllerModel>(
        &mut self,
        sender: &mut ReportSender<M>,
        reporting: &mut PeriodicPolicy,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> Result<ConnectionSessionId, SessionError> {
        let next = self.prepare_next()?;
        reporting.reset_for_new_session();
        self.commit(next, sender, observed, input);
        Ok(next)
    }

    pub(crate) fn begin_direct<M: ControllerModel>(
        &mut self,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> Result<ConnectionSessionId, SessionError> {
        let next = self.prepare_next()?;
        self.commit(next, sender, observed, input);
        Ok(next)
    }

    fn prepare_next(&self) -> Result<ConnectionSessionId, SessionError> {
        let next = self
            .last_issued
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .map(ConnectionSessionId)
            .ok_or(SessionError::IdExhausted)?;
        Ok(next)
    }

    fn commit<M: ControllerModel>(
        &mut self,
        next: ConnectionSessionId,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) {
        sender.reset_for_new_session();
        observed.reset();
        input.reset_to_neutral();
        self.last_issued = next.get();
        self.current = Some(next);
    }

    pub(crate) fn end_current(&mut self, session_id: ConnectionSessionId) -> bool {
        if self.current != Some(session_id) {
            return false;
        }
        self.current = None;
        true
    }

    #[must_use]
    pub(crate) fn tag_current(&self, event: TransportEvent) -> Option<SessionEvent> {
        self.current
            .map(|session_id| SessionEvent { session_id, event })
    }

    pub(crate) fn take_current(&self, event: SessionEvent) -> Option<TransportEvent> {
        (Some(event.session_id) == self.current).then_some(event.event)
    }
}

impl Default for ConnectionSessions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::{
            DeviceInfoBluetoothAddress, OutputReport, PreparedOutputAction, ProtocolSession,
            SwitchHidProtocol, parse_output_report,
        },
        runtime::{
            connection::ObservedSubcommands,
            output::{OutputHandling, OutputHandlingError},
            periodic::PeriodicPolicy,
            sender::ReportSender,
            state::InputStateStore,
            transport::{
                TransportEvent, TransportPort, activity_channel,
                fake::{FakeTransport, FakeTransportControl},
            },
        },
    };

    use super::ConnectionSessions;

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
    const REPORT_PERIOD: Duration = Duration::from_millis(8);

    #[test]
    fn new_periodic_session_resets_every_connection_scoped_state() {
        let protocol = protocol();
        let (mut transport, _control) = open_transport();
        let mut sessions = ConnectionSessions::new();
        let mut sender = ReportSender::<Pro>::new();
        let mut reporting = PeriodicPolicy::new(REPORT_PERIOD).expect("valid policy");
        let mut observed = ObservedSubcommands::default();
        let mut input = InputStateStore::<Pro>::new();

        let first = sessions
            .begin_periodic(&mut sender, &mut reporting, &mut observed, &mut input)
            .expect("first session");
        input.commit(pressed_state(ButtonKind::A));
        for id in [0x03, 0x30, 0x40, 0x48] {
            assert!(observed.observe(id));
        }

        accept_reply(
            &protocol,
            &mut sender,
            &input.snapshot(),
            0x03,
            &[0x30],
            &mut transport,
        );
        accept_reply(
            &protocol,
            &mut sender,
            &input.snapshot(),
            0x30,
            &[0x01],
            &mut transport,
        );
        accept_reply(
            &protocol,
            &mut sender,
            &input.snapshot(),
            0x48,
            &[0x01],
            &mut transport,
        );
        let imu_mode = output_action(&protocol, &sender, &input.snapshot(), 0x40, &[0x02]);
        let reply_acceptance = sender
            .send_reply(imu_mode, &mut transport)
            .expect("IMU mode reply accepted");
        let completion =
            Ok::<_, OutputHandlingError>(OutputHandling::ReplyAccepted(reply_acceptance));
        reporting
            .record_output_completion(Duration::from_millis(50), &completion)
            .expect("holdoff deadline");
        sender
            .send_input(&protocol, &input.snapshot(), 100_000_000, &mut transport)
            .expect("input accepted");

        assert_eq!(sender.timer(), 5);
        assert_eq!(sender.session().report_mode(), Some(0x30));
        assert_eq!(sender.session().player_lights(), Some(0x01));
        assert!(sender.session().vibration_enabled());
        assert!(sender.session().imu_enabled());
        assert_eq!(
            sender.session().imu_encoding_state().previous_report_ns(),
            Some(100_000_000)
        );
        assert_eq!(
            reporting.reply_holdoff_until(),
            Some(Duration::from_millis(350))
        );
        assert!(!observed.is_empty());
        assert_ne!(input.snapshot(), InputState::<Pro>::neutral());

        let second = sessions
            .begin_periodic(&mut sender, &mut reporting, &mut observed, &mut input)
            .expect("second session");

        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
        assert_eq!(sessions.current(), Some(second));
        assert_eq!(sender.timer(), 0);
        assert_eq!(sender.session(), ProtocolSession::default());
        assert_eq!(reporting.reply_holdoff_until(), None);
        assert!(observed.is_empty());
        assert_eq!(input.snapshot(), InputState::<Pro>::neutral());
    }

    #[test]
    fn events_tagged_by_an_old_session_are_discarded() {
        let mut sessions = ConnectionSessions::new();
        let mut sender = ReportSender::<Pro>::new();
        let mut observed = ObservedSubcommands::default();
        let mut input = InputStateStore::<Pro>::new();

        assert!(sessions.tag_current(TransportEvent::Connected).is_none());
        let first = sessions
            .begin_direct(&mut sender, &mut observed, &mut input)
            .expect("first session");
        let stale = sessions
            .tag_current(TransportEvent::Disconnected { reason: Some(0x13) })
            .expect("current session tags its event");
        assert!(sessions.end_current(first));
        assert_eq!(sessions.current(), None);

        let second = sessions
            .begin_direct(&mut sender, &mut observed, &mut input)
            .expect("second session");

        assert_eq!(first.get(), 1);
        assert_eq!(second.get(), 2);
        assert_eq!(sessions.take_current(stale), None);
        let current = sessions
            .tag_current(TransportEvent::Connected)
            .expect("new session tags its event");
        assert_eq!(
            sessions.take_current(current),
            Some(TransportEvent::Connected)
        );
    }

    fn pressed_state(kind: ButtonKind) -> InputState<Pro> {
        let button = ProButton::try_from(kind).expect("button supported by Pro Controller");
        InputState::neutral().with_buttons([button])
    }

    fn protocol() -> SwitchHidProtocol<Pro> {
        SwitchHidProtocol::new(
            None,
            DeviceInfoBluetoothAddress::from_wire_bytes(DEVICE_INFO_ADDRESS),
        )
    }

    fn open_transport() -> (FakeTransport, FakeTransportControl) {
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
        let OutputReport::Subcommand { request, .. } =
            parse_output_report(&raw).expect("valid output report")
        else {
            panic!("0x01 must prepare a subcommand");
        };
        sender
            .prepare_reply(protocol, request, state)
            .expect("supported subcommand")
    }

    fn accept_reply(
        protocol: &SwitchHidProtocol<Pro>,
        sender: &mut ReportSender<Pro>,
        state: &InputState<Pro>,
        subcommand_id: u8,
        payload: &[u8],
        transport: &mut dyn TransportPort,
    ) {
        let action = output_action(protocol, sender, state, subcommand_id, payload);
        sender
            .send_reply(action, transport)
            .expect("session reply accepted");
    }
}
