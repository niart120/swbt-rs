#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T11 defines periodic policy before T21 worker integration"
    )
)]

use std::{error::Error as StdError, fmt, time::Duration};

use crate::{
    controller::input::TapPlan,
    input::InputState,
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    runtime::{
        output::{OutputHandling, OutputHandlingError},
        scheduler::{ReportScheduler, SchedulerError, TickDecision},
        sender::ReportSender,
        state::InputStateStore,
        transport::{SendAcceptance, TransportError, TransportPort, TransportResult},
    },
};

const REPLY_HOLDOFF: Duration = Duration::from_millis(300);

pub(crate) fn commit_candidate<M: ControllerModel>(
    candidate: InputState<M>,
    state: &mut InputStateStore<M>,
) {
    state.commit(candidate);
}

pub(crate) struct PendingPeriodicTap<M: ControllerModel> {
    released: InputState<M>,
    duration: Duration,
    first_error: Option<TransportError>,
}

impl<M: ControllerModel> PendingPeriodicTap<M> {
    #[must_use]
    pub(crate) const fn duration(&self) -> Duration {
        self.duration
    }

    pub(crate) fn finish(
        self,
        now_ns: u64,
        state: &mut InputStateStore<M>,
        protocol: &SwitchHidProtocol<M>,
        sender: &mut ReportSender<M>,
        transport: &mut dyn TransportPort,
    ) -> Result<(), PeriodicError> {
        let release_error =
            commit_and_send_candidate(self.released, now_ns, state, protocol, sender, transport)
                .err();

        match self.first_error.or(release_error) {
            Some(error) => Err(PeriodicError::Transport(error)),
            None => Ok(()),
        }
    }
}

pub(crate) fn begin_tap<M: ControllerModel>(
    ready: bool,
    plan: TapPlan<M>,
    now_ns: u64,
    state: &mut InputStateStore<M>,
    protocol: &SwitchHidProtocol<M>,
    sender: &mut ReportSender<M>,
    transport: &mut dyn TransportPort,
) -> Result<PendingPeriodicTap<M>, PeriodicError> {
    if !ready {
        return Err(PeriodicError::NotReady);
    }

    let (pressed, released, duration) = plan.into_parts();
    let first_error =
        commit_and_send_candidate(pressed, now_ns, state, protocol, sender, transport).err();
    Ok(PendingPeriodicTap {
        released,
        duration,
        first_error,
    })
}

fn commit_and_send_candidate<M: ControllerModel>(
    candidate: InputState<M>,
    now_ns: u64,
    state: &mut InputStateStore<M>,
    protocol: &SwitchHidProtocol<M>,
    sender: &mut ReportSender<M>,
    transport: &mut dyn TransportPort,
) -> TransportResult<SendAcceptance> {
    commit_candidate(candidate, state);
    sender.send_input(protocol, &state.snapshot(), now_ns, transport)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomaticInput {
    NotDue,
    HeldOff {
        until: Duration,
    },
    Sent {
        acceptance: SendAcceptance,
        skipped: u64,
    },
}

#[derive(Debug)]
pub(crate) enum PeriodicError {
    NotReady,
    Scheduler(SchedulerError),
    ClockOverflow,
    Transport(TransportError),
}

impl From<SchedulerError> for PeriodicError {
    fn from(error: SchedulerError) -> Self {
        Self::Scheduler(error)
    }
}

impl From<TransportError> for PeriodicError {
    fn from(error: TransportError) -> Self {
        Self::Transport(error)
    }
}

impl fmt::Display for PeriodicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotReady => formatter.write_str("periodic input requires a ready runtime"),
            Self::Scheduler(error) => write!(formatter, "periodic scheduler error: {error}"),
            Self::ClockOverflow => formatter.write_str("monotonic clock exceeds nanosecond range"),
            Self::Transport(error) => write!(formatter, "periodic transport error: {error}"),
        }
    }
}

