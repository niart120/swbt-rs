use std::{error::Error as StdError, fmt, time::Duration};

use crate::{
    input::InputState,
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    runtime::{
        connection::ObservedSubcommands,
        scheduler::{ReportScheduler, SchedulerError, TickDecision},
        sender::ReportSender,
        session::ConnectionSessionId,
        transport::{HidChannel, SendAcceptance, TransportPort, TransportResult},
    },
};

const BOOTSTRAP_RETRY: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HandshakeCompletion {
    session_id: ConnectionSessionId,
}

impl HandshakeCompletion {
    #[must_use]
    pub(crate) const fn session_id(self) -> ConnectionSessionId {
        self.session_id
    }
}

#[derive(Debug)]
pub(crate) enum HandshakeProgress {
    StaleSession,
    WaitingForTopology,
    WaitingUntil {
        deadline: Duration,
    },
    BootstrapAttempted {
        result: TransportResult<SendAcceptance>,
        skipped: u64,
    },
    SubcommandObserved,
}

#[derive(Debug)]
pub(crate) enum HandshakeError {
    Scheduler(SchedulerError),
    ClockOverflow,
}

impl From<SchedulerError> for HandshakeError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(error)
    }
}

impl fmt::Display for HandshakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scheduler(error) => write!(formatter, "handshake schedule failed: {error}"),
            Self::ClockOverflow => {
                formatter.write_str("handshake monotonic time exceeds the protocol range")
            }
        }
    }
}

impl StdError for HandshakeError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Scheduler(error) => Some(error),
            Self::ClockOverflow => None,
        }
    }
}

pub(crate) struct Handshake {
    session_id: ConnectionSessionId,
    completion_condition: HandshakeCompletionCondition,
    link_observed: bool,
    control_observed: bool,
    interrupt_observed: bool,
    retry: Option<ReportScheduler>,
    bootstrap_accepted: bool,
    bootstrap_stopped: bool,
}

#[derive(Clone, Copy)]
enum HandshakeCompletionCondition {
    FirstSubcommand,
    ProtocolReady,
}

impl HandshakeCompletionCondition {
    fn is_satisfied(self, observed: &ObservedSubcommands, protocol_ready: bool) -> bool {
        match self {
            Self::FirstSubcommand => !observed.is_empty(),
            Self::ProtocolReady => protocol_ready,
        }
    }
}

impl Handshake {
    pub(crate) const fn new(session_id: ConnectionSessionId) -> Self {
        Self::with_completion_condition(session_id, HandshakeCompletionCondition::FirstSubcommand)
    }

    pub(crate) const fn until_protocol_ready(session_id: ConnectionSessionId) -> Self {
        Self::with_completion_condition(session_id, HandshakeCompletionCondition::ProtocolReady)
    }

    const fn with_completion_condition(
        session_id: ConnectionSessionId,
        completion_condition: HandshakeCompletionCondition,
    ) -> Self {
        Self {
            session_id,
            completion_condition,
            link_observed: false,
            control_observed: false,
            interrupt_observed: false,
            retry: None,
            bootstrap_accepted: false,
            bootstrap_stopped: false,
        }
    }

    pub(crate) fn observe_link(&mut self, session_id: ConnectionSessionId) {
        if session_id == self.session_id {
            self.link_observed = true;
        }
    }

    pub(crate) fn observe_channel(&mut self, session_id: ConnectionSessionId, channel: HidChannel) {
        if session_id != self.session_id {
            return;
        }
        match channel {
            HidChannel::Control => self.control_observed = true,
            HidChannel::Interrupt => self.interrupt_observed = true,
        }
    }

