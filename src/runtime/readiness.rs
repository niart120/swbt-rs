#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T19 defines readiness coordination before T21 worker integration"
    )
)]

use std::{error::Error as StdError, fmt, time::Duration};

use crate::{
    model::ControllerModel,
    runtime::{
        handshake::{Handshake, HandshakeCompletion},
        periodic::{PeriodicPolicy, PeriodicStart},
        scheduler::SchedulerError,
        sender::ReportSender,
        session::{ConnectionSessionId, ConnectionSessions},
    },
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ReadySession {
    session_id: ConnectionSessionId,
}

impl ReadySession {
    #[must_use]
    pub(crate) const fn session_id(&self) -> ConnectionSessionId {
        self.session_id
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadinessWait {
    Handshake,
    Protocol,
    PeriodicHoldoff { until: Duration },
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ReadinessProgress {
    Pending(ReadinessWait),
    Ready(ReadySession),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadinessError {
    StaleSession {
        expected: ConnectionSessionId,
        current: Option<ConnectionSessionId>,
    },
    HandshakeSessionMismatch {
        expected: ConnectionSessionId,
        actual: ConnectionSessionId,
    },
    Disconnected {
        reason: Option<u8>,
    },
    TimedOut,
    Scheduler(SchedulerError),
}

impl From<SchedulerError> for ReadinessError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(error)
    }
}

impl fmt::Display for ReadinessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleSession {
                expected,
                current: Some(current),
            } => write!(
                formatter,
                "readiness requires session {}, but current session is {}",
                expected.get(),
                current.get(),
            ),
            Self::StaleSession {
                expected,
                current: None,
            } => write!(
                formatter,
                "readiness requires session {}, but no current session exists",
                expected.get(),
            ),
            Self::HandshakeSessionMismatch { expected, actual } => write!(
                formatter,
                "readiness requires handshake session {}, but completion belongs to session {}",
                expected.get(),
                actual.get(),
            ),
            Self::Disconnected {
                reason: Some(reason),
            } => {
                write!(
                    formatter,
                    "connection ended before readiness (reason 0x{reason:02x})"
                )
            }
            Self::Disconnected { reason: None } => {
                formatter.write_str("connection ended before readiness")
            }
            Self::TimedOut => formatter.write_str("readiness operation timed out"),
            Self::Scheduler(error) => write!(formatter, "readiness scheduler failed: {error}"),
        }
    }
}

impl StdError for ReadinessError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Scheduler(error) => Some(error),
            Self::StaleSession { .. }
            | Self::HandshakeSessionMismatch { .. }
            | Self::Disconnected { .. }
            | Self::TimedOut => None,
        }
    }
}

pub(crate) struct ReadinessGate {
    session_id: ConnectionSessionId,
    operation_deadline: Duration,
    handshake: Option<HandshakeCompletion>,
}

impl ReadinessGate {
    pub(crate) const fn new(session_id: ConnectionSessionId, operation_deadline: Duration) -> Self {
        Self {
            session_id,
            operation_deadline,
            handshake: None,
        }
    }

    #[must_use]
    pub(crate) const fn operation_deadline(&self) -> Duration {
        self.operation_deadline
    }

    pub(crate) fn evaluate_direct<M: ControllerModel>(
        &mut self,
        now: Duration,
        sessions: &ConnectionSessions,
        handshake: &mut Option<Handshake>,
        sender: &ReportSender<M>,
    ) -> Result<ReadinessProgress, ReadinessError> {
        self.ensure_current(sessions)?;
        self.ensure_before_timeout(now)?;
        if !self.collect_handshake(handshake)? {
            return Ok(ReadinessProgress::Pending(ReadinessWait::Handshake));
        }
        if !sender.session().protocol_ready() {
            return Ok(ReadinessProgress::Pending(ReadinessWait::Protocol));
        }
        Ok(ReadinessProgress::Ready(self.take_ready_session()))
    }

