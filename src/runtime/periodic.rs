#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T11 defines periodic policy before T21 worker integration"
    )
)]

use std::{error::Error as StdError, fmt, time::Duration};

use crate::{
    input::InputState,
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    runtime::{
        output::{OutputHandling, OutputHandlingError},
        scheduler::{ReportScheduler, SchedulerError, TickDecision},
        sender::ReportSender,
        state::InputStateStore,
        transport::{SendAcceptance, TransportError, TransportPort},
    },
};

const REPLY_HOLDOFF: Duration = Duration::from_millis(300);

pub(crate) fn commit_candidate<M: ControllerModel>(
    candidate: InputState<M>,
    state: &mut InputStateStore<M>,
) {
    state.commit(candidate);
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
            Self::Scheduler(error) => write!(formatter, "periodic scheduler error: {error}"),
            Self::ClockOverflow => formatter.write_str("monotonic clock exceeds nanosecond range"),
            Self::Transport(error) => write!(formatter, "periodic transport error: {error}"),
        }
    }
}

impl StdError for PeriodicError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Scheduler(error) => Some(error),
            Self::ClockOverflow => None,
            Self::Transport(error) => Some(error),
        }
    }
}

pub(crate) type PeriodicResult = Result<AutomaticInput, PeriodicError>;

pub(crate) struct PeriodicPolicy {
    scheduler: ReportScheduler,
    reply_holdoff_until: Option<Duration>,
}

impl PeriodicPolicy {
    pub(crate) fn start(started_at: Duration, period: Duration) -> Result<Self, SchedulerError> {
        Ok(Self {
            scheduler: ReportScheduler::start(started_at, period)?,
            reply_holdoff_until: None,
        })
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

    #[must_use]
    pub(crate) fn next_deadline(&self) -> Duration {
        self.reply_holdoff_until
            .map_or(self.scheduler.next_deadline(), |holdoff| {
                holdoff.max(self.scheduler.next_deadline())
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
        if let Some(until) = self.reply_holdoff_until {
            if now < until {
                return Ok(AutomaticInput::HeldOff { until });
            }
        }

        let TickDecision::Due { skipped } = self.scheduler.step(now)? else {
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
    use std::time::Duration;

    use crate::{
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::{DeviceInfoBluetoothAddress, SwitchHidProtocol},
        runtime::{
            connection::ObservedSubcommands,
            output::{OutputHandling, OutputHandlingContext, OutputHandlingError, handle_output},
            periodic::{AutomaticInput, PeriodicPolicy},
            sender::ReportSender,
            state::InputStateStore,
            transport::{
                HidChannel, TransportErrorKind, TransportPort, activity_channel,
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
        assert_eq!(harness.policy.next_deadline(), REPLY_HOLDOFF);

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
        assert_eq!(harness.policy.next_deadline(), Duration::from_millis(400));
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
        assert_eq!(harness.policy.next_deadline(), REPORT_PERIOD);

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
        assert_eq!(harness.policy.next_deadline(), Duration::from_millis(500));
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
            Self {
                policy: PeriodicPolicy::start(Duration::ZERO, REPORT_PERIOD)
                    .expect("valid periodic schedule"),
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
}
