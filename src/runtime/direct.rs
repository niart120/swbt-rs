use std::{error::Error as StdError, fmt, time::Duration};

use crate::{
    controller::input::TapPlan,
    input::InputState,
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    runtime::{
        connection::ObservedSubcommands,
        output::{
            OutputHandling, OutputHandlingContext, OutputHandlingError, OutputObservation,
            handle_output,
        },
        sender::ReportSender,
        state::InputStateStore,
        transport::{
            SendAcceptance, TransportError, TransportEvent, TransportPort, TransportResult,
        },
    },
};

pub(crate) fn send_candidate<M: ControllerModel>(
    candidate: InputState<M>,
    now_ns: u64,
    store: &mut InputStateStore<M>,
    protocol: &SwitchHidProtocol<M>,
    sender: &mut ReportSender<M>,
    transport: &mut dyn TransportPort,
) -> TransportResult<SendAcceptance> {
    let acceptance = sender.send_input(protocol, &candidate, now_ns, transport)?;
    store.commit(candidate);
    Ok(acceptance)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DirectTapInterruption {
    Disconnected { reason: Option<u8> },
    Shutdown,
}

#[derive(Debug)]
pub(crate) enum DirectTapError {
    NotReady,
    DeadlineOverflow,
    ClockOverflow,
    Transport(TransportError),
    Interrupted(DirectTapInterruption),
}

impl From<TransportError> for DirectTapError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

impl fmt::Display for DirectTapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReady => formatter.write_str("direct input requires a ready runtime"),
            Self::DeadlineOverflow => formatter.write_str("direct tap deadline overflowed"),
            Self::ClockOverflow => formatter.write_str("monotonic clock exceeds nanosecond range"),
            Self::Transport(error) => write!(formatter, "direct transport error: {error}"),
            Self::Interrupted(DirectTapInterruption::Disconnected { reason }) => {
                write!(formatter, "direct tap interrupted by disconnect {reason:?}")
            }
            Self::Interrupted(DirectTapInterruption::Shutdown) => {
                formatter.write_str("direct tap interrupted by shutdown")
            }
        }
    }
}

impl StdError for DirectTapError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::NotReady
            | Self::DeadlineOverflow
            | Self::ClockOverflow
            | Self::Interrupted(_) => None,
        }
    }
}

pub(crate) enum DirectTapStimulus {
    #[cfg(test)]
    Time(Duration),
    Transport(TransportEvent),
    Shutdown,
}

pub(crate) struct DirectTapContext<'a, M: ControllerModel> {
    pub(crate) observe_output: &'a mut dyn FnMut(OutputObservation),
    pub(crate) protocol: &'a SwitchHidProtocol<M>,
    pub(crate) state: &'a mut InputStateStore<M>,
    pub(crate) observed: &'a mut ObservedSubcommands,
    pub(crate) sender: &'a mut ReportSender<M>,
    pub(crate) status: Option<&'a crate::runtime::status::StatusPublisher<M>>,
    pub(crate) transport: &'a mut dyn TransportPort,
}

pub(crate) enum DirectTapStep<M: ControllerModel> {
    Pending {
        tap: PendingDirectTap<M>,
        output: Option<Result<OutputHandling, OutputHandlingError>>,
    },
    Complete(Result<(), DirectTapError>),
}

pub(crate) struct PendingDirectTap<M: ControllerModel> {
    released: InputState<M>,
    release_at: Duration,
}

impl<M: ControllerModel> PendingDirectTap<M> {
    #[must_use]
    pub(crate) const fn release_at(&self) -> Duration {
        self.release_at
    }

    pub(crate) fn finish(
        self,
        now: Duration,
        state: &mut InputStateStore<M>,
        protocol: &SwitchHidProtocol<M>,
        sender: &mut ReportSender<M>,
        transport: &mut dyn TransportPort,
    ) -> Result<(), DirectTapError> {
        let now_ns = monotonic_ns(now)?;
        send_candidate(self.released, now_ns, state, protocol, sender, transport)
            .map(|_acceptance| ())
            .map_err(DirectTapError::Transport)
    }