    #[must_use]
    pub(crate) const fn topology_ready(&self) -> bool {
        self.link_observed && self.control_observed && self.interrupt_observed
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn bootstrap_stopped(&self) -> bool {
        self.bootstrap_stopped
    }

    #[must_use]
    pub(crate) const fn completion(&self) -> Option<HandshakeCompletion> {
        if self.bootstrap_stopped {
            Some(HandshakeCompletion {
                session_id: self.session_id,
            })
        } else {
            None
        }
    }

    #[must_use]
    pub(crate) fn next_deadline(&self) -> Option<Duration> {
        if self.bootstrap_stopped {
            return None;
        }
        self.retry.as_ref().map(ReportScheduler::next_deadline)
    }

    pub(crate) fn step<M: ControllerModel>(
        &mut self,
        session_id: ConnectionSessionId,
        now: Duration,
        observed: &ObservedSubcommands,
        protocol: &SwitchHidProtocol<M>,
        sender: &mut ReportSender<M>,
        transport: &mut dyn TransportPort,
    ) -> Result<HandshakeProgress, HandshakeError> {
        if session_id != self.session_id {
            return Ok(HandshakeProgress::StaleSession);
        }
        if self.bootstrap_stopped {
            return Ok(HandshakeProgress::SubcommandObserved);
        }
        if !self.topology_ready() {
            return Ok(HandshakeProgress::WaitingForTopology);
        }
        if self.bootstrap_accepted
            && self
                .completion_condition
                .is_satisfied(observed, sender.session().protocol_ready())
        {
            self.stop_bootstrap();
            return Ok(HandshakeProgress::SubcommandObserved);
        }

        match self.retry.as_mut() {
            None => {
                let now_ns = monotonic_ns(now)?;
                let scheduler = ReportScheduler::start(now, BOOTSTRAP_RETRY)?;
                self.retry = Some(scheduler);
                let result = send_bootstrap(now_ns, protocol, sender, transport);
                Ok(self.record_bootstrap_attempt(
                    observed,
                    sender.session().protocol_ready(),
                    result,
                    0,
                ))
            }
            Some(scheduler) => {
                let deadline = scheduler.next_deadline();
                if now < deadline {
                    return Ok(HandshakeProgress::WaitingUntil { deadline });
                }
                let now_ns = monotonic_ns(now)?;
                let TickDecision::Due { skipped } = scheduler.step(now)? else {
                    return Ok(HandshakeProgress::WaitingUntil {
                        deadline: scheduler.next_deadline(),
                    });
                };
                let result = send_bootstrap(now_ns, protocol, sender, transport);
                Ok(self.record_bootstrap_attempt(
                    observed,
                    sender.session().protocol_ready(),
                    result,
                    skipped,
                ))
            }
        }
    }

    fn record_bootstrap_attempt(
        &mut self,
        observed: &ObservedSubcommands,
        protocol_ready: bool,
        result: TransportResult<SendAcceptance>,
        skipped: u64,
    ) -> HandshakeProgress {
        if result.is_ok() {
            self.bootstrap_accepted = true;
            if self
                .completion_condition
                .is_satisfied(observed, protocol_ready)
            {
                self.stop_bootstrap();
            }
        }
        HandshakeProgress::BootstrapAttempted { result, skipped }
    }

    fn stop_bootstrap(&mut self) {
        self.retry = None;
        self.bootstrap_stopped = true;
    }
}

fn monotonic_ns(now: Duration) -> Result<u64, HandshakeError> {
    u64::try_from(now.as_nanos()).map_err(|_| HandshakeError::ClockOverflow)
}

fn send_bootstrap<M: ControllerModel>(
    now_ns: u64,
    protocol: &SwitchHidProtocol<M>,
    sender: &mut ReportSender<M>,
    transport: &mut dyn TransportPort,
) -> TransportResult<SendAcceptance> {
    sender.send_input(protocol, &InputState::neutral(), now_ns, transport)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use crate::{
        model::Pro,
        protocol::SwitchHidProtocol,
        runtime::{
            connection::ObservedSubcommands,
            handshake::{Handshake, HandshakeProgress},
            sender::ReportSender,
            session::{ConnectionSessionId, ConnectionSessions},
            state::InputStateStore,
            test_support::runtime_baseline_checkpoint,
            transport::{
                ActivityNotifier, HidChannel, SendAcceptance, TransportCapabilities,
                TransportErrorKind, TransportEvent, TransportPort, TransportResult,
                activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];

    #[test]
    fn bootstrap_waits_for_link_and_both_hid_channels() {
        let protocol = protocol();
        let (mut transport, control) = open_transport();
        let mut sender = ReportSender::<Pro>::new();
        let mut observed = ObservedSubcommands::default();
        let (mut handshake, session_id) = new_handshake(&mut sender, &mut observed);
        let now = Duration::from_millis(100);

        handshake.observe_channel(session_id, HidChannel::Control);
        handshake.observe_channel(session_id, HidChannel::Interrupt);
        assert!(matches!(
            handshake
                .step(
                    session_id,
                    now,
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("wait for link"),
            HandshakeProgress::WaitingForTopology
        ));
        assert!(control.accepted_interrupts().is_empty());
        assert_eq!(sender.timer(), 0);

        handshake.observe_link(session_id);
        assert_bootstrap_accepted(
            handshake
                .step(
                    session_id,
                    now,
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("send bootstrap immediately"),
            0,
        );

        assert!(handshake.topology_ready());
        assert_eq!(
            handshake.next_deadline(),
            Some(Duration::from_millis(1_100))
        );
        assert_eq!(sender.timer(), 1);
        assert_eq!(neutral_attempts(&control), [(0x30, 0, [0, 0, 0])]);

        handshake.observe_channel(session_id, HidChannel::Control);
        assert_waiting_until(
            handshake
                .step(
                    session_id,
                    now,
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("repeated channel event does not resend"),
            Duration::from_millis(1_100),
        );
        assert_eq!(neutral_attempts(&control).len(), 1);
    }

    #[test]
    fn channels_are_order_independent_and_stale_session_signals_are_ignored() {
        for [first, second] in [
            [HidChannel::Control, HidChannel::Interrupt],
            [HidChannel::Interrupt, HidChannel::Control],
        ] {
            let protocol = protocol();
            let (mut transport, control) = open_transport();
            let mut sender = ReportSender::<Pro>::new();
            let mut observed = ObservedSubcommands::default();
            let (stale_session, session_id) = two_session_ids(&mut sender, &mut observed);
            let mut handshake = Handshake::new(session_id);
            let now = Duration::from_millis(100);

            handshake.observe_link(session_id);
            handshake.observe_channel(stale_session, second);
            handshake.observe_channel(session_id, first);
            assert!(matches!(
                handshake
                    .step(
                        session_id,
                        now,
                        &observed,
                        &protocol,
                        &mut sender,
                        &mut transport,
                    )
                    .expect("stale channel does not complete topology"),
                HandshakeProgress::WaitingForTopology
            ));
            assert!(control.accepted_interrupts().is_empty());

            handshake.observe_channel(session_id, second);
            assert_bootstrap_accepted(
                handshake
                    .step(
                        session_id,
                        now,
                        &observed,
                        &protocol,
                        &mut sender,
                        &mut transport,
                    )
                    .expect("either current-session channel order is accepted"),
                0,
            );
            assert!(matches!(
                handshake
                    .step(
                        stale_session,
                        Duration::from_millis(1_100),
                        &observed,
                        &protocol,
                        &mut sender,
                        &mut transport,
                    )
                    .expect("stale session step is ignored"),
                HandshakeProgress::StaleSession
            ));
            assert_eq!(neutral_attempts(&control).len(), 1);
        }
    }

    #[test]
    fn early_subcommand_still_requires_an_accepted_bootstrap_after_topology() {
        let protocol = protocol();
        let (mut transport, control) = open_transport();
        control.script_sends([ScriptedSendOutcome::Rejected, ScriptedSendOutcome::Accepted]);
        let mut sender = ReportSender::<Pro>::new();
        let mut observed = ObservedSubcommands::default();
        let (mut handshake, session_id) = new_handshake(&mut sender, &mut observed);

        handshake.observe_channel(session_id, HidChannel::Control);
        assert!(observed.observe(0x08));
        assert!(matches!(
            handshake
                .step(
                    session_id,
                    Duration::ZERO,
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("early subcommand still waits for topology"),
            HandshakeProgress::WaitingForTopology
        ));
        assert!(transport.attempts.is_empty());
        assert_eq!(handshake.completion(), None);

        handshake.observe_link(session_id);
        handshake.observe_channel(session_id, HidChannel::Interrupt);
        let progress = handshake
            .step(
                session_id,
                Duration::from_millis(100),
                &observed,
                &protocol,
                &mut sender,
                &mut transport,
            )
            .expect("topology completion attempts bootstrap");
        let HandshakeProgress::BootstrapAttempted {
            result: Err(error),
            skipped: 0,
        } = progress
        else {
            panic!("rejected bootstrap must remain active");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(sender.timer(), 0);
        assert_eq!(handshake.completion(), None);
        assert_eq!(
            handshake.next_deadline(),
            Some(Duration::from_millis(1_100))
        );

        assert_bootstrap_accepted(
            handshake
                .step(
                    session_id,
                    Duration::from_millis(1_100),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("accepted retry completes the early observation"),
            0,
        );
        assert_eq!(sender.timer(), 1);
        assert_eq!(
            handshake
                .completion()
                .expect("accepted bootstrap and observation complete handshake")
                .session_id(),
            session_id
        );
        assert_eq!(handshake.next_deadline(), None);
        assert_eq!(
            transport
                .attempts
                .iter()
                .map(|report| (report[0], report[1]))
                .collect::<Vec<_>>(),
            [(0x30, 0), (0x30, 0)]
        );
        assert_eq!(neutral_attempts(&control), [(0x30, 0, [0, 0, 0])]);
    }

    #[test]
    fn rust_spec_delta_bootstrap_retry_uses_absolute_start_phase() {
        let python = runtime_baseline_checkpoint(
            "handshake.retry_after_send_latency",
            "python_relative_retry",
        );
        assert_eq!(
            python["second_start_minus_first_start_ns"],
            1_250_000_000_u64
        );
        assert_eq!(
            python["second_start_minus_first_completion_ns"],
            1_000_000_000_u64
        );

        let protocol = protocol();
        let clock = FakeClock::at(Duration::from_millis(100));
        let (mut transport, control) =
            open_transport_with_latency(clock.clone(), Duration::from_millis(250));
        let mut sender = ReportSender::<Pro>::new();
        let mut observed = ObservedSubcommands::default();
        let (mut handshake, session_id) = ready_handshake(&mut sender, &mut observed);
        let first_start = clock.now();

        assert_bootstrap_accepted(
            handshake
                .step(
                    session_id,
                    clock.now(),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("initial bootstrap"),
            0,
        );
        let first_completion = clock.now();
        assert_eq!(clock.now(), Duration::from_millis(350));
        assert_eq!(
            handshake.next_deadline(),
            Some(Duration::from_millis(1_100))
        );
        let retry_start = handshake.next_deadline().expect("absolute retry deadline");
        assert_eq!(retry_start - first_start, Duration::from_secs(1));
        assert_eq!(
            retry_start - first_completion,
            Duration::from_millis(750),
            "Rust does not restart the retry interval after send completion"
        );
        assert_waiting_until(
            handshake
                .step(
                    session_id,
                    clock.now(),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("send latency does not move the phase"),
            Duration::from_millis(1_100),
        );

        clock.set(Duration::from_millis(1_100));
        assert_bootstrap_accepted(
            handshake
                .step(
                    session_id,
                    clock.now(),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("first absolute retry"),
            0,
        );
        assert_eq!(clock.now(), Duration::from_millis(1_350));
        assert_eq!(
            handshake.next_deadline(),
            Some(Duration::from_millis(2_100))
        );

        clock.set(Duration::from_millis(3_400));
        assert_bootstrap_accepted(
            handshake
                .step(
                    session_id,
                    clock.now(),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("overrun emits one retry"),
            1,
        );
        assert_eq!(clock.now(), Duration::from_millis(3_650));
        assert_waiting_until(
            handshake
                .step(
                    session_id,
                    clock.now(),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("send completion instant does not burst"),
            Duration::from_millis(4_100),
        );
        assert_eq!(neutral_attempts(&control).len(), 3);

        assert!(observed.observe(0x08));
        clock.set(Duration::from_millis(4_100));
        assert!(matches!(
            handshake
                .step(
                    session_id,
                    clock.now(),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("first subcommand stops bootstrap"),
            HandshakeProgress::SubcommandObserved
        ));
        assert!(handshake.bootstrap_stopped());
        assert_eq!(handshake.next_deadline(), None);
        assert_eq!(
            handshake
                .completion()
                .expect("observed subcommand completes handshake")
                .session_id(),
            session_id
        );
        assert_eq!(neutral_attempts(&control).len(), 3);
        assert_eq!(
            neutral_attempts(&control),
            [
                (0x30, 0, [0, 0, 0]),
                (0x30, 1, [0, 0, 0]),
                (0x30, 2, [0, 0, 0]),
            ]
        );
    }

    #[test]
    fn rejected_bootstrap_retries_on_the_same_absolute_phase() {
        let protocol = protocol();
        let (mut transport, control) = open_transport();
        control.script_sends([
            ScriptedSendOutcome::Rejected,
            ScriptedSendOutcome::Rejected,
            ScriptedSendOutcome::Accepted,
        ]);
        let mut sender = ReportSender::<Pro>::new();
        let mut observed = ObservedSubcommands::default();
        let (mut handshake, session_id) = ready_handshake(&mut sender, &mut observed);

        let progress = handshake
            .step(
                session_id,
                Duration::ZERO,
                &observed,
                &protocol,
                &mut sender,
                &mut transport,
            )
            .expect("initial bootstrap schedule");
        let HandshakeProgress::BootstrapAttempted {
            result: Err(error),
            skipped: 0,
        } = progress
        else {
            panic!("bootstrap rejection must be a non-terminal attempt");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(sender.timer(), 0);
        assert_eq!(handshake.next_deadline(), Some(Duration::from_secs(1)));

        assert_waiting_until(
            handshake
                .step(
                    session_id,
                    Duration::from_millis(999),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("pre-deadline check"),
            Duration::from_secs(1),
        );
        let progress = handshake
            .step(
                session_id,
                Duration::from_secs(1),
                &observed,
                &protocol,
                &mut sender,
                &mut transport,
            )
            .expect("absolute retry schedule");
        let HandshakeProgress::BootstrapAttempted {
            result: Err(error),
            skipped: 0,
        } = progress
        else {
            panic!("retry rejection must remain a non-terminal attempt");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(sender.timer(), 0);
        assert_eq!(handshake.next_deadline(), Some(Duration::from_secs(2)));

        assert_bootstrap_accepted(
            handshake
                .step(
                    session_id,
                    Duration::from_secs(2),
                    &observed,
                    &protocol,
                    &mut sender,
                    &mut transport,
                )
                .expect("second absolute retry is accepted"),
            0,
        );
        assert_eq!(sender.timer(), 1);
        assert_eq!(handshake.next_deadline(), Some(Duration::from_secs(3)));
        assert_eq!(
            transport
                .attempts
                .iter()
                .map(|report| (report[0], report[1], [report[3], report[4], report[5]]))
                .collect::<Vec<_>>(),
            [
                (0x30, 0, [0, 0, 0]),
                (0x30, 0, [0, 0, 0]),
                (0x30, 0, [0, 0, 0]),
            ]
        );
    }

    fn new_handshake(
        sender: &mut ReportSender<Pro>,
        observed: &mut ObservedSubcommands,
    ) -> (Handshake, ConnectionSessionId) {
        let mut sessions = ConnectionSessions::new();
        let mut state = InputStateStore::new();
        let session_id = sessions.begin_direct(sender, observed, &mut state);
        (Handshake::new(session_id), session_id)
    }

    fn ready_handshake(
        sender: &mut ReportSender<Pro>,
        observed: &mut ObservedSubcommands,
    ) -> (Handshake, ConnectionSessionId) {
        let (mut handshake, session_id) = new_handshake(sender, observed);
        handshake.observe_link(session_id);
        handshake.observe_channel(session_id, HidChannel::Control);
        handshake.observe_channel(session_id, HidChannel::Interrupt);
        (handshake, session_id)
    }

    fn two_session_ids(
        sender: &mut ReportSender<Pro>,
        observed: &mut ObservedSubcommands,
    ) -> (ConnectionSessionId, ConnectionSessionId) {
        let mut sessions = ConnectionSessions::new();
        let mut state = InputStateStore::new();
        let first = sessions.begin_direct(sender, observed, &mut state);
        assert!(sessions.end_current(first));
        let second = sessions.begin_direct(sender, observed, &mut state);
        (first, second)
    }

    fn protocol() -> SwitchHidProtocol<Pro> {
        SwitchHidProtocol::new(None, DEVICE_INFO_ADDRESS)
    }

    fn open_transport() -> (RecordingTransport, FakeTransportControl) {
        open_recording_transport(None)
    }

    fn open_transport_with_latency(
        clock: FakeClock,
        send_latency: Duration,
    ) -> (RecordingTransport, FakeTransportControl) {
        open_recording_transport(Some((clock, send_latency)))
    }

    fn open_recording_transport(
        latency: Option<(FakeClock, Duration)>,
    ) -> (RecordingTransport, FakeTransportControl) {
        let (inner, control) = FakeTransport::with_limits(8, 8);
        let mut transport = RecordingTransport {
            inner,
            attempts: Vec::new(),
            latency,
        };
        let (notifier, _wake_receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        (transport, control)
    }

    fn neutral_attempts(control: &FakeTransportControl) -> Vec<(u8, u8, [u8; 3])> {
        control
            .accepted_interrupts()
            .iter()
            .map(|report| (report[0], report[1], [report[3], report[4], report[5]]))
            .collect()
    }

    struct RecordingTransport {
        inner: FakeTransport,
        attempts: Vec<Box<[u8]>>,
        latency: Option<(FakeClock, Duration)>,
    }

    impl TransportPort for RecordingTransport {
        fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
            self.inner.open(activity)
        }

        fn start_pairing(&mut self) -> TransportResult<()> {
            self.inner.start_pairing()
        }

        fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            self.inner.poll(timeout)
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
            self.attempts.push(Box::from(payload));
            let result = self.inner.send_interrupt(payload);
            if let Some((clock, send_latency)) = &self.latency {
                clock.advance(*send_latency);
            }
            result
        }

        fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
            self.inner.drain_interrupt(timeout)
        }

        fn disconnect(&mut self) -> TransportResult<()> {
            self.inner.disconnect()
        }

        fn close(&mut self) -> TransportResult<()> {
            self.inner.close()
        }
    }

    #[derive(Clone)]
    struct FakeClock {
        now: Arc<Mutex<Duration>>,
    }

    impl FakeClock {
        fn at(now: Duration) -> Self {
            Self {
                now: Arc::new(Mutex::new(now)),
            }
        }

        fn now(&self) -> Duration {
            *self.now.lock().expect("fake clock lock")
        }

        fn set(&self, next: Duration) {
            let mut now = self.now.lock().expect("fake clock lock");
            assert!(next >= *now, "fake monotonic clock cannot move backwards");
            *now = next;
        }

        fn advance(&self, elapsed: Duration) {
            let next = self.now().checked_add(elapsed).expect("fake clock range");
            self.set(next);
        }
    }

    fn assert_bootstrap_accepted(progress: HandshakeProgress, expected_skipped: u64) {
        let HandshakeProgress::BootstrapAttempted {
            result: Ok(_),
            skipped,
        } = progress
        else {
            panic!("bootstrap must be accepted");
        };
        assert_eq!(skipped, expected_skipped);
    }

    fn assert_waiting_until(progress: HandshakeProgress, expected_deadline: Duration) {
        let HandshakeProgress::WaitingUntil { deadline } = progress else {
            panic!("handshake must wait for its next deadline");
        };
        assert_eq!(deadline, expected_deadline);
    }
}