impl StdError for PeriodicError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::NotReady => None,
            Self::Scheduler(error) => Some(error),
            Self::ClockOverflow => None,
            Self::Transport(error) => Some(error),
        }
    }
}

pub(crate) type PeriodicResult = Result<AutomaticInput, PeriodicError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PeriodicStart {
    HeldOff { until: Duration },
    Started { first_deadline: Duration },
}

pub(crate) struct PeriodicPolicy {
    period: Duration,
    scheduler: Option<ReportScheduler>,
    reply_holdoff_until: Option<Duration>,
}

impl PeriodicPolicy {
    pub(crate) fn new(period: Duration) -> Result<Self, SchedulerError> {
        if period.is_zero() {
            return Err(SchedulerError::ZeroPeriod);
        }
        Ok(Self {
            period,
            scheduler: None,
            reply_holdoff_until: None,
        })
    }

    pub(crate) fn start_when_unheld(
        &mut self,
        now: Duration,
    ) -> Result<PeriodicStart, SchedulerError> {
        if let Some(until) = self.reply_holdoff_until {
            if now < until {
                return Ok(PeriodicStart::HeldOff { until });
            }
        }
        if self.scheduler.is_none() {
            self.scheduler = Some(ReportScheduler::start(now, self.period)?);
        }
        let first_deadline = self
            .scheduler
            .as_ref()
            .expect("scheduler was started above")
            .next_deadline();
        Ok(PeriodicStart::Started { first_deadline })
    }

    pub(crate) fn record_output_completion(
        &mut self,
        completed_at: Duration,
        completion: &Result<OutputHandling, OutputHandlingError>,
    ) -> Result<(), PeriodicError> {
        if !matches!(completion, Ok(OutputHandling::ReplyAccepted(_))) {
            return Ok(());
        }

        let candidate = completed_at
            .checked_add(REPLY_HOLDOFF)
            .ok_or(SchedulerError::DeadlineOverflow)?;
        self.reply_holdoff_until = Some(
            self.reply_holdoff_until
                .map_or(candidate, |current| current.max(candidate)),
        );
        Ok(())
    }

    #[must_use]
    pub(crate) const fn reply_holdoff_until(&self) -> Option<Duration> {
        self.reply_holdoff_until
    }

    pub(crate) fn reset_for_new_session(&mut self) {
        self.stop_session();
    }

    pub(crate) fn stop_session(&mut self) {
        self.scheduler = None;
        self.reply_holdoff_until = None;
    }

    #[must_use]
    pub(crate) fn next_deadline(&self) -> Option<Duration> {
        self.scheduler.as_ref().map(|scheduler| {
            self.reply_holdoff_until
                .map_or(scheduler.next_deadline(), |holdoff| {
                    holdoff.max(scheduler.next_deadline())
                })
        })
    }

