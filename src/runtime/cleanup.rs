#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T20 defines explicit cleanup before T21 worker integration"
    )
)]

use std::time::Duration;

use crate::{
    input::InputState,
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    runtime::{
        lifecycle::{LifecycleAction, LifecycleStateMachine},
        sender::ReportSender,
        transport::{TransportError, TransportPort, TransportResult},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseMode {
    WithNeutral,
    WithoutNeutral,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CleanupPhase {
    Neutral,
    DrainInterrupt,
    Disconnect,
    TransportClose,
}

#[derive(Debug)]
pub(crate) struct CleanupFailure {
    phase: CleanupPhase,
    error: TransportError,
}

impl CleanupFailure {
    #[must_use]
    pub(crate) const fn phase(&self) -> CleanupPhase {
        self.phase
    }

    #[must_use]
    pub(crate) const fn source_error(&self) -> &TransportError {
        &self.error
    }
}

#[derive(Debug)]
pub(crate) enum ExplicitCloseError<J> {
    Cleanup(CleanupFailure),
    Join(J),
    CleanupAndJoin { cleanup: CleanupFailure, join: J },
}

pub(crate) struct CleanupContext<'a, M: ControllerModel> {
    pub(crate) connected: bool,
    pub(crate) now_ns: u64,
    pub(crate) lifecycle: &'a mut LifecycleStateMachine,
    pub(crate) protocol: &'a SwitchHidProtocol<M>,
    pub(crate) sender: &'a mut ReportSender<M>,
    pub(crate) transport: &'a mut dyn TransportPort,
}

pub(crate) struct CleanupSequence {
    mode: CloseMode,
    drain_timeout: Duration,
}

impl CleanupSequence {
    pub(crate) const fn new(mode: CloseMode, drain_timeout: Duration) -> Self {
        Self {
            mode,
            drain_timeout,
        }
    }

    pub(crate) fn run<M: ControllerModel>(self, context: CleanupContext<'_, M>) -> CloseCompletion {
        let CleanupContext {
            connected,
            now_ns,
            lifecycle,
            protocol,
            sender,
            transport,
        } = context;

        if lifecycle.request_close() != LifecycleAction::BeginCleanup {
            return CloseCompletion::not_performed();
        }

        let mut first_failure = None;
        if self.mode == CloseMode::WithNeutral && connected {
            record_first_failure(
                &mut first_failure,
                CleanupPhase::Neutral,
                sender
                    .send_input(protocol, &InputState::neutral(), now_ns, transport)
                    .map(|_| ()),
            );
        }
        record_first_failure(
            &mut first_failure,
            CleanupPhase::DrainInterrupt,
            transport.drain_interrupt(self.drain_timeout),
        );
        record_first_failure(
            &mut first_failure,
            CleanupPhase::Disconnect,
            transport.disconnect(),
        );
        record_first_failure(
            &mut first_failure,
            CleanupPhase::TransportClose,
            transport.close(),
        );

        let completed = lifecycle.complete_close();
        debug_assert_eq!(completed, LifecycleAction::Closed);
        CloseCompletion {
            performed: true,
            first_failure,
        }
    }
}

pub(crate) struct CloseCompletion {
    performed: bool,
    first_failure: Option<CleanupFailure>,
}

impl CloseCompletion {
    const fn not_performed() -> Self {
        Self {
            performed: false,
            first_failure: None,
        }
    }

    #[must_use]
    pub(crate) const fn performed(&self) -> bool {
        self.performed
    }

    pub(crate) fn finish_with_join<J>(
        self,
        join: impl FnOnce() -> Result<(), J>,
    ) -> Result<(), ExplicitCloseError<J>> {
        if !self.performed {
            return Ok(());
        }

        match (self.first_failure, join()) {
            (None, Ok(())) => Ok(()),
            (None, Err(join)) => Err(ExplicitCloseError::Join(join)),
            (Some(cleanup), Ok(())) => Err(ExplicitCloseError::Cleanup(cleanup)),
            (Some(cleanup), Err(join)) => Err(ExplicitCloseError::CleanupAndJoin { cleanup, join }),
        }
    }
}

fn record_first_failure(
    first_failure: &mut Option<CleanupFailure>,
    phase: CleanupPhase,
    result: TransportResult<()>,
) {
    if first_failure.is_some() {
        return;
    }
    if let Err(error) = result {
        *first_failure = Some(CleanupFailure { phase, error });
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        input::InputState,
        model::Pro,
        protocol::{
            DeviceInfoBluetoothAddress, OutputReport, SwitchHidProtocol, parse_output_report,
        },
        runtime::{
            cleanup::{
                CleanupContext, CleanupPhase, CleanupSequence, CloseMode, ExplicitCloseError,
            },
            lifecycle::{LifecycleAction, LifecycleState, LifecycleStateMachine},
            sender::ReportSender,
            transport::{
                ActivityNotifier, SendAcceptance, TransportError, TransportErrorKind,
                TransportEvent, TransportPort, TransportResult,
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
    const DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

    #[test]
    fn explicit_close_finishes_pending_reply_before_neutral_drain_and_join() {
        let protocol = protocol();
        let mut sender = ReportSender::<Pro>::new();
        let mut transport = ScriptedCleanupPort::new([]);
        let mut lifecycle = open_lifecycle();
        send_pending_reply(&protocol, &mut sender, &mut transport)
            .expect("pending reply completes before cleanup");

        let completion =
            CleanupSequence::new(CloseMode::WithNeutral, DRAIN_TIMEOUT).run(CleanupContext {
                connected: true,
                now_ns: 20,
                lifecycle: &mut lifecycle,
                protocol: &protocol,
                sender: &mut sender,
                transport: &mut transport,
            });

        assert!(completion.performed());
        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        transport.trace.push(Trace::CompletionObserved);
        completion
            .finish_with_join(|| {
                transport.trace.push(Trace::Join);
                Ok::<(), ()>(())
            })
            .expect("cleanup and join succeed");
        assert_eq!(sender.timer(), 2);
        assert_eq!(
            transport.trace,
            [
                pending_reply(0),
                neutral_input(1),
                Trace::Drain(DRAIN_TIMEOUT),
                Trace::Disconnect,
                Trace::TransportClose,
                Trace::CompletionObserved,
                Trace::Join,
            ]
        );

        let trace_before_repeat = transport.trace.clone();
        let repeated =
            CleanupSequence::new(CloseMode::WithNeutral, DRAIN_TIMEOUT).run(CleanupContext {
                connected: true,
                now_ns: 30,
                lifecycle: &mut lifecycle,
                protocol: &protocol,
                sender: &mut sender,
                transport: &mut transport,
            });
        assert!(!repeated.performed());
        repeated
            .finish_with_join(|| {
                transport.trace.push(Trace::Join);
                Ok::<(), ()>(())
            })
            .expect("repeated close is a no-op");
        assert_eq!(transport.trace, trace_before_repeat);
    }

    #[test]
    fn close_without_neutral_keeps_the_cleanup_order_but_skips_the_report() {
        let protocol = protocol();
        let mut sender = ReportSender::<Pro>::new();
        let mut transport = ScriptedCleanupPort::new([]);
        let mut lifecycle = open_lifecycle();
        send_pending_reply(&protocol, &mut sender, &mut transport)
            .expect("pending reply completes before cleanup");

        let completion =
            CleanupSequence::new(CloseMode::WithoutNeutral, DRAIN_TIMEOUT).run(CleanupContext {
                connected: true,
                now_ns: 20,
                lifecycle: &mut lifecycle,
                protocol: &protocol,
                sender: &mut sender,
                transport: &mut transport,
            });
        transport.trace.push(Trace::CompletionObserved);
        completion
            .finish_with_join(|| {
                transport.trace.push(Trace::Join);
                Ok::<(), ()>(())
            })
            .expect("cleanup and join succeed");

        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        assert_eq!(sender.timer(), 1);
        assert_eq!(
            transport.trace,
            [
                pending_reply(0),
                Trace::Drain(DRAIN_TIMEOUT),
                Trace::Disconnect,
                Trace::TransportClose,
                Trace::CompletionObserved,
                Trace::Join,
            ]
        );
    }

    #[test]
    fn explicit_close_while_disconnected_skips_only_the_neutral_report() {
        let protocol = protocol();
        let mut sender = ReportSender::<Pro>::new();
        let mut transport = ScriptedCleanupPort::new([]);
        let mut lifecycle = open_lifecycle();

        let completion =
            CleanupSequence::new(CloseMode::WithNeutral, DRAIN_TIMEOUT).run(CleanupContext {
                connected: false,
                now_ns: 20,
                lifecycle: &mut lifecycle,
                protocol: &protocol,
                sender: &mut sender,
                transport: &mut transport,
            });
        transport.trace.push(Trace::CompletionObserved);
        completion
            .finish_with_join(|| {
                transport.trace.push(Trace::Join);
                Ok::<(), ()>(())
            })
            .expect("cleanup and join succeed");

        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        assert_eq!(sender.timer(), 0);
        assert_eq!(
            transport.trace,
            [
                Trace::Drain(DRAIN_TIMEOUT),
                Trace::Disconnect,
                Trace::TransportClose,
                Trace::CompletionObserved,
                Trace::Join,
            ]
        );
    }

    #[test]
    fn explicit_close_continues_after_each_phase_failure_and_keeps_the_first_error() {
        let cases = [
            FailureCase::single("pending reply", FailurePoint::PendingReply, None, None, 0),
            FailureCase::single(
                "neutral",
                FailurePoint::Neutral,
                Some(CleanupPhase::Neutral),
                Some(TransportErrorKind::SendRejected),
                1,
            ),
            FailureCase::single(
                "drain",
                FailurePoint::Drain,
                Some(CleanupPhase::DrainInterrupt),
                Some(TransportErrorKind::SourceTerminated),
                1,
            ),
            FailureCase::single(
                "disconnect",
                FailurePoint::Disconnect,
                Some(CleanupPhase::Disconnect),
                Some(TransportErrorKind::SourceTerminated),
                1,
            ),
            FailureCase::single(
                "transport close",
                FailurePoint::TransportClose,
                Some(CleanupPhase::TransportClose),
                Some(TransportErrorKind::SourceTerminated),
                1,
            ),
            FailureCase {
                name: "neutral then drain",
                failures: &[
                    FailurePoint::PendingReply,
                    FailurePoint::Neutral,
                    FailurePoint::Drain,
                ],
                expected_phase: Some(CleanupPhase::Neutral),
                expected_kind: Some(TransportErrorKind::SendRejected),
                neutral_timer: 0,
            },
        ];

        for case in cases {
            let protocol = protocol();
            let mut sender = ReportSender::<Pro>::new();
            let mut transport = ScriptedCleanupPort::new(case.failures.iter().copied());
            let mut lifecycle = open_lifecycle();
            let pending = send_pending_reply(&protocol, &mut sender, &mut transport);
            assert_eq!(
                pending.as_ref().err().map(TransportError::kind),
                case.failures
                    .contains(&FailurePoint::PendingReply)
                    .then_some(TransportErrorKind::SendRejected),
                "{}",
                case.name
            );

            let completion =
                CleanupSequence::new(CloseMode::WithNeutral, DRAIN_TIMEOUT).run(CleanupContext {
                    connected: true,
                    now_ns: 20,
                    lifecycle: &mut lifecycle,
                    protocol: &protocol,
                    sender: &mut sender,
                    transport: &mut transport,
                });
            transport.trace.push(Trace::CompletionObserved);
            let result = completion.finish_with_join(|| {
                transport.trace.push(Trace::Join);
                Ok::<(), ()>(())
            });
            match (case.expected_phase, case.expected_kind) {
                (Some(expected_phase), Some(expected_kind)) => {
                    let error = result.expect_err(case.name);
                    let ExplicitCloseError::Cleanup(error) = error else {
                        panic!("{}: expected cleanup failure", case.name);
                    };
                    assert_eq!(error.phase(), expected_phase, "{}", case.name);
                    assert_eq!(error.source_error().kind(), expected_kind, "{}", case.name);
                }
                (None, None) => result.expect(case.name),
                _ => panic!("{}: invalid failure expectation", case.name),
            }
            assert_eq!(lifecycle.state(), LifecycleState::Closed, "{}", case.name);
            assert_eq!(
                transport.trace,
                [
                    pending_reply(0),
                    neutral_input(case.neutral_timer),
                    Trace::Drain(DRAIN_TIMEOUT),
                    Trace::Disconnect,
                    Trace::TransportClose,
                    Trace::CompletionObserved,
                    Trace::Join,
                ],
                "{}",
                case.name
            );
        }
    }

    #[test]
    fn cleanup_and_join_failures_are_both_preserved_after_all_cleanup_phases() {
        let protocol = protocol();
        let mut sender = ReportSender::<Pro>::new();
        let mut transport = ScriptedCleanupPort::new([FailurePoint::Neutral, FailurePoint::Drain]);
        let mut lifecycle = open_lifecycle();

        let completion =
            CleanupSequence::new(CloseMode::WithNeutral, DRAIN_TIMEOUT).run(CleanupContext {
                connected: true,
                now_ns: 20,
                lifecycle: &mut lifecycle,
                protocol: &protocol,
                sender: &mut sender,
                transport: &mut transport,
            });
        transport.trace.push(Trace::CompletionObserved);
        let error = completion
            .finish_with_join(|| {
                transport.trace.push(Trace::Join);
                Err("join failed")
            })
            .expect_err("cleanup and join both fail");

        let ExplicitCloseError::CleanupAndJoin { cleanup, join } = error else {
            panic!("both failures must be preserved");
        };
        assert_eq!(cleanup.phase(), CleanupPhase::Neutral);
        assert_eq!(
            cleanup.source_error().kind(),
            TransportErrorKind::SendRejected
        );
        assert_eq!(join, "join failed");
        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        assert_eq!(
            transport.trace,
            [
                neutral_input(0),
                Trace::Drain(DRAIN_TIMEOUT),
                Trace::Disconnect,
                Trace::TransportClose,
                Trace::CompletionObserved,
                Trace::Join,
            ]
        );
    }

    struct FailureCase {
        name: &'static str,
        failures: &'static [FailurePoint],
        expected_phase: Option<CleanupPhase>,
        expected_kind: Option<TransportErrorKind>,
        neutral_timer: u8,
    }

    impl FailureCase {
        const fn single(
            name: &'static str,
            failure: FailurePoint,
            expected_phase: Option<CleanupPhase>,
            expected_kind: Option<TransportErrorKind>,
            neutral_timer: u8,
        ) -> Self {
            let failures = match failure {
                FailurePoint::PendingReply => &[FailurePoint::PendingReply],
                FailurePoint::Neutral => &[FailurePoint::Neutral],
                FailurePoint::Drain => &[FailurePoint::Drain],
                FailurePoint::Disconnect => &[FailurePoint::Disconnect],
                FailurePoint::TransportClose => &[FailurePoint::TransportClose],
            };
            Self {
                name,
                failures,
                expected_phase,
                expected_kind,
                neutral_timer,
            }
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum FailurePoint {
        PendingReply,
        Neutral,
        Drain,
        Disconnect,
        TransportClose,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Trace {
        Send {
            report_id: u8,
            timer: u8,
            buttons: [u8; 3],
            subcommand: Option<u8>,
        },
        Drain(Duration),
        Disconnect,
        TransportClose,
        CompletionObserved,
        Join,
    }

    fn pending_reply(timer: u8) -> Trace {
        Trace::Send {
            report_id: 0x21,
            timer,
            buttons: [0; 3],
            subcommand: Some(0x08),
        }
    }

    fn neutral_input(timer: u8) -> Trace {
        Trace::Send {
            report_id: 0x30,
            timer,
            buttons: [0; 3],
            subcommand: None,
        }
    }

    struct ScriptedCleanupPort {
        failures: Vec<FailurePoint>,
        trace: Vec<Trace>,
    }

    impl ScriptedCleanupPort {
        fn new(failures: impl IntoIterator<Item = FailurePoint>) -> Self {
            Self {
                failures: failures.into_iter().collect(),
                trace: Vec::new(),
            }
        }

        fn fails(&self, point: FailurePoint) -> bool {
            self.failures.contains(&point)
        }

        fn operation_result(&self, point: FailurePoint) -> TransportResult<()> {
            if self.fails(point) {
                let kind = match point {
                    FailurePoint::PendingReply | FailurePoint::Neutral => {
                        TransportErrorKind::SendRejected
                    }
                    FailurePoint::Drain
                    | FailurePoint::Disconnect
                    | FailurePoint::TransportClose => TransportErrorKind::SourceTerminated,
                };
                Err(TransportError::new(kind))
            } else {
                Ok(())
            }
        }
    }

    impl TransportPort for ScriptedCleanupPort {
        fn open(&mut self, _activity: ActivityNotifier) -> TransportResult<()> {
            Ok(())
        }

        fn poll(&mut self, _timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            Ok(Vec::new())
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
            let report_id = payload[0];
            let point = if report_id == 0x21 {
                FailurePoint::PendingReply
            } else {
                FailurePoint::Neutral
            };
            self.trace.push(Trace::Send {
                report_id,
                timer: payload[1],
                buttons: [payload[3], payload[4], payload[5]],
                subcommand: (report_id == 0x21).then_some(payload[14]),
            });
            self.operation_result(point)?;
            Ok(SendAcceptance::ACCEPTED)
        }

        fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
            self.trace.push(Trace::Drain(timeout));
            self.operation_result(FailurePoint::Drain)
        }

        fn disconnect(&mut self) -> TransportResult<()> {
            self.trace.push(Trace::Disconnect);
            self.operation_result(FailurePoint::Disconnect)
        }

        fn close(&mut self) -> TransportResult<()> {
            self.trace.push(Trace::TransportClose);
            self.operation_result(FailurePoint::TransportClose)
        }
    }

    fn send_pending_reply(
        protocol: &SwitchHidProtocol<Pro>,
        sender: &mut ReportSender<Pro>,
        transport: &mut dyn TransportPort,
    ) -> TransportResult<()> {
        let raw = subcommand_report(0x08, &[]);
        let OutputReport::Subcommand { request, .. } =
            parse_output_report(&raw).expect("valid output report")
        else {
            panic!("0x01 must contain a subcommand");
        };
        let prepared = sender
            .prepare_reply(protocol, request, &InputState::neutral())
            .expect("device-info status is supported");
        sender.send_reply(prepared, transport).map(|_| ())
    }

    fn open_lifecycle() -> LifecycleStateMachine {
        let mut lifecycle = LifecycleStateMachine::new();
        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::OpenTransport));
        assert_eq!(lifecycle.complete_open(), LifecycleAction::Opened);
        lifecycle
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