    pub(crate) fn step(
        self,
        stimulus: DirectTapStimulus,
        context: DirectTapContext<'_, M>,
    ) -> DirectTapStep<M> {
        let DirectTapContext {
            observe_output,
            protocol,
            state,
            observed,
            sender,
            status,
            transport,
        } = context;

        match stimulus {
            #[cfg(test)]
            DirectTapStimulus::Time(now) if now < self.release_at => DirectTapStep::Pending {
                tap: self,
                output: None,
            },
            #[cfg(test)]
            DirectTapStimulus::Time(now) => {
                DirectTapStep::Complete(self.finish(now, state, protocol, sender, transport))
            }
            DirectTapStimulus::Transport(TransportEvent::HidOutput { channel, payload }) => {
                let current = state.snapshot();
                let output = handle_output(
                    channel,
                    &payload,
                    OutputHandlingContext {
                        observe_output,
                        protocol,
                        current: &current,
                        observed,
                        sender,
                        status,
                        transport,
                    },
                );
                DirectTapStep::Pending {
                    tap: self,
                    output: Some(output),
                }
            }
            DirectTapStimulus::Transport(TransportEvent::Disconnected { reason }) => {
                DirectTapStep::Complete(Err(DirectTapError::Interrupted(
                    DirectTapInterruption::Disconnected { reason },
                )))
            }
            DirectTapStimulus::Shutdown => DirectTapStep::Complete(Err(
                DirectTapError::Interrupted(DirectTapInterruption::Shutdown),
            )),
            DirectTapStimulus::Transport(TransportEvent::Connected) => DirectTapStep::Pending {
                tap: self,
                output: None,
            },
            DirectTapStimulus::Transport(TransportEvent::HidChannelOpened { .. }) => {
                DirectTapStep::Pending {
                    tap: self,
                    output: None,
                }
            }
        }
    }
}

pub(crate) fn begin_tap<M: ControllerModel>(
    ready: bool,
    plan: TapPlan<M>,
    started_at: Duration,
    state: &mut InputStateStore<M>,
    protocol: &SwitchHidProtocol<M>,
    sender: &mut ReportSender<M>,
    transport: &mut dyn TransportPort,
) -> Result<PendingDirectTap<M>, DirectTapError> {
    if !ready {
        return Err(DirectTapError::NotReady);
    }

    let (pressed, released, duration) = plan.into_parts();
    let release_at = started_at
        .checked_add(duration)
        .ok_or(DirectTapError::DeadlineOverflow)?;
    let now_ns = monotonic_ns(started_at)?;
    monotonic_ns(release_at)?;
    send_candidate(pressed, now_ns, state, protocol, sender, transport)?;
    Ok(PendingDirectTap {
        released,
        release_at,
    })
}