    pub(crate) fn evaluate_periodic<M: ControllerModel>(
        &mut self,
        now: Duration,
        sessions: &ConnectionSessions,
        handshake: &mut Option<Handshake>,
        sender: &ReportSender<M>,
        periodic: &mut PeriodicPolicy,
    ) -> Result<ReadinessProgress, ReadinessError> {
        self.ensure_current(sessions)?;
        self.ensure_before_timeout(now)?;
        if !self.collect_handshake(handshake)? {
            return Ok(ReadinessProgress::Pending(ReadinessWait::Handshake));
        }
        if !sender.session().protocol_ready() {
            return Ok(ReadinessProgress::Pending(ReadinessWait::Protocol));
        }

        match periodic.start_when_unheld(now)? {
            PeriodicStart::HeldOff { until } => {
                Ok(ReadinessProgress::Pending(ReadinessWait::PeriodicHoldoff {
                    until,
                }))
            }
            PeriodicStart::Started { .. } => {
                Ok(ReadinessProgress::Ready(self.take_ready_session()))
            }
        }
    }

    pub(crate) fn abort(
        self,
        handshake: &mut Option<Handshake>,
        error: ReadinessError,
    ) -> ReadinessError {
        handshake.take();
        error
    }

    fn ensure_current(&self, sessions: &ConnectionSessions) -> Result<(), ReadinessError> {
        let current = sessions.current();
        if current == Some(self.session_id) {
            return Ok(());
        }
        Err(ReadinessError::StaleSession {
            expected: self.session_id,
            current,
        })
    }

    fn ensure_before_timeout(&self, now: Duration) -> Result<(), ReadinessError> {
        if now < self.operation_deadline {
            Ok(())
        } else {
            Err(ReadinessError::TimedOut)
        }
    }

    fn collect_handshake(
        &mut self,
        handshake: &mut Option<Handshake>,
    ) -> Result<bool, ReadinessError> {
        if self.handshake.is_some() {
            return Ok(true);
        }
        let Some(completion) = handshake.as_ref().and_then(Handshake::completion) else {
            return Ok(false);
        };
        if completion.session_id() != self.session_id {
            return Err(ReadinessError::HandshakeSessionMismatch {
                expected: self.session_id,
                actual: completion.session_id(),
            });
        }
        self.handshake = Some(completion);
        handshake.take();
        Ok(true)
    }