    pub(crate) fn send_due<M: ControllerModel>(
        &mut self,
        now: Duration,
        state: &InputStateStore<M>,
        protocol: &SwitchHidProtocol<M>,
        sender: &mut ReportSender<M>,
        transport: &mut dyn TransportPort,
    ) -> PeriodicResult {
        let scheduler = self.scheduler.as_mut().ok_or(PeriodicError::NotReady)?;
        if let Some(until) = self.reply_holdoff_until {
            if now < until {
                return Ok(AutomaticInput::HeldOff { until });
            }
        }

        let TickDecision::Due { skipped } = scheduler.step(now)? else {
            return Ok(AutomaticInput::NotDue);
        };
        let snapshot = state.snapshot();
        let now_ns = u64::try_from(now.as_nanos()).map_err(|_| PeriodicError::ClockOverflow)?;
        let acceptance = sender.send_input(protocol, &snapshot, now_ns, transport)?;
        Ok(AutomaticInput::Sent {
            acceptance,
            skipped,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::time::Duration;

    use crate::{
        controller::input::tap_plan,
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::{DeviceInfoBluetoothAddress, SwitchHidProtocol},
        runtime::{
            connection::ObservedSubcommands,
            output::{OutputHandling, OutputHandlingContext, OutputHandlingError, handle_output},
            periodic::{AutomaticInput, PeriodicError, PeriodicPolicy, PeriodicStart, begin_tap},
            sender::ReportSender,
            state::InputStateStore,
            transport::{
                ActivityNotifier, HidChannel, SendAcceptance, TransportError, TransportErrorKind,
                TransportEvent, TransportPort, TransportResult, activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const REPORT_PERIOD: Duration = Duration::from_millis(100);
    const REPLY_HOLDOFF: Duration = Duration::from_millis(300);
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

    #[test]
    fn accepted_reply_holds_latest_input_until_the_300_ms_boundary() {
        let mut harness = Harness::new();
        harness
            .control
            .script_sends([ScriptedSendOutcome::Accepted, ScriptedSendOutcome::Accepted]);
        harness.store.commit(pressed_state(ButtonKind::A));

        let accepted = harness
            .handle_subcommand(Duration::ZERO, 0x40, &[0x02])
            .expect("IMU reply accepted");
        assert!(matches!(accepted, OutputHandling::ReplyAccepted(_)));
        assert_eq!(harness.policy.reply_holdoff_until(), Some(REPLY_HOLDOFF));
        assert_eq!(harness.policy.next_deadline(), Some(REPLY_HOLDOFF));

        harness.store.commit(pressed_state(ButtonKind::B));
        assert_eq!(
            harness
                .send_due(Duration::from_millis(299))
                .expect("holdoff check succeeds"),
            AutomaticInput::HeldOff {
                until: REPLY_HOLDOFF
            }
        );
        assert_eq!(harness.control.accepted_interrupts().len(), 1);

        let sent = harness
            .send_due(REPLY_HOLDOFF)
            .expect("input accepted at holdoff boundary");
        assert!(matches!(sent, AutomaticInput::Sent { skipped: 2, .. }));
        assert_eq!(
            harness.policy.next_deadline(),
            Some(Duration::from_millis(400))
        );
        assert_eq!(
            harness
                .send_due(REPLY_HOLDOFF)
                .expect("same instant is no longer due"),
            AutomaticInput::NotDue
        );

        let accepted = harness.control.accepted_interrupts();
        assert_eq!(
            accepted
                .iter()
                .map(|report| (report[0], report[1]))
                .collect::<Vec<_>>(),
            [(0x21, 0), (0x30, 1)]
        );
        assert_eq!(accepted[0][14], 0x40);
        assert_eq!(&accepted[1][3..6], &[0x04, 0x00, 0x00]);
        assert_eq!(harness.sender.timer(), 2);
    }

    #[test]
    fn rejected_reply_does_not_start_holdoff() {
        let mut harness = Harness::new();
        harness
            .control
            .script_sends([ScriptedSendOutcome::Rejected, ScriptedSendOutcome::Accepted]);
        harness.store.commit(pressed_state(ButtonKind::A));

        let rejected = harness
            .handle_subcommand(Duration::ZERO, 0x40, &[0x02])
            .expect_err("IMU reply rejected");
        let OutputHandlingError::Transport(error) = rejected else {
            panic!("scripted rejection must remain a transport error");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(harness.policy.reply_holdoff_until(), None);
        assert_eq!(harness.policy.next_deadline(), Some(REPORT_PERIOD));

        assert_eq!(
            harness
                .send_due(Duration::from_millis(99))
                .expect("pre-deadline check succeeds"),
            AutomaticInput::NotDue
        );
        let sent = harness
            .send_due(REPORT_PERIOD)
            .expect("rejected reply does not suppress input");
        assert!(matches!(sent, AutomaticInput::Sent { skipped: 0, .. }));

        let accepted = harness.control.accepted_interrupts();
        assert_eq!(accepted.len(), 1);
        assert_eq!((accepted[0][0], accepted[0][1]), (0x30, 0));
        assert_eq!(&accepted[0][3..6], &[0x08, 0x00, 0x00]);
        assert_eq!(harness.sender.timer(), 1);
    }

    #[test]
    fn later_accepted_reply_extends_holdoff_from_its_completion_time() {
        let mut harness = Harness::new();
        harness.control.script_sends([
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
        ]);

        harness
            .handle_subcommand(Duration::ZERO, 0x08, &[])
            .expect("first reply accepted");
        assert_eq!(harness.policy.reply_holdoff_until(), Some(REPLY_HOLDOFF));

        harness
            .handle_subcommand(Duration::from_millis(100), 0x08, &[])
            .expect("later reply accepted");
        assert_eq!(
            harness.policy.reply_holdoff_until(),
            Some(Duration::from_millis(400))
        );
        assert_eq!(
            harness
                .send_due(Duration::from_millis(399))
                .expect("extended holdoff check succeeds"),
            AutomaticInput::HeldOff {
                until: Duration::from_millis(400)
            }
        );

        let sent = harness
            .send_due(Duration::from_millis(400))
            .expect("input accepted at extended boundary");
        assert!(matches!(sent, AutomaticInput::Sent { skipped: 3, .. }));
        assert_eq!(
            harness.policy.next_deadline(),
            Some(Duration::from_millis(500))
        );
        assert_eq!(
            harness
                .control
                .accepted_interrupts()
                .iter()
                .map(|report| (report[0], report[1]))
                .collect::<Vec<_>>(),
            [(0x21, 0), (0x21, 1), (0x30, 2)]
        );
    }

    #[test]
    fn periodic_tap_rejects_not_ready_before_press_or_send() {
        let protocol = protocol();
        let current = pressed_state(ButtonKind::ZL);
        let plan =
            tap_plan(&current, [ProButton::A], Duration::from_millis(80)).expect("valid tap plan");
        let mut store = InputStateStore::new();
        store.commit(current.clone());
        let mut sender = ReportSender::new();
        let session = sender.session();
        let mut transport = FailingTransport::new([TransportErrorKind::SendRejected]);

        let error = match begin_tap(
            false,
            plan,
            0,
            &mut store,
            &protocol,
            &mut sender,
            &mut transport,
        ) {
            Err(error) => error,
            Ok(_) => panic!("non-ready tap must be rejected"),
        };

        assert!(matches!(error, PeriodicError::NotReady));
        assert_eq!(store.snapshot(), current);
        assert_eq!(sender.timer(), 0);
        assert_eq!(sender.session(), session);
        assert!(transport.attempts.is_empty());
        assert_eq!(transport.failures.len(), 1);
    }

    #[test]
    fn periodic_tap_commits_both_states_and_returns_the_first_send_error() {
        let protocol = protocol();
        let current = InputState::neutral().with_buttons([ProButton::A, ProButton::ZL]);
        let plan = tap_plan(
            &current,
            [ProButton::A, ProButton::B],
            Duration::from_millis(80),
        )
        .expect("valid tap plan");
        let pressed =
            InputState::neutral().with_buttons([ProButton::A, ProButton::B, ProButton::ZL]);
        let released = pressed_state(ButtonKind::ZL);
        let mut store = InputStateStore::new();
        store.commit(current.clone());
        let mut sender = ReportSender::new();
        let mut transport =
            FailingTransport::new([TransportErrorKind::SendRejected, TransportErrorKind::Closed]);

        let pending = begin_tap(
            true,
            plan,
            10,
            &mut store,
            &protocol,
            &mut sender,
            &mut transport,
        )
        .expect("ready tap starts even when its press send fails");

        assert_eq!(store.snapshot(), pressed);
        assert_eq!(pending.duration(), Duration::from_millis(80));
        assert_eq!(transport.attempts.len(), 1);

        let error = pending
            .finish(20, &mut store, &protocol, &mut sender, &mut transport)
            .expect_err("the first of two send failures must be returned");

        let PeriodicError::Transport(error) = error else {
            panic!("tap send failure must retain its transport error");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(store.snapshot(), released);
        assert_eq!(sender.timer(), 0);
        assert_eq!(transport.attempts.len(), 2);
        assert!(transport.failures.is_empty());
        assert_eq!(
            transport
                .attempts
                .iter()
                .map(|report| (report[1], [report[3], report[4], report[5]]))
                .collect::<Vec<_>>(),
            [(0, [0x0C, 0x00, 0x80]), (0, [0x00, 0x00, 0x80])]
        );
    }

    struct Harness {
        policy: PeriodicPolicy,
        protocol: SwitchHidProtocol<Pro>,
        sender: ReportSender<Pro>,
        store: InputStateStore<Pro>,
        observed: ObservedSubcommands,
        transport: FakeTransport,
        control: FakeTransportControl,
    }

    impl Harness {
        fn new() -> Self {
            let (mut transport, control) = FakeTransport::with_limits(8, 8);
            let (notifier, _wake_receiver) = activity_channel();
            transport.open(notifier).expect("open fake transport");
            let mut policy = PeriodicPolicy::new(REPORT_PERIOD).expect("valid periodic period");
            assert_eq!(
                policy
                    .start_when_unheld(Duration::ZERO)
                    .expect("start periodic schedule"),
                PeriodicStart::Started {
                    first_deadline: REPORT_PERIOD,
                }
            );
            Self {
                policy,
                protocol: SwitchHidProtocol::new(
                    None,
                    DeviceInfoBluetoothAddress::from_wire_bytes(DEVICE_INFO_ADDRESS),
                ),
                sender: ReportSender::new(),
                store: InputStateStore::new(),
                observed: ObservedSubcommands::default(),
                transport,
                control,
            }
        }

        fn handle_subcommand(
            &mut self,
            completed_at: Duration,
            subcommand_id: u8,
            payload: &[u8],
        ) -> Result<OutputHandling, OutputHandlingError> {
            let raw = subcommand_report(subcommand_id, payload);
            let current = self.store.snapshot();
            let mut ignore_output = |_| {};
            let completion = handle_output(
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
            );
            self.policy
                .record_output_completion(completed_at, &completion)
                .expect("holdoff deadline remains representable");
            completion
        }

        fn send_due(&mut self, now: Duration) -> crate::runtime::periodic::PeriodicResult {
            self.policy.send_due(
                now,
                &self.store,
                &self.protocol,
                &mut self.sender,
                &mut self.transport,
            )
        }
    }

    fn pressed_state(kind: ButtonKind) -> InputState<Pro> {
        let button = ProButton::try_from(kind).expect("button supported by Pro Controller");
        InputState::neutral().with_buttons([button])
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

    struct FailingTransport {
        failures: VecDeque<TransportErrorKind>,
        attempts: Vec<Box<[u8]>>,
    }

    impl FailingTransport {
        fn new(failures: impl IntoIterator<Item = TransportErrorKind>) -> Self {
            Self {
                failures: failures.into_iter().collect(),
                attempts: Vec::new(),
            }
        }
    }

    impl TransportPort for FailingTransport {
        fn open(&mut self, _activity: ActivityNotifier) -> TransportResult<()> {
            Ok(())
        }

        fn poll(&mut self, _timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            Ok(Vec::new())
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
            self.attempts.push(Box::from(payload));
            let kind = self
                .failures
                .pop_front()
                .expect("test must script every send failure");
            Err(TransportError::new(kind))
        }

        fn disconnect(&mut self) -> TransportResult<()> {
            Ok(())
        }

        fn close(&mut self) -> TransportResult<()> {
            Ok(())
        }
    }
}