fn monotonic_ns(now: Duration) -> Result<u64, DirectTapError> {
    u64::try_from(now.as_nanos()).map_err(|_| DirectTapError::ClockOverflow)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        controller::input::{press_candidate, tap_plan},
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::SwitchHidProtocol,
        runtime::{
            connection::ObservedSubcommands,
            direct::{
                DirectTapContext, DirectTapError, DirectTapInterruption, DirectTapStep,
                DirectTapStimulus, PendingDirectTap, begin_tap as begin_direct_tap, send_candidate,
            },
            output::{OutputHandling, OutputHandlingContext, OutputHandlingError, handle_output},
            sender::ReportSender,
            state::InputStateStore,
            transport::{
                ActivityNotifier, HidChannel, SendAcceptance, TransportErrorKind, TransportEvent,
                TransportPort, TransportResult, activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

    #[test]
    fn direct_send_commits_only_after_acceptance_and_retry() {
        let mut harness = Harness::new();
        harness.control.script_sends([
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Rejected,
            ScriptedSendOutcome::Accepted,
        ]);
        let pressed_a = pressed_state(ButtonKind::A);
        let pressed_x = pressed_state(ButtonKind::X);

        harness
            .send(pressed_a.clone())
            .expect("first candidate accepted");
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);

        let error = harness
            .send(pressed_x.clone())
            .expect_err("second candidate rejected");
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);

        harness
            .send(pressed_x.clone())
            .expect("retry candidate accepted");
        assert_eq!(harness.store.snapshot(), pressed_x);
        assert_eq!(harness.sender.timer(), 2);
        assert_eq!(
            harness
                .control
                .accepted_interrupts()
                .iter()
                .map(|report| { (report[0], report[1], [report[3], report[4], report[5]],) })
                .collect::<Vec<_>>(),
            [(0x30, 0, [0x08, 0x00, 0x00]), (0x30, 1, [0x02, 0x00, 0x00]),]
        );
    }

    #[test]
    fn accepted_then_disconnect_remains_committed() {
        let mut harness = Harness::new();
        harness
            .control
            .script_sends([ScriptedSendOutcome::AcceptedThenDisconnect { reason: Some(0x13) }]);
        let pressed_a = pressed_state(ButtonKind::A);

        harness
            .send(pressed_a.clone())
            .expect("input accepted before disconnect event");
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);

        assert_eq!(
            harness
                .transport
                .poll(Duration::ZERO)
                .expect("queued disconnect follows accepted send"),
            [TransportEvent::Disconnected { reason: Some(0x13) }]
        );
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);
    }

    #[test]
    fn direct_tap_stops_after_a_rejected_press_without_attempting_release() {
        let mut harness = Harness::new();
        harness.make_protocol_ready();
        let current = pressed_state(ButtonKind::ZL);
        harness.store.commit(current.clone());
        harness
            .control
            .script_sends([ScriptedSendOutcome::Rejected, ScriptedSendOutcome::Accepted]);
        let plan =
            tap_plan(&current, [ProButton::A], Duration::from_millis(80)).expect("valid tap plan");

        let error = match harness.begin_tap(true, plan, Duration::from_secs(10)) {
            Err(error) => error,
            Ok(_) => panic!("rejected press must not create a pending tap"),
        };

        let DirectTapError::Transport(error) = error else {
            panic!("press rejection must retain its transport error");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(harness.store.snapshot(), current);
        assert_eq!(harness.sender.timer(), 2);
        assert_eq!(harness.transport.attempts.len(), 3);
        assert_eq!(harness.control.accepted_interrupts().len(), 2);
        assert_eq!(
            (
                harness.transport.attempts[2][0],
                harness.transport.attempts[2][1],
                [
                    harness.transport.attempts[2][3],
                    harness.transport.attempts[2][4],
                    harness.transport.attempts[2][5],
                ],
            ),
            (0x30, 2, [0x08, 0x00, 0x80])
        );
    }

    #[test]
    fn direct_tap_handles_inbound_reply_before_rejected_release() {
        let mut harness = Harness::new();
        harness.make_protocol_ready();
        let current = pressed_state(ButtonKind::ZL);
        harness.store.commit(current.clone());
        let inbound = subcommand_report(0x08, &[]);
        harness.control.script_sends([
            ScriptedSendOutcome::AcceptedThenEvent(TransportEvent::HidOutput {
                channel: HidChannel::Interrupt,
                payload: inbound.into_boxed_slice(),
            }),
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Rejected,
        ]);
        let started_at = Duration::from_secs(10);
        let duration = Duration::from_millis(80);
        let plan = tap_plan(&current, [ProButton::A], duration).expect("valid tap plan");
        let pressed = InputState::neutral().with_buttons([ProButton::A, ProButton::ZL]);

        let pending = harness
            .begin_tap(true, plan, started_at)
            .expect("tap press accepted");
        assert_eq!(harness.store.snapshot(), pressed);
        assert_eq!(pending.release_at(), started_at + duration);
        assert_eq!(harness.sender.timer(), 3);

        let DirectTapStep::Pending {
            tap: pending,
            output: None,
        } = harness.step_tap(
            pending,
            DirectTapStimulus::Time(started_at + duration - Duration::from_nanos(1)),
        )
        else {
            panic!("time before the release deadline must keep the tap pending");
        };
        assert_eq!(harness.store.snapshot(), pressed);
        assert_eq!(harness.sender.timer(), 3);
        assert_eq!(harness.transport.attempts.len(), 3);

        let [event] = harness
            .transport
            .poll(Duration::ZERO)
            .expect("accepted press queues one inbound output")
            .try_into()
            .expect("one queued event");
        let DirectTapStep::Pending {
            tap: pending,
            output: Some(output),
        } = harness.step_tap(pending, DirectTapStimulus::Transport(event))
        else {
            panic!("inbound reply must keep the tap pending");
        };
        assert!(matches!(output, Ok(OutputHandling::ReplyAccepted(_))));
        assert!(!harness.observed.observe(0x08));
        assert_eq!(harness.store.snapshot(), pressed);
        assert_eq!(harness.sender.timer(), 4);

        let DirectTapStep::Complete(result) =
            harness.step_tap(pending, DirectTapStimulus::Time(started_at + duration))
        else {
            panic!("deadline must complete the tap");
        };
        let error = result.expect_err("release is scripted to fail");
        let DirectTapError::Transport(error) = error else {
            panic!("release rejection must retain its transport error");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(harness.store.snapshot(), pressed);
        assert_eq!(harness.sender.timer(), 4);
        assert_eq!(harness.control.accepted_interrupts().len(), 4);
        assert_eq!(
            harness
                .transport
                .attempts
                .iter()
                .map(|report| {
                    (
                        report[0],
                        report[1],
                        [report[3], report[4], report[5]],
                        (report[0] == 0x21).then_some(report[14]),
                    )
                })
                .collect::<Vec<_>>(),
            [
                (0x21, 0, [0x00, 0x00, 0x00], Some(0x03)),
                (0x21, 1, [0x00, 0x00, 0x00], Some(0x30)),
                (0x30, 2, [0x08, 0x00, 0x80], None),
                (0x21, 3, [0x08, 0x00, 0x80], Some(0x08)),
                (0x30, 4, [0x00, 0x00, 0x80], None),
            ]
        );
    }

    #[test]
    fn direct_tap_disconnect_and_shutdown_cancel_without_release() {
        {
            let (mut harness, pending, pressed) =
                ready_pending_tap(ScriptedSendOutcome::AcceptedThenDisconnect {
                    reason: Some(0x13),
                });
            let [event] = harness
                .transport
                .poll(Duration::ZERO)
                .expect("disconnect follows accepted press")
                .try_into()
                .expect("one disconnect event");

            let error =
                completed_error(harness.step_tap(pending, DirectTapStimulus::Transport(event)));

            assert_eq!(
                interruption(error),
                DirectTapInterruption::Disconnected { reason: Some(0x13) }
            );
            assert_cancelled_tap(&harness, &pressed);
        }

        {
            let (mut harness, pending, pressed) = ready_pending_tap(ScriptedSendOutcome::Accepted);

            let error = completed_error(harness.step_tap(pending, DirectTapStimulus::Shutdown));

            assert_eq!(interruption(error), DirectTapInterruption::Shutdown);
            assert_cancelled_tap(&harness, &pressed);
        }
    }

    struct Harness {
        protocol: SwitchHidProtocol<Pro>,
        sender: ReportSender<Pro>,
        store: InputStateStore<Pro>,
        observed: ObservedSubcommands,
        transport: RecordingTransport,
        control: FakeTransportControl,
    }

    impl Harness {
        fn new() -> Self {
            let (transport, control) = FakeTransport::with_limits(8, 8);
            let mut transport = RecordingTransport::new(transport);
            let (notifier, _wake_receiver) = activity_channel();
            transport.open(notifier).expect("open fake transport");
            Self {
                protocol: SwitchHidProtocol::new(None, DEVICE_INFO_ADDRESS),
                sender: ReportSender::new(),
                store: InputStateStore::new(),
                observed: ObservedSubcommands::default(),
                transport,
                control,
            }
        }

        fn send(
            &mut self,
            candidate: InputState<Pro>,
        ) -> crate::runtime::transport::TransportResult<crate::runtime::transport::SendAcceptance>
        {
            send_candidate(
                candidate,
                0,
                &mut self.store,
                &self.protocol,
                &mut self.sender,
                &mut self.transport,
            )
        }

        fn make_protocol_ready(&mut self) {
            assert!(matches!(
                self.handle_subcommand(0x03, &[0x30]),
                Ok(OutputHandling::ReplyAccepted(_))
            ));
            assert!(matches!(
                self.handle_subcommand(0x30, &[0x01]),
                Ok(OutputHandling::ReplyAccepted(_))
            ));
            assert!(self.sender.session().protocol_ready());
        }

        fn handle_subcommand(
            &mut self,
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
                    status: None,
                    transport: &mut self.transport,
                },
            )
        }

        fn begin_tap(
            &mut self,
            ready: bool,
            plan: crate::controller::input::TapPlan<Pro>,
            started_at: Duration,
        ) -> Result<PendingDirectTap<Pro>, DirectTapError> {
            begin_direct_tap(
                ready,
                plan,
                started_at,
                &mut self.store,
                &self.protocol,
                &mut self.sender,
                &mut self.transport,
            )
        }

        fn step_tap(
            &mut self,
            tap: PendingDirectTap<Pro>,
            stimulus: DirectTapStimulus,
        ) -> DirectTapStep<Pro> {
            let mut ignore_output = |_| {};
            tap.step(
                stimulus,
                DirectTapContext {
                    observe_output: &mut ignore_output,
                    protocol: &self.protocol,
                    state: &mut self.store,
                    observed: &mut self.observed,
                    sender: &mut self.sender,
                    status: None,
                    transport: &mut self.transport,
                },
            )
        }
    }

    fn pressed_state(kind: ButtonKind) -> InputState<Pro> {
        let button = ProButton::try_from(kind).expect("button supported by Pro Controller");
        press_candidate(&InputState::neutral(), [button]).expect("non-empty press is valid")
    }

    fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut raw = vec![0x01, 0];
        raw.extend_from_slice(&NEUTRAL_RUMBLE);
        raw.push(subcommand_id);
        raw.extend_from_slice(payload);
        raw
    }

    fn ready_pending_tap(
        press_outcome: ScriptedSendOutcome,
    ) -> (Harness, PendingDirectTap<Pro>, InputState<Pro>) {
        let mut harness = Harness::new();
        harness.make_protocol_ready();
        let current = pressed_state(ButtonKind::ZL);
        harness.store.commit(current.clone());
        harness
            .control
            .script_sends([press_outcome, ScriptedSendOutcome::Accepted]);
        let plan =
            tap_plan(&current, [ProButton::A], Duration::from_millis(80)).expect("valid tap plan");
        let pending = harness
            .begin_tap(true, plan, Duration::from_secs(10))
            .expect("tap press accepted");
        let pressed = InputState::neutral().with_buttons([ProButton::A, ProButton::ZL]);
        assert_eq!(harness.store.snapshot(), pressed);
        (harness, pending, pressed)
    }

    fn completed_error(step: DirectTapStep<Pro>) -> DirectTapError {
        let DirectTapStep::Complete(result) = step else {
            panic!("interruption must complete the tap");
        };
        result.expect_err("interruption must fail the tap")
    }

    fn interruption(error: DirectTapError) -> DirectTapInterruption {
        let DirectTapError::Interrupted(interruption) = error else {
            panic!("tap must retain its interruption cause");
        };
        interruption
    }

    fn assert_cancelled_tap(harness: &Harness, pressed: &InputState<Pro>) {
        assert_eq!(harness.store.snapshot(), *pressed);
        assert_eq!(harness.sender.timer(), 3);
        assert_eq!(harness.transport.attempts.len(), 3);
        assert_eq!(harness.control.accepted_interrupts().len(), 3);
    }

    struct RecordingTransport {
        inner: FakeTransport,
        attempts: Vec<Box<[u8]>>,
    }

    impl RecordingTransport {
        fn new(inner: FakeTransport) -> Self {
            Self {
                inner,
                attempts: Vec::new(),
            }
        }
    }

    impl TransportPort for RecordingTransport {
        fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
            self.inner.open(activity)
        }

        fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            self.inner.poll(timeout)
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
            self.attempts.push(Box::from(payload));
            self.inner.send_interrupt(payload)
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
}