    fn take_ready_session(&mut self) -> ReadySession {
        let completion = self
            .handshake
            .take()
            .expect("readiness is emitted only after handshake collection");
        ReadySession {
            session_id: completion.session_id(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        model::Pro,
        protocol::{DeviceInfoBluetoothAddress, SwitchHidProtocol},
        runtime::{
            connection::ObservedSubcommands,
            handshake::{Handshake, HandshakeProgress},
            lifecycle::{LifecycleAction, LifecycleState, LifecycleStateMachine},
            output::{OutputHandling, OutputHandlingContext, OutputHandlingError, handle_output},
            periodic::{AutomaticInput, PeriodicPolicy},
            readiness::{ReadinessError, ReadinessGate, ReadinessProgress, ReadinessWait},
            sender::ReportSender,
            session::{ConnectionSessionId, ConnectionSessions},
            state::InputStateStore,
            transport::{
                HidChannel, TransportErrorKind, TransportEvent, TransportPort, activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
    const REPORT_PERIOD: Duration = Duration::from_millis(8);
    const OPERATION_DEADLINE: Duration = Duration::from_secs(1);

    #[test]
    fn periodic_ready_starts_after_accepted_mode_nonzero_lights_and_last_reply_holdoff() {
        let mut harness = Harness::new();
        let mut periodic = PeriodicPolicy::new(REPORT_PERIOD).expect("valid period");
        let session_id = harness.begin_periodic(&mut periodic);
        let mut lifecycle = connecting_lifecycle();
        let mut gate = ReadinessGate::new(session_id, OPERATION_DEADLINE);
        let mut handshake = Some(Handshake::new(session_id));
        harness.control.script_sends([
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Rejected,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
        ]);

        harness.bootstrap(
            handshake.as_mut().expect("active handshake"),
            session_id,
            Duration::from_millis(100),
        );
        assert_eq!(harness.sender.timer(), 1);

        let rejected = harness.handle_subcommand(Duration::from_millis(110), 0x03, &[0x30]);
        periodic
            .record_output_completion(Duration::from_millis(110), &rejected)
            .expect("rejection does not create a holdoff");
        let OutputHandlingError::Transport(error) =
            rejected.expect_err("first report-mode reply is rejected")
        else {
            panic!("scripted rejection must remain a transport error");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(harness.sender.timer(), 1);
        harness.finish_handshake(
            handshake.as_mut().expect("active handshake"),
            session_id,
            Duration::from_millis(110),
        );
        assert_eq!(
            gate.evaluate_periodic(
                Duration::from_millis(110),
                &harness.sessions,
                &mut handshake,
                &harness.sender,
                &mut periodic,
            )
            .expect("rejected reply remains retryable"),
            ReadinessProgress::Pending(ReadinessWait::Protocol)
        );
        assert!(handshake.is_none(), "completed handshake is collected");
        assert_eq!(lifecycle.state(), LifecycleState::Connecting);
        assert_eq!(periodic.next_deadline(), None);

        let accepted = harness.handle_subcommand(Duration::from_millis(120), 0x03, &[0x30]);
        assert!(matches!(accepted, Ok(OutputHandling::ReplyAccepted(_))));
        periodic
            .record_output_completion(Duration::from_millis(120), &accepted)
            .expect("report-mode holdoff");
        assert_eq!(harness.sender.timer(), 2);
        assert_eq!(
            gate.evaluate_periodic(
                Duration::from_millis(120),
                &harness.sessions,
                &mut handshake,
                &harness.sender,
                &mut periodic,
            )
            .expect("report mode alone is not ready"),
            ReadinessProgress::Pending(ReadinessWait::Protocol)
        );

        let zero_lights = harness.handle_subcommand(Duration::from_millis(130), 0x30, &[0x00]);
        assert!(matches!(zero_lights, Ok(OutputHandling::ReplyAccepted(_))));
        periodic
            .record_output_completion(Duration::from_millis(130), &zero_lights)
            .expect("zero-lights holdoff");
        assert_eq!(harness.sender.timer(), 3);
        assert_eq!(harness.sender.session().player_lights(), Some(0));
        assert_eq!(
            gate.evaluate_periodic(
                Duration::from_millis(130),
                &harness.sessions,
                &mut handshake,
                &harness.sender,
                &mut periodic,
            )
            .expect("zero lights remain retryable"),
            ReadinessProgress::Pending(ReadinessWait::Protocol)
        );

        let nonzero_lights = harness.handle_subcommand(Duration::from_millis(140), 0x30, &[0x01]);
        assert!(matches!(
            nonzero_lights,
            Ok(OutputHandling::ReplyAccepted(_))
        ));
        periodic
            .record_output_completion(Duration::from_millis(140), &nonzero_lights)
            .expect("nonzero-lights holdoff");
        assert!(harness.sender.session().protocol_ready());
        assert_eq!(harness.sender.timer(), 4);
        assert_eq!(
            gate.evaluate_periodic(
                Duration::from_millis(140),
                &harness.sessions,
                &mut handshake,
                &harness.sender,
                &mut periodic,
            )
            .expect("accepted state waits for the latest reply holdoff"),
            ReadinessProgress::Pending(ReadinessWait::PeriodicHoldoff {
                until: Duration::from_millis(440),
            })
        );
        assert_eq!(periodic.next_deadline(), None);
        assert_eq!(harness.control.accepted_interrupts().len(), 4);

        assert_eq!(
            gate.evaluate_periodic(
                Duration::from_millis(439),
                &harness.sessions,
                &mut handshake,
                &harness.sender,
                &mut periodic,
            )
            .expect("one millisecond remains"),
            ReadinessProgress::Pending(ReadinessWait::PeriodicHoldoff {
                until: Duration::from_millis(440),
            })
        );

        let ready = match gate
            .evaluate_periodic(
                Duration::from_millis(440),
                &harness.sessions,
                &mut handshake,
                &harness.sender,
                &mut periodic,
            )
            .expect("holdoff boundary starts reporting")
        {
            ReadinessProgress::Ready(ready) => ready,
            ReadinessProgress::Pending(wait) => panic!("unexpected pending state: {wait:?}"),
        };
        assert_eq!(ready.session_id(), session_id);
        assert!(lifecycle.mark_ready(ready));
        assert_eq!(lifecycle.state(), LifecycleState::Ready);
        assert_eq!(periodic.next_deadline(), Some(Duration::from_millis(448)));
        assert_eq!(
            harness
                .send_due(Duration::from_millis(447), &mut periodic)
                .expect("pre-deadline check"),
            AutomaticInput::NotDue
        );
        assert_eq!(harness.control.accepted_interrupts().len(), 4);

        assert!(matches!(
            harness
                .send_due(Duration::from_millis(448), &mut periodic)
                .expect("first periodic report"),
            AutomaticInput::Sent { skipped: 0, .. }
        ));
        assert_eq!(harness.sender.timer(), 5);
        assert_eq!(periodic.next_deadline(), Some(Duration::from_millis(456)));
        assert_eq!(
            accepted_report_keys(&harness.control),
            [
                (0x30, 0, None),
                (0x21, 1, Some(0x03)),
                (0x21, 2, Some(0x30)),
                (0x21, 3, Some(0x30)),
                (0x30, 4, None),
            ]
        );
    }

    #[test]
    fn direct_ready_requires_current_handshake_completion_and_sends_no_confirmation_input() {
        let mut harness = Harness::new();
        let first = harness.begin_direct();
        let stale_handshake = completed_handshake(first);
        assert!(harness.sessions.end_current(first));

        let current = harness.begin_direct();
        let mut lifecycle = connecting_lifecycle();
        let mut current_handshake = Some(Handshake::new(current));
        harness.control.script_sends([
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
        ]);
        harness.bootstrap(
            current_handshake.as_mut().expect("current handshake"),
            current,
            Duration::from_millis(100),
        );
        harness
            .handle_subcommand(Duration::from_millis(110), 0x03, &[0x30])
            .expect("report-mode reply accepted");
        harness
            .handle_subcommand(Duration::from_millis(120), 0x30, &[0x01])
            .expect("player-lights reply accepted");
        let mut current_gate = ReadinessGate::new(current, OPERATION_DEADLINE);
        assert_eq!(
            current_gate
                .evaluate_direct(
                    Duration::from_millis(120),
                    &harness.sessions,
                    &mut current_handshake,
                    &harness.sender,
                )
                .expect("protocol state cannot replace handshake completion"),
            ReadinessProgress::Pending(ReadinessWait::Handshake)
        );
        harness.finish_handshake(
            current_handshake.as_mut().expect("current handshake"),
            current,
            Duration::from_millis(120),
        );
        assert!(harness.sender.session().protocol_ready());

        let mut superseded_gate = ReadinessGate::new(first, OPERATION_DEADLINE);
        let mut no_handshake = None;
        let error = superseded_gate
            .evaluate_direct(
                Duration::from_millis(120),
                &harness.sessions,
                &mut no_handshake,
                &harness.sender,
            )
            .expect_err("an old gate cannot read the current sender session");
        assert!(matches!(
            error,
            ReadinessError::StaleSession {
                expected,
                current: Some(actual),
            } if expected == first && actual == current
        ));
        superseded_gate.abort(&mut no_handshake, error);

        let mut stale_gate = ReadinessGate::new(current, OPERATION_DEADLINE);
        let mut stale_handshake = Some(stale_handshake);
        let error = stale_gate
            .evaluate_direct(
                Duration::from_millis(120),
                &harness.sessions,
                &mut stale_handshake,
                &harness.sender,
            )
            .expect_err("old handshake proof cannot complete the current session");
        assert!(matches!(
            error,
            ReadinessError::HandshakeSessionMismatch {
                expected,
                actual,
            } if expected == current && actual == first
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "readiness requires handshake session {}, but completion belongs to session {}",
                current.get(),
                first.get(),
            )
        );
        let error = stale_gate.abort(&mut stale_handshake, error);
        assert!(matches!(
            error,
            ReadinessError::HandshakeSessionMismatch { .. }
        ));
        assert!(stale_handshake.is_none());
        assert_eq!(lifecycle.state(), LifecycleState::Connecting);

        let ready = match current_gate
            .evaluate_direct(
                Duration::from_millis(120),
                &harness.sessions,
                &mut current_handshake,
                &harness.sender,
            )
            .expect("same-session handshake completes Direct readiness")
        {
            ReadinessProgress::Ready(ready) => ready,
            ReadinessProgress::Pending(wait) => panic!("unexpected pending state: {wait:?}"),
        };
        assert_eq!(ready.session_id(), current);
        assert!(current_handshake.is_none());
        assert!(lifecycle.mark_ready(ready));
        assert_eq!(lifecycle.state(), LifecycleState::Ready);
        assert_eq!(harness.sender.timer(), 3);
        assert_eq!(
            accepted_report_keys(&harness.control),
            [
                (0x30, 0, None),
                (0x21, 1, Some(0x03)),
                (0x21, 2, Some(0x30)),
            ]
        );
    }

    #[test]
    fn disconnect_ends_pending_periodic_readiness_without_a_scheduler() {
        let mut harness = Harness::new();
        let mut periodic = PeriodicPolicy::new(REPORT_PERIOD).expect("valid period");
        let session_id = harness.begin_periodic(&mut periodic);
        let mut lifecycle = connecting_lifecycle();
        let mut gate = ReadinessGate::new(session_id, OPERATION_DEADLINE);
        let mut handshake = Some(Handshake::new(session_id));
        harness
            .control
            .script_sends([ScriptedSendOutcome::AcceptedThenDisconnect { reason: Some(0x13) }]);
        harness.bootstrap(
            handshake.as_mut().expect("active handshake"),
            session_id,
            Duration::from_millis(100),
        );
        assert_eq!(
            gate.evaluate_periodic(
                Duration::from_millis(100),
                &harness.sessions,
                &mut handshake,
                &harness.sender,
                &mut periodic,
            )
            .expect("bootstrap alone does not complete handshake"),
            ReadinessProgress::Pending(ReadinessWait::Handshake)
        );
        assert!(handshake.is_some());
        assert_eq!(periodic.next_deadline(), None);

        let events = harness
            .transport
            .poll(Duration::ZERO)
            .expect("accepted bootstrap queues disconnect");
        assert_eq!(events.len(), 1);
        let Some(TransportEvent::Disconnected { reason }) = events.into_iter().next() else {
            panic!("expected a disconnect");
        };

        let error = gate.abort(&mut handshake, ReadinessError::Disconnected { reason });
        periodic.stop_session();
        assert!(harness.sessions.end_current(session_id));
        assert!(lifecycle.mark_connection_ended());

        assert!(matches!(
            error,
            ReadinessError::Disconnected { reason: Some(0x13) }
        ));
        assert!(handshake.is_none());
        assert_eq!(periodic.next_deadline(), None);
        assert_eq!(harness.sessions.current(), None);
        assert_eq!(lifecycle.state(), LifecycleState::Open);
        assert_eq!(harness.sender.timer(), 1);
        assert_eq!(harness.control.accepted_interrupts().len(), 1);
    }

    #[test]
    fn timeout_ends_pending_periodic_readiness_without_a_scheduler() {
        let mut pending = PendingPeriodic::new();

        assert_eq!(
            pending.gate.operation_deadline(),
            Duration::from_millis(400)
        );
        assert_eq!(
            pending
                .gate
                .evaluate_periodic(
                    Duration::from_millis(399),
                    &pending.harness.sessions,
                    &mut pending.handshake,
                    &pending.harness.sender,
                    &mut pending.periodic,
                )
                .expect("operation remains pending before its external deadline"),
            ReadinessProgress::Pending(ReadinessWait::PeriodicHoldoff {
                until: Duration::from_millis(420),
            })
        );

        let error = pending
            .gate
            .evaluate_periodic(
                Duration::from_millis(400),
                &pending.harness.sessions,
                &mut pending.handshake,
                &pending.harness.sender,
                &mut pending.periodic,
            )
            .expect_err("the operation deadline is terminal before the holdoff");
        assert!(matches!(error, ReadinessError::TimedOut));
        let error = pending.gate.abort(&mut pending.handshake, error);
        pending.periodic.stop_session();
        assert!(pending.harness.sessions.end_current(pending.session_id));
        assert!(pending.lifecycle.mark_connection_ended());

        assert!(matches!(error, ReadinessError::TimedOut));
        assert!(pending.handshake.is_none());
        assert_eq!(pending.periodic.next_deadline(), None);
        assert_eq!(pending.harness.sessions.current(), None);
        assert_eq!(pending.lifecycle.state(), LifecycleState::Open);
        assert_eq!(pending.harness.sender.timer(), 3);
        assert_eq!(pending.harness.control.accepted_interrupts().len(), 3);
    }

    struct PendingPeriodic {
        harness: Harness,
        periodic: PeriodicPolicy,
        lifecycle: LifecycleStateMachine,
        gate: ReadinessGate,
        handshake: Option<Handshake>,
        session_id: ConnectionSessionId,
    }

    impl PendingPeriodic {
        fn new() -> Self {
            let mut harness = Harness::new();
            let mut periodic = PeriodicPolicy::new(REPORT_PERIOD).expect("valid period");
            let session_id = harness.begin_periodic(&mut periodic);
            let lifecycle = connecting_lifecycle();
            let mut gate = ReadinessGate::new(session_id, Duration::from_millis(400));
            let mut handshake = Some(Handshake::new(session_id));
            harness.control.script_sends([
                ScriptedSendOutcome::Accepted,
                ScriptedSendOutcome::Accepted,
                ScriptedSendOutcome::Accepted,
            ]);

            harness.bootstrap(
                handshake.as_mut().expect("active handshake"),
                session_id,
                Duration::from_millis(100),
            );
            let report_mode = harness.handle_subcommand(Duration::from_millis(110), 0x03, &[0x30]);
            periodic
                .record_output_completion(Duration::from_millis(110), &report_mode)
                .expect("report-mode holdoff");
            report_mode.expect("report-mode reply accepted");
            harness.finish_handshake(
                handshake.as_mut().expect("active handshake"),
                session_id,
                Duration::from_millis(110),
            );
            let lights = harness.handle_subcommand(Duration::from_millis(120), 0x30, &[0x01]);
            periodic
                .record_output_completion(Duration::from_millis(120), &lights)
                .expect("player-lights holdoff");
            lights.expect("player-lights reply accepted");

            assert_eq!(
                gate.evaluate_periodic(
                    Duration::from_millis(120),
                    &harness.sessions,
                    &mut handshake,
                    &harness.sender,
                    &mut periodic,
                )
                .expect("protocol ready but held off"),
                ReadinessProgress::Pending(ReadinessWait::PeriodicHoldoff {
                    until: Duration::from_millis(420),
                })
            );
            assert!(handshake.is_none());
            assert_eq!(periodic.next_deadline(), None);

            Self {
                harness,
                periodic,
                lifecycle,
                gate,
                handshake,
                session_id,
            }
        }
    }

    struct Harness {
        protocol: SwitchHidProtocol<Pro>,
        sender: ReportSender<Pro>,
        store: InputStateStore<Pro>,
        observed: ObservedSubcommands,
        sessions: ConnectionSessions,
        transport: FakeTransport,
        control: FakeTransportControl,
    }

    impl Harness {
        fn new() -> Self {
            let (mut transport, control) = FakeTransport::with_limits(8, 8);
            let (notifier, _wake_receiver) = activity_channel();
            transport.open(notifier).expect("open fake transport");
            Self {
                protocol: protocol(),
                sender: ReportSender::new(),
                store: InputStateStore::new(),
                observed: ObservedSubcommands::default(),
                sessions: ConnectionSessions::new(),
                transport,
                control,
            }
        }

        fn begin_periodic(&mut self, periodic: &mut PeriodicPolicy) -> ConnectionSessionId {
            self.sessions
                .begin_periodic(
                    &mut self.sender,
                    periodic,
                    &mut self.observed,
                    &mut self.store,
                )
                .expect("periodic session")
        }

        fn begin_direct(&mut self) -> ConnectionSessionId {
            self.sessions
                .begin_direct(&mut self.sender, &mut self.observed, &mut self.store)
                .expect("direct session")
        }

        fn bootstrap(
            &mut self,
            handshake: &mut Handshake,
            session_id: ConnectionSessionId,
            now: Duration,
        ) {
            handshake.observe_link(session_id);
            handshake.observe_channel(session_id, HidChannel::Control);
            handshake.observe_channel(session_id, HidChannel::Interrupt);
            let HandshakeProgress::BootstrapAttempted {
                result: Ok(_),
                skipped: 0,
            } = handshake
                .step(
                    session_id,
                    now,
                    &self.observed,
                    &self.protocol,
                    &mut self.sender,
                    &mut self.transport,
                )
                .expect("bootstrap step")
            else {
                panic!("topology completion must send bootstrap");
            };
        }

        fn finish_handshake(
            &mut self,
            handshake: &mut Handshake,
            session_id: ConnectionSessionId,
            now: Duration,
        ) {
            assert!(matches!(
                handshake
                    .step(
                        session_id,
                        now,
                        &self.observed,
                        &self.protocol,
                        &mut self.sender,
                        &mut self.transport,
                    )
                    .expect("handshake completion"),
                HandshakeProgress::SubcommandObserved
            ));
        }

        fn handle_subcommand(
            &mut self,
            _completed_at: Duration,
            subcommand_id: u8,
            payload: &[u8],
        ) -> Result<OutputHandling, OutputHandlingError> {
            let raw = subcommand_report(subcommand_id, payload);
            let current = self.store.snapshot();
            let mut ignore_output = |_| {};
            handle_output(
                HidChannel::Interrupt,
                &raw,
                OutputHandlingContext {
                    observe_output: &mut ignore_output,
                    protocol: &self.protocol,
                    current: &current,
                    observed: &mut self.observed,
                    sender: &mut self.sender,
                    transport: &mut self.transport,
                },
            )
        }

        fn send_due(
            &mut self,
            now: Duration,
            periodic: &mut PeriodicPolicy,
        ) -> crate::runtime::periodic::PeriodicResult {
            periodic.send_due(
                now,
                &self.store,
                &self.protocol,
                &mut self.sender,
                &mut self.transport,
            )
        }
    }

    fn connecting_lifecycle() -> LifecycleStateMachine {
        let mut lifecycle = LifecycleStateMachine::new();
        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::OpenTransport));
        assert_eq!(lifecycle.complete_open(), LifecycleAction::Opened);
        assert!(lifecycle.begin_connection());
        assert_eq!(lifecycle.state(), LifecycleState::Connecting);
        lifecycle
    }

    fn completed_handshake(session_id: ConnectionSessionId) -> Handshake {
        let mut harness = Harness::new();
        let mut handshake = Handshake::new(session_id);
        harness.bootstrap(&mut handshake, session_id, Duration::ZERO);
        assert!(harness.observed.observe(0x08));
        harness.finish_handshake(&mut handshake, session_id, Duration::ZERO);
        handshake
    }

    fn accepted_report_keys(control: &FakeTransportControl) -> Vec<(u8, u8, Option<u8>)> {
        control
            .accepted_interrupts()
            .iter()
            .map(|report| {
                (
                    report[0],
                    report[1],
                    (report[0] == 0x21).then_some(report[14]),
                )
            })
            .collect()
    }

    fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut raw = vec![0x01, 0];
        raw.extend_from_slice(&NEUTRAL_RUMBLE);
        raw.push(subcommand_id);
        raw.extend_from_slice(payload);
        raw
    }

    fn protocol() -> SwitchHidProtocol<Pro> {
        SwitchHidProtocol::new(
            None,
            DeviceInfoBluetoothAddress::from_wire_bytes(DEVICE_INFO_ADDRESS),
        )
    }
}
