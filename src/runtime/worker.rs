use std::{
    convert::Infallible,
    marker::PhantomData,
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::Duration,
};

use crate::{
    controller::input::{press_candidate, release_candidate, tap_plan},
    diagnostics::{LifecycleState, event::WorkerFailureCategory},
    error::Error,
    input::{Button, InputState},
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    reporting::{self, Direct, Periodic, ReportingMode},
    runtime::{
        cleanup::{
            CleanupContext, CleanupFailure, CleanupSequence, CloseCompletion, CloseMode,
            ExplicitCloseError,
        },
        clock::{deadline_after, protocol_timestamp},
        connection::ObservedSubcommands,
        direct::{
            DirectTapContext, DirectTapError, DirectTapStep, DirectTapStimulus, PendingDirectTap,
            begin_tap as begin_direct_tap, send_candidate as send_direct,
        },
        handshake::{Handshake, HandshakeError, HandshakeProgress},
        lifecycle::{LifecycleCommandError, LifecycleStateMachine},
        output::{
            OutputHandling, OutputHandlingContext, OutputHandlingError, OutputObservation,
            handle_output,
        },
        periodic::{
            AutomaticInput, PendingPeriodicTap, PeriodicError, PeriodicPolicy,
            begin_tap as begin_periodic_tap,
        },
        readiness::{ReadinessError, ReadinessGate, ReadinessProgress},
        scheduler::SchedulerError,
        sender::ReportSender,
        session::{ConnectionSessionId, ConnectionSessions, SessionEvent},
        state::InputStateStore,
        status::StatusPublisher,
        transport::{TransportError, TransportErrorKind, TransportEvent, TransportPort},
    },
};

#[cfg(test)]
use crate::runtime::status::status_projection;

const EXPLICIT_CLOSE_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) trait MonotonicClock: Send {
    fn now(&self) -> Duration;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerWaitRequest {
    Activity,
    ActivityOrDeadline(Duration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerWaitError {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not run the channel waiter"
        )
    )]
    Disconnected,
}

pub(crate) trait WorkerWaiter: Send {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError>;
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not run the channel waiter"
    )
)]
pub(crate) struct ChannelWorkerWaiter {
    receiver: Receiver<()>,
}

impl ChannelWorkerWaiter {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not construct the channel waiter"
        )
    )]
    pub(crate) const fn new(receiver: Receiver<()>) -> Self {
        Self { receiver }
    }
}

impl WorkerWaiter for ChannelWorkerWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError> {
        match request {
            WorkerWaitRequest::Activity => self
                .receiver
                .recv()
                .map_err(|_| WorkerWaitError::Disconnected),
            WorkerWaitRequest::ActivityOrDeadline(deadline) => {
                let now = clock.now();
                if deadline <= now {
                    return Ok(());
                }
                match self.receiver.recv_timeout(deadline - now) {
                    Ok(()) | Err(RecvTimeoutError::Timeout) => Ok(()),
                    Err(RecvTimeoutError::Disconnected) => Err(WorkerWaitError::Disconnected),
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShutdownRequest {
    Explicit(CloseMode),
    Dropped,
}

impl ShutdownRequest {
    pub(crate) const fn explicit(mode: CloseMode) -> Self {
        Self::Explicit(mode)
    }

    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not own a runtime worker"
        )
    )]
    pub(crate) const fn dropped() -> Self {
        Self::Dropped
    }
}

pub(crate) trait PriorityShutdown: Send {
    fn take(&mut self) -> Option<ShutdownRequest>;
}

pub(crate) trait CommandSource<C> {
    fn try_next(&mut self) -> Option<C>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WorkerBudget {
    poll_batches: usize,
}

impl WorkerBudget {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not construct controller workers"
        )
    )]
    pub(crate) const fn new(poll_batches: usize) -> Self {
        assert!(poll_batches > 0, "worker poll batch count must be positive");
        Self { poll_batches }
    }
}

pub enum CommonCommand<M: ControllerModel> {
    Press(Vec<Button<M>>),
    Release(Vec<Button<M>>),
    Tap {
        buttons: Vec<Button<M>>,
        duration: Duration,
    },
    Neutral,
}

pub enum PeriodicCommand<M: ControllerModel> {
    Common(CommonCommand<M>),
    Apply(InputState<M>),
}

pub enum DirectCommand<M: ControllerModel> {
    Common(CommonCommand<M>),
    Send(InputState<M>),
}

pub(crate) enum RuntimeCommand<M, R>
where
    M: ControllerModel,
    R: ReportingMode,
{
    Pair { timeout: Duration },
    Reconnect { timeout: Duration },
    Input(<R as reporting::sealed::Sealed>::Command<M>),
}

#[cfg(test)]
struct InputCommandSource<'a, M, R>
where
    M: ControllerModel,
    R: ReportingMode,
{
    commands: &'a mut dyn CommandSource<<R as reporting::sealed::Sealed>::Command<M>>,
    _types: PhantomData<fn() -> (M, R)>,
}

#[cfg(test)]
impl<M, R> CommandSource<RuntimeCommand<M, R>> for InputCommandSource<'_, M, R>
where
    M: ControllerModel,
    R: ReportingMode,
{
    fn try_next(&mut self) -> Option<RuntimeCommand<M, R>> {
        self.commands.try_next().map(RuntimeCommand::Input)
    }
}

#[derive(Debug)]
pub(crate) enum WorkerCommandError {
    Input(Error),
    Lifecycle(LifecycleCommandError),
    Pair(PairingError),
    Reconnect(ReconnectError),
    Periodic(PeriodicError),
    Direct(DirectTapError),
    Shutdown,
    Disconnected,
}

#[derive(Debug)]
pub(crate) enum WorkerCommandProgress {
    Complete(Result<(), WorkerCommandError>),
    Pending,
}

#[derive(Debug)]
pub(crate) enum WorkerOperationError {
    Output(OutputHandlingError),
    Periodic(PeriodicError),
    Readiness,
}

#[derive(Debug)]
pub(crate) enum WorkerCoreError {
    InvalidLifecycle,
    Handshake(HandshakeError),
    Transport(TransportError),
}

#[derive(Debug)]
pub(crate) enum PairingError {
    Begin(WorkerCoreError),
    Readiness(ReadinessError),
    InvalidKeyStore,
    WorkerFailed,
}

#[derive(Debug)]
pub(crate) enum ReconnectError {
    Begin(WorkerCoreError),
    Readiness(ReadinessError),
    InvalidKeyStore,
    WorkerFailed,
}

#[derive(Clone, Copy)]
enum ConnectionCommandKind {
    Pair,
    Reconnect,
}

#[derive(Clone, Copy)]
enum ConnectionAttemptFailure {
    Readiness(ReadinessError),
    InvalidKeyStore,
    WorkerFailed,
}

impl WorkerCoreError {
    pub(crate) fn status_message(&self) -> &'static str {
        match self {
            Self::InvalidLifecycle => "worker lifecycle invariant failed",
            Self::Handshake(_) => "worker handshake failed",
            Self::Transport(error) if error.kind() == TransportErrorKind::InvalidKeyStore => {
                "worker pairing key store failed"
            }
            Self::Transport(_) => "worker transport failed",
        }
    }
}

pub(crate) struct StepProgress {
    commands: usize,
    hci_events: usize,
    due_actions: usize,
    skipped_deadlines: u64,
    immediate: bool,
    next_deadline: Option<Duration>,
    command_result: Option<Result<(), WorkerCommandError>>,
    operation_errors: Vec<WorkerOperationError>,
}

impl StepProgress {
    fn new() -> Self {
        Self {
            commands: 0,
            hci_events: 0,
            due_actions: 0,
            skipped_deadlines: 0,
            immediate: false,
            next_deadline: None,
            command_result: None,
            operation_errors: Vec::new(),
        }
    }

    pub(crate) fn wait_request(&self, now: Duration) -> Option<WorkerWaitRequest> {
        if self.immediate || self.next_deadline.is_some_and(|deadline| deadline <= now) {
            return None;
        }
        Some(self.next_deadline.map_or(
            WorkerWaitRequest::Activity,
            WorkerWaitRequest::ActivityOrDeadline,
        ))
    }

    pub(crate) fn take_command_result(&mut self) -> Option<Result<(), WorkerCommandError>> {
        self.command_result.take()
    }

    fn record_command_progress(&mut self, progress: WorkerCommandProgress) {
        let WorkerCommandProgress::Complete(result) = progress else {
            return;
        };
        assert!(
            self.command_result.replace(result).is_none(),
            "worker step produced multiple command results"
        );
    }
}

pub(crate) fn wait_for_next_iteration(
    progress: &StepProgress,
    clock: &dyn MonotonicClock,
    waiter: &mut dyn WorkerWaiter,
) -> Result<(), WorkerWaitError> {
    let Some(request) = progress.wait_request(clock.now()) else {
        return Ok(());
    };
    waiter.wait(request, clock)
}

pub(crate) enum WorkerStep {
    Continue(StepProgress),
    Closed {
        completion: CloseCompletion,
        interrupted: Option<WorkerCommandError>,
        progress: StepProgress,
    },
    Failed {
        error: WorkerCoreError,
        progress: StepProgress,
    },
}

struct ConnectionWork {
    session_id: ConnectionSessionId,
    handshake: Option<Handshake>,
    readiness: ReadinessGate,
}

pub(crate) struct PeriodicRuntime<M: ControllerModel> {
    policy: PeriodicPolicy,
    pending_tap: Option<ScheduledPeriodicTap<M>>,
}

struct ScheduledPeriodicTap<M: ControllerModel> {
    tap: PendingPeriodicTap<M>,
    release_at: Duration,
}

pub(crate) struct DirectRuntime<M: ControllerModel> {
    pending_tap: Option<PendingDirectTap<M>>,
}

pub(crate) trait WorkerReporting<M: ControllerModel>: ReportingMode {
    type RuntimeState: Send + 'static;

    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not construct concrete workers"
        )
    )]
    fn build_worker(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        config: &<Self as reporting::sealed::Sealed>::Config,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
        status: StatusPublisher<M>,
    ) -> WorkerCore<M, Self>
    where
        Self: Sized;

    fn begin_session(
        runtime: &mut Self::RuntimeState,
        sessions: &mut ConnectionSessions,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> ConnectionSessionId;

    fn begin_handshake(session_id: ConnectionSessionId) -> Handshake;

    fn handle_command(
        runtime: &mut Self::RuntimeState,
        command: <Self as reporting::sealed::Sealed>::Command<M>,
        context: ReportingCommandContext<'_, M>,
    ) -> WorkerCommandProgress;

    fn handle_event(
        runtime: &mut Self::RuntimeState,
        event: TransportEvent,
        context: ReportingEventContext<'_, M>,
    ) -> ReportingEvent;

    fn record_output(
        runtime: &mut Self::RuntimeState,
        completed_at: Duration,
        completion: &Result<OutputHandling, OutputHandlingError>,
    ) -> Result<(), WorkerOperationError>;

    fn evaluate_readiness(
        runtime: &mut Self::RuntimeState,
        now: Duration,
        sessions: &ConnectionSessions,
        handshake: &mut Option<Handshake>,
        sender: &ReportSender<M>,
        readiness: &mut ReadinessGate,
    ) -> Result<ReadinessProgress, ReadinessError>;

    fn next_deadline(runtime: &Self::RuntimeState) -> Option<Duration>;

    fn automatic_due_before_ready(runtime: &Self::RuntimeState) -> bool;

    fn run_due(
        runtime: &mut Self::RuntimeState,
        now: Duration,
        context: ReportingDueContext<'_, M>,
    ) -> ReportingDue;

    fn cancel_for_shutdown(
        runtime: &mut Self::RuntimeState,
        context: ReportingEventContext<'_, M>,
    ) -> Option<WorkerCommandError>;

    fn stop_session(runtime: &mut Self::RuntimeState);

    fn has_pending(runtime: &Self::RuntimeState) -> bool;
}

pub(crate) struct ReportingCommandContext<'a, M: ControllerModel> {
    ready: bool,
    now: Duration,
    input: &'a mut InputStateStore<M>,
    protocol: &'a SwitchHidProtocol<M>,
    sender: &'a mut ReportSender<M>,
    transport: &'a mut dyn TransportPort,
}

pub(crate) struct ReportingEventContext<'a, M: ControllerModel> {
    observe_output: &'a mut dyn FnMut(OutputObservation),
    protocol: &'a SwitchHidProtocol<M>,
    input: &'a mut InputStateStore<M>,
    observed: &'a mut ObservedSubcommands,
    sender: &'a mut ReportSender<M>,
    status: Option<&'a StatusPublisher<M>>,
    transport: &'a mut dyn TransportPort,
}

pub(crate) struct ReportingDueContext<'a, M: ControllerModel> {
    input: &'a mut InputStateStore<M>,
    protocol: &'a SwitchHidProtocol<M>,
    sender: &'a mut ReportSender<M>,
    transport: &'a mut dyn TransportPort,
}

pub(crate) enum ReportingEvent {
    Passthrough(TransportEvent),
    Output(Result<OutputHandling, OutputHandlingError>),
    Disconnected {
        reason: Option<u8>,
        completion: Option<WorkerCommandError>,
    },
}

pub(crate) struct ReportingDue {
    actions: usize,
    immediate: bool,
    completion: Option<WorkerCommandProgress>,
    errors: Vec<WorkerOperationError>,
}

impl ReportingDue {
    fn none() -> Self {
        Self {
            actions: 0,
            immediate: false,
            completion: None,
            errors: Vec::new(),
        }
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(dead_code, reason = "feature-disabled builds do not own a worker core")
)]
pub(crate) struct WorkerCore<M, R>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    lifecycle: LifecycleStateMachine,
    input: InputStateStore<M>,
    reporting: R::RuntimeState,
    sender: ReportSender<M>,
    protocol: SwitchHidProtocol<M>,
    observed: ObservedSubcommands,
    sessions: ConnectionSessions,
    connection: Option<ConnectionWork>,
    connection_command_pending: Option<ConnectionCommandKind>,
    connected: bool,
    status: StatusPublisher<M>,
    transport: Box<dyn TransportPort>,
    budget: WorkerBudget,
    observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
    reporting_marker: PhantomData<fn() -> R>,
}

impl<M: ControllerModel> WorkerCore<M, Periodic> {
    #[cfg(test)]
    pub(crate) fn new_periodic(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        period: Duration,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
    ) -> Result<Self, SchedulerError> {
        let (status, _reader) = status_projection::<M, Periodic>();
        Self::new_periodic_with_status(protocol, transport, period, budget, observe_output, status)
    }

    pub(crate) fn new_periodic_with_status(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        period: Duration,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
        status: StatusPublisher<M>,
    ) -> Result<Self, SchedulerError> {
        let reporting = PeriodicRuntime {
            policy: PeriodicPolicy::new(period)?,
            pending_tap: None,
        };
        Ok(Self::from_open_transport(
            protocol,
            transport,
            reporting,
            budget,
            observe_output,
            status,
        ))
    }
}

impl<M: ControllerModel> WorkerCore<M, Direct> {
    #[cfg(test)]
    pub(crate) fn new_direct(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
    ) -> Self {
        let (status, _reader) = status_projection::<M, Direct>();
        Self::new_direct_with_status(protocol, transport, budget, observe_output, status)
    }

    pub(crate) fn new_direct_with_status(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
        status: StatusPublisher<M>,
    ) -> Self {
        Self::from_open_transport(
            protocol,
            transport,
            DirectRuntime { pending_tap: None },
            budget,
            observe_output,
            status,
        )
    }
}

impl<M, R> WorkerCore<M, R>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    fn from_open_transport(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        reporting: R::RuntimeState,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
        status: StatusPublisher<M>,
    ) -> Self {
        let lifecycle = LifecycleStateMachine::new();
        status.set_lifecycle(LifecycleState::Open);
        Self {
            lifecycle,
            input: InputStateStore::with_status(status.clone()),
            reporting,
            sender: ReportSender::with_status(status.clone()),
            protocol,
            observed: ObservedSubcommands::default(),
            sessions: ConnectionSessions::new(),
            connection: None,
            connection_command_pending: None,
            connected: false,
            status,
            transport,
            budget,
            observe_output,
            reporting_marker: PhantomData,
        }
    }

    pub(crate) fn begin_connection(
        &mut self,
        now: Duration,
        timeout: Duration,
    ) -> Result<ConnectionSessionId, WorkerCoreError> {
        let operation_deadline = deadline_after(now, timeout);
        if !self.lifecycle.begin_connection() {
            return Err(WorkerCoreError::InvalidLifecycle);
        }
        let session_id = R::begin_session(
            &mut self.reporting,
            &mut self.sessions,
            &mut self.sender,
            &mut self.observed,
            &mut self.input,
        );
        self.connection = Some(ConnectionWork {
            session_id,
            handshake: Some(R::begin_handshake(session_id)),
            readiness: ReadinessGate::new(session_id, operation_deadline),
        });
        self.connected = false;
        let snapshot = self.input.snapshot();
        self.status
            .begin_session(session_id.non_zero(), self.lifecycle.state(), &snapshot);
        Ok(session_id)
    }

    #[cfg(test)]
    pub(crate) fn step(
        &mut self,
        clock: &dyn MonotonicClock,
        shutdown: &mut dyn PriorityShutdown,
        commands: &mut dyn CommandSource<<R as reporting::sealed::Sealed>::Command<M>>,
    ) -> WorkerStep {
        let mut commands = InputCommandSource::<M, R> {
            commands,
            _types: PhantomData,
        };
        self.step_runtime(clock, shutdown, &mut commands)
    }

    pub(crate) fn step_runtime(
        &mut self,
        clock: &dyn MonotonicClock,
        shutdown: &mut dyn PriorityShutdown,
        commands: &mut dyn CommandSource<RuntimeCommand<M, R>>,
    ) -> WorkerStep {
        let mut progress = StepProgress::new();
        if let Some(request) = shutdown.take() {
            return self.close(request, clock.now(), progress);
        }

        if self.connection_command_pending.is_none() && !R::has_pending(&self.reporting) {
            if let Some(command) = commands.try_next() {
                progress.commands += 1;
                let result = match command {
                    RuntimeCommand::Pair { timeout } => {
                        match self
                            .transport
                            .start_pairing()
                            .map_err(WorkerCoreError::Transport)
                            .and_then(|()| self.begin_connection(clock.now(), timeout))
                        {
                            Ok(_) => {
                                self.connection_command_pending = Some(ConnectionCommandKind::Pair);
                                WorkerCommandProgress::Pending
                            }
                            Err(error) => WorkerCommandProgress::Complete(Err(
                                WorkerCommandError::Pair(PairingError::Begin(error)),
                            )),
                        }
                    }
                    RuntimeCommand::Reconnect { timeout } => {
                        match self
                            .transport
                            .start_reconnect()
                            .map_err(WorkerCoreError::Transport)
                            .and_then(|()| self.begin_connection(clock.now(), timeout))
                        {
                            Ok(_) => {
                                self.connection_command_pending =
                                    Some(ConnectionCommandKind::Reconnect);
                                WorkerCommandProgress::Pending
                            }
                            Err(error) => WorkerCommandProgress::Complete(Err(
                                WorkerCommandError::Reconnect(ReconnectError::Begin(error)),
                            )),
                        }
                    }
                    RuntimeCommand::Input(command) => match self.lifecycle.ensure_input_command() {
                        Ok(()) => R::handle_command(
                            &mut self.reporting,
                            command,
                            ReportingCommandContext {
                                ready: self.lifecycle.state() == LifecycleState::Ready,
                                now: clock.now(),
                                input: &mut self.input,
                                protocol: &self.protocol,
                                sender: &mut self.sender,
                                transport: self.transport.as_mut(),
                            },
                        ),
                        Err(error) => WorkerCommandProgress::Complete(Err(
                            WorkerCommandError::Lifecycle(error),
                        )),
                    },
                };
                let (result, terminal) = match nonterminal_command_progress(result) {
                    Ok(result) => (result, None),
                    Err(termination) => (termination.completion, Some(termination.error)),
                };
                progress.record_command_progress(result);
                if let Some(request) = shutdown.take() {
                    return self.close(request, clock.now(), progress);
                }
                if let Some(error) = terminal {
                    return self.fail(error, progress);
                }
            }
        }
        if progress.commands == 1
            && self.connection_command_pending.is_none()
            && !R::has_pending(&self.reporting)
        {
            progress.immediate = true;
        }

        for batch_index in 0..self.budget.poll_batches {
            let events = match self.transport.poll(Duration::ZERO) {
                Ok(events) => events,
                Err(error) => {
                    if let Some(request) = shutdown.take() {
                        return self.close(request, clock.now(), progress);
                    }
                    return self.fail(WorkerCoreError::Transport(error), progress);
                }
            };
            if events.is_empty() {
                break;
            }
            let events = events
                .into_iter()
                .map(|event| self.sessions.tag_current(event))
                .collect::<Vec<_>>();
            let final_budgeted_batch = batch_index + 1 == self.budget.poll_batches;
            for event in events {
                progress.hci_events += 1;
                let topology_changed = match event {
                    Some(event) => match self.handle_event(event, clock, &mut progress) {
                        Ok(topology_changed) => topology_changed,
                        Err(error) => {
                            if let Some(request) = shutdown.take() {
                                return self.close(request, clock.now(), progress);
                            }
                            return self.fail(error, progress);
                        }
                    },
                    None => false,
                };
                if let Some(request) = shutdown.take() {
                    return self.close(request, clock.now(), progress);
                }
                if topology_changed {
                    match self.drive_connection(clock.now(), &mut progress) {
                        Ok(actions) => progress.due_actions += actions,
                        Err(error) => {
                            if let Some(request) = shutdown.take() {
                                return self.close(request, clock.now(), progress);
                            }
                            return self.fail(error, progress);
                        }
                    }
                    if let Some(request) = shutdown.take() {
                        return self.close(request, clock.now(), progress);
                    }
                }
            }
            if final_budgeted_batch {
                progress.immediate = true;
            }
        }

        match self.drive_connection(clock.now(), &mut progress) {
            Ok(actions) => progress.due_actions += actions,
            Err(error) => {
                if let Some(request) = shutdown.take() {
                    return self.close(request, clock.now(), progress);
                }
                return self.fail(error, progress);
            }
        }
        if let Some(request) = shutdown.take() {
            return self.close(request, clock.now(), progress);
        }

        if self.lifecycle.state() == LifecycleState::Ready
            || (self.lifecycle.state() == LifecycleState::Connecting
                && R::automatic_due_before_ready(&self.reporting))
        {
            let due = R::run_due(
                &mut self.reporting,
                clock.now(),
                ReportingDueContext {
                    input: &mut self.input,
                    protocol: &self.protocol,
                    sender: &mut self.sender,
                    transport: self.transport.as_mut(),
                },
            );
            progress.due_actions += due.actions;
            progress.immediate |= due.immediate;
            let mut terminal = None;
            if let Some(completion) = due.completion {
                match nonterminal_command_progress(completion) {
                    Ok(completion) => progress.record_command_progress(completion),
                    Err(termination) => {
                        progress.record_command_progress(termination.completion);
                        terminal.get_or_insert(termination.error);
                    }
                }
            }
            for error in due.errors {
                match nonterminal_operation_error(error) {
                    Ok(error) => progress.operation_errors.push(error),
                    Err(termination) => {
                        progress.operation_errors.push(termination.completion);
                        terminal.get_or_insert(termination.error);
                    }
                }
            }
            if let Some(request) = shutdown.take() {
                return self.close(request, clock.now(), progress);
            }
            if let Some(error) = terminal {
                return self.fail(error, progress);
            }
        }
        progress.next_deadline = minimum_deadline(progress.next_deadline, self.next_deadline());
        WorkerStep::Continue(progress)
    }

    fn handle_event(
        &mut self,
        event: SessionEvent,
        clock: &dyn MonotonicClock,
        progress: &mut StepProgress,
    ) -> Result<bool, WorkerCoreError> {
        let Some(event) = self.sessions.take_current(event) else {
            return Ok(false);
        };
        let event = R::handle_event(
            &mut self.reporting,
            event,
            ReportingEventContext {
                observe_output: self.observe_output.as_mut(),
                protocol: &self.protocol,
                input: &mut self.input,
                observed: &mut self.observed,
                sender: &mut self.sender,
                status: Some(&self.status),
                transport: self.transport.as_mut(),
            },
        );
        match event {
            ReportingEvent::Passthrough(TransportEvent::Connected) => {
                self.connected = true;
                self.status.set_connected(true);
                if let Some(connection) = self.connection.as_mut() {
                    if let Some(handshake) = connection.handshake.as_mut() {
                        handshake.observe_link(connection.session_id);
                    }
                }
                Ok(true)
            }
            ReportingEvent::Passthrough(TransportEvent::HidChannelOpened { channel }) => {
                if let Some(connection) = self.connection.as_mut() {
                    if let Some(handshake) = connection.handshake.as_mut() {
                        handshake.observe_channel(connection.session_id, channel);
                    }
                }
                Ok(true)
            }
            ReportingEvent::Passthrough(TransportEvent::HidOutput { channel, payload }) => {
                let completion = self.handle_output(channel, &payload);
                self.record_output(clock.now(), completion, progress)?;
                Ok(false)
            }
            ReportingEvent::Passthrough(TransportEvent::Disconnected { reason })
            | ReportingEvent::Disconnected {
                reason,
                completion: None,
            } => {
                self.end_connection(reason, None, progress);
                Ok(false)
            }
            ReportingEvent::Disconnected { reason, completion } => {
                self.end_connection(reason, completion, progress);
                Ok(false)
            }
            ReportingEvent::Output(completion) => {
                self.record_output(clock.now(), completion, progress)?;
                Ok(false)
            }
        }
    }

    fn handle_output(
        &mut self,
        channel: crate::runtime::transport::HidChannel,
        payload: &[u8],
    ) -> Result<OutputHandling, OutputHandlingError> {
        let current = self.input.snapshot();
        handle_output(
            channel,
            payload,
            OutputHandlingContext {
                observe_output: self.observe_output.as_mut(),
                protocol: &self.protocol,
                current: &current,
                observed: &mut self.observed,
                sender: &mut self.sender,
                status: Some(&self.status),
                transport: self.transport.as_mut(),
            },
        )
    }

    fn record_output(
        &mut self,
        completed_at: Duration,
        completion: Result<OutputHandling, OutputHandlingError>,
        progress: &mut StepProgress,
    ) -> Result<(), WorkerCoreError> {
        if let Err(error) = R::record_output(&mut self.reporting, completed_at, &completion) {
            record_operation_error(progress, error)?;
        }
        if let Err(error) = completion {
            record_operation_error(progress, WorkerOperationError::Output(error))?;
        }
        Ok(())
    }

    fn end_connection(
        &mut self,
        reason: Option<u8>,
        completion: Option<WorkerCommandError>,
        progress: &mut StepProgress,
    ) {
        if let Some(error) = completion {
            progress.record_command_progress(WorkerCommandProgress::Complete(Err(error)));
            progress.immediate = true;
        }
        if let Some(mut connection) = self.connection.take() {
            let error = connection.readiness.abort(
                &mut connection.handshake,
                ReadinessError::Disconnected { reason },
            );
            self.complete_connection_failure(ConnectionAttemptFailure::Readiness(error), progress);
            progress
                .operation_errors
                .push(WorkerOperationError::Readiness);
        }
        if let Some(session_id) = self.sessions.current() {
            self.sessions.end_current(session_id);
        }
        self.connected = false;
        R::stop_session(&mut self.reporting);
        self.lifecycle.mark_connection_ended();
        self.status.end_session(self.lifecycle.state(), reason);
    }

    fn drive_connection(
        &mut self,
        now: Duration,
        progress: &mut StepProgress,
    ) -> Result<usize, WorkerCoreError> {
        let Some(mut connection) = self.connection.take() else {
            return Ok(0);
        };
        let mut actions = 0;
        if let Err(error) = connection.readiness.ensure_active(now, &self.sessions) {
            self.abort_connection(connection, error, progress);
            return Ok(actions);
        }
        if let Some(handshake) = connection.handshake.as_mut() {
            let handshake_progress = match handshake.step(
                connection.session_id,
                now,
                &self.observed,
                &self.protocol,
                &mut self.sender,
                self.transport.as_mut(),
            ) {
                Ok(handshake_progress) => handshake_progress,
                Err(error) => {
                    self.connection = Some(connection);
                    return Err(WorkerCoreError::Handshake(error));
                }
            };
            match handshake_progress {
                HandshakeProgress::WaitingUntil { deadline } => {
                    progress.next_deadline =
                        minimum_deadline(progress.next_deadline, Some(deadline));
                }
                HandshakeProgress::BootstrapAttempted { result, skipped } => {
                    actions += 1;
                    progress.skipped_deadlines = progress.skipped_deadlines.saturating_add(skipped);
                    if let Err(error) = result {
                        if error.kind() == TransportErrorKind::SendRejected {
                            progress.operation_errors.push(WorkerOperationError::Output(
                                OutputHandlingError::Transport(error),
                            ));
                        } else {
                            self.connection = Some(connection);
                            return Err(WorkerCoreError::Transport(error));
                        }
                    }
                }
                HandshakeProgress::StaleSession
                | HandshakeProgress::WaitingForTopology
                | HandshakeProgress::SubcommandObserved => {}
            }
        }

        match R::evaluate_readiness(
            &mut self.reporting,
            now,
            &self.sessions,
            &mut connection.handshake,
            &self.sender,
            &mut connection.readiness,
        ) {
            Ok(ReadinessProgress::Pending(_)) => {
                self.connection = Some(connection);
            }
            Ok(ReadinessProgress::Ready(ready)) => {
                if ready != connection.session_id || !self.lifecycle.mark_ready() {
                    self.connection = Some(connection);
                    return Err(WorkerCoreError::InvalidLifecycle);
                }
                self.status.set_lifecycle(self.lifecycle.state());
                self.complete_connection_command(Ok(()), progress);
                actions += 1;
            }
            Err(error) => {
                self.abort_connection(connection, error, progress);
            }
        }
        Ok(actions)
    }

    fn abort_connection(
        &mut self,
        mut connection: ConnectionWork,
        error: ReadinessError,
        progress: &mut StepProgress,
    ) {
        let error = connection.readiness.abort(&mut connection.handshake, error);
        self.complete_connection_failure(ConnectionAttemptFailure::Readiness(error), progress);
        progress
            .operation_errors
            .push(WorkerOperationError::Readiness);
        if let Some(session_id) = self.sessions.current() {
            self.sessions.end_current(session_id);
        }
        self.connected = false;
        R::stop_session(&mut self.reporting);
        self.lifecycle.mark_connection_ended();
        self.status.end_session(self.lifecycle.state(), None);
    }

    fn close(
        &mut self,
        request: ShutdownRequest,
        now: Duration,
        mut progress: StepProgress,
    ) -> WorkerStep {
        self.complete_connection_command(Err(WorkerCommandError::Shutdown), &mut progress);
        let interrupted = R::cancel_for_shutdown(
            &mut self.reporting,
            ReportingEventContext {
                observe_output: self.observe_output.as_mut(),
                protocol: &self.protocol,
                input: &mut self.input,
                observed: &mut self.observed,
                sender: &mut self.sender,
                status: Some(&self.status),
                transport: self.transport.as_mut(),
            },
        );
        R::stop_session(&mut self.reporting);
        let now_ns = protocol_timestamp(now);
        let cleanup = match request {
            ShutdownRequest::Explicit(mode) => {
                CleanupSequence::new(mode, EXPLICIT_CLOSE_DRAIN_TIMEOUT)
            }
            ShutdownRequest::Dropped => CleanupSequence::for_drop(),
        };
        let completion = cleanup.run(CleanupContext {
            connected: self.connected,
            now_ns,
            lifecycle: &mut self.lifecycle,
            protocol: &self.protocol,
            sender: &mut self.sender,
            status: Some(&self.status),
            transport: self.transport.as_mut(),
        });
        self.connected = false;
        self.connection = None;
        if let Some(session_id) = self.sessions.current() {
            self.sessions.end_current(session_id);
        }
        self.status.close(LifecycleState::Closed);
        WorkerStep::Closed {
            completion,
            interrupted,
            progress,
        }
    }

    pub(crate) fn cleanup_unspawned_without_neutral(
        &mut self,
        now: Duration,
    ) -> Result<(), CleanupFailure> {
        let WorkerStep::Closed { completion, .. } = self.close(
            ShutdownRequest::explicit(CloseMode::WithoutNeutral),
            now,
            StepProgress::new(),
        ) else {
            unreachable!("closing an unspawned worker must produce a closed step");
        };

        finish_cleanup_without_join(completion)
    }

    pub(crate) fn cleanup_after_failure_without_neutral(
        &mut self,
        now: Duration,
    ) -> Result<(), CleanupFailure> {
        R::stop_session(&mut self.reporting);
        let completion =
            CleanupSequence::new(CloseMode::WithoutNeutral, EXPLICIT_CLOSE_DRAIN_TIMEOUT).run(
                CleanupContext {
                    connected: self.connected,
                    now_ns: protocol_timestamp(now),
                    lifecycle: &mut self.lifecycle,
                    protocol: &self.protocol,
                    sender: &mut self.sender,
                    status: None,
                    transport: self.transport.as_mut(),
                },
            );
        self.connected = false;
        self.connection = None;
        if let Some(session_id) = self.sessions.current() {
            self.sessions.end_current(session_id);
        }

        finish_cleanup_without_join(completion)
    }

    fn fail(&mut self, error: WorkerCoreError, mut progress: StepProgress) -> WorkerStep {
        let failure = match &error {
            WorkerCoreError::Transport(source)
                if source.kind() == TransportErrorKind::InvalidKeyStore =>
            {
                ConnectionAttemptFailure::InvalidKeyStore
            }
            _ => ConnectionAttemptFailure::WorkerFailed,
        };
        self.complete_connection_failure(failure, &mut progress);
        self.lifecycle.mark_failed();
        self.connected = false;
        self.status.fail(
            error.status_message(),
            match error {
                WorkerCoreError::Transport(_) => WorkerFailureCategory::Transport,
                _ => WorkerFailureCategory::Internal,
            },
        );
        WorkerStep::Failed { error, progress }
    }

    fn complete_connection_command(
        &mut self,
        result: Result<(), WorkerCommandError>,
        progress: &mut StepProgress,
    ) {
        if self.connection_command_pending.take().is_some() {
            progress.record_command_progress(WorkerCommandProgress::Complete(result));
            progress.immediate = true;
        }
    }

    fn complete_connection_failure(
        &mut self,
        failure: ConnectionAttemptFailure,
        progress: &mut StepProgress,
    ) {
        let error = match self.connection_command_pending {
            Some(ConnectionCommandKind::Pair) => WorkerCommandError::Pair(match failure {
                ConnectionAttemptFailure::Readiness(error) => PairingError::Readiness(error),
                ConnectionAttemptFailure::InvalidKeyStore => PairingError::InvalidKeyStore,
                ConnectionAttemptFailure::WorkerFailed => PairingError::WorkerFailed,
            }),
            Some(ConnectionCommandKind::Reconnect) => {
                WorkerCommandError::Reconnect(match failure {
                    ConnectionAttemptFailure::Readiness(error) => ReconnectError::Readiness(error),
                    ConnectionAttemptFailure::InvalidKeyStore => ReconnectError::InvalidKeyStore,
                    ConnectionAttemptFailure::WorkerFailed => ReconnectError::WorkerFailed,
                })
            }
            None => return,
        };
        self.complete_connection_command(Err(error), progress);
    }

    pub(crate) fn status_publisher(&self) -> StatusPublisher<M> {
        self.status.clone()
    }

    #[must_use]
    fn next_deadline(&self) -> Option<Duration> {
        let mut deadline = R::next_deadline(&self.reporting);
        if let Some(connection) = self.connection.as_ref() {
            deadline = minimum_deadline(deadline, Some(connection.readiness.operation_deadline()));
            deadline = minimum_deadline(
                deadline,
                connection
                    .handshake
                    .as_ref()
                    .and_then(Handshake::next_deadline),
            );
        }
        deadline
    }

    #[cfg(test)]
    fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    #[cfg(test)]
    fn input_snapshot(&self) -> InputState<M> {
        self.input.snapshot()
    }

    #[cfg(test)]
    fn sender_timer(&self) -> u8 {
        self.sender.timer()
    }

    #[cfg(test)]
    fn has_pending_reporting_command(&self) -> bool {
        R::has_pending(&self.reporting)
    }
}

fn finish_cleanup_without_join(completion: CloseCompletion) -> Result<(), CleanupFailure> {
    match completion.finish_with_join(|| Ok::<(), Infallible>(())) {
        Ok(()) => Ok(()),
        Err(ExplicitCloseError::Cleanup(cleanup)) => Err(cleanup),
        Err(ExplicitCloseError::Join(join)) => match join {},
        Err(ExplicitCloseError::CleanupAndJoin { join, .. }) => match join {},
    }
}

impl<M: ControllerModel> WorkerReporting<M> for Periodic {
    type RuntimeState = PeriodicRuntime<M>;

    fn build_worker(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        config: &<Self as reporting::sealed::Sealed>::Config,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
        status: StatusPublisher<M>,
    ) -> WorkerCore<M, Self> {
        WorkerCore::new_periodic_with_status(
            protocol,
            transport,
            config.report_period(),
            budget,
            observe_output,
            status,
        )
        .expect("validated report period is non-zero")
    }

    fn begin_session(
        runtime: &mut Self::RuntimeState,
        sessions: &mut ConnectionSessions,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> ConnectionSessionId {
        sessions.begin_periodic(sender, &mut runtime.policy, observed, input)
    }

    fn begin_handshake(session_id: ConnectionSessionId) -> Handshake {
        Handshake::new(session_id)
    }

    fn handle_command(
        runtime: &mut Self::RuntimeState,
        command: <Self as reporting::sealed::Sealed>::Command<M>,
        context: ReportingCommandContext<'_, M>,
    ) -> WorkerCommandProgress {
        match command {
            PeriodicCommand::Apply(candidate) => {
                context.input.commit(candidate);
                WorkerCommandProgress::Complete(Ok(()))
            }
            PeriodicCommand::Common(CommonCommand::Press(buttons)) => complete_input_candidate(
                press_candidate(&context.input.snapshot(), buttons),
                context.input,
            ),
            PeriodicCommand::Common(CommonCommand::Release(buttons)) => complete_input_candidate(
                release_candidate(&context.input.snapshot(), buttons),
                context.input,
            ),
            PeriodicCommand::Common(CommonCommand::Neutral) => {
                context.input.commit(InputState::neutral());
                WorkerCommandProgress::Complete(Ok(()))
            }
            PeriodicCommand::Common(CommonCommand::Tap { buttons, duration }) => {
                let plan = match tap_plan(&context.input.snapshot(), buttons, duration) {
                    Ok(plan) => plan,
                    Err(error) => {
                        return WorkerCommandProgress::Complete(Err(WorkerCommandError::Input(
                            error,
                        )));
                    }
                };
                let (now_ns, release_at) = periodic_tap_times(context.now, plan.2);
                match begin_periodic_tap(
                    context.ready,
                    plan,
                    now_ns,
                    context.input,
                    context.protocol,
                    context.sender,
                    context.transport,
                ) {
                    Ok(tap) => {
                        runtime.pending_tap = Some(ScheduledPeriodicTap { tap, release_at });
                        WorkerCommandProgress::Pending
                    }
                    Err(error) => {
                        WorkerCommandProgress::Complete(Err(WorkerCommandError::Periodic(error)))
                    }
                }
            }
        }
    }

    fn handle_event(
        runtime: &mut Self::RuntimeState,
        event: TransportEvent,
        _context: ReportingEventContext<'_, M>,
    ) -> ReportingEvent {
        if let TransportEvent::Disconnected { reason } = event {
            let completion = runtime
                .pending_tap
                .take()
                .map(|_| WorkerCommandError::Disconnected);
            ReportingEvent::Disconnected { reason, completion }
        } else {
            ReportingEvent::Passthrough(event)
        }
    }

    fn record_output(
        runtime: &mut Self::RuntimeState,
        completed_at: Duration,
        completion: &Result<OutputHandling, OutputHandlingError>,
    ) -> Result<(), WorkerOperationError> {
        runtime
            .policy
            .record_output_completion(completed_at, completion)
            .map_err(WorkerOperationError::Periodic)
    }

    fn evaluate_readiness(
        runtime: &mut Self::RuntimeState,
        now: Duration,
        sessions: &ConnectionSessions,
        handshake: &mut Option<Handshake>,
        sender: &ReportSender<M>,
        readiness: &mut ReadinessGate,
    ) -> Result<ReadinessProgress, ReadinessError> {
        readiness.evaluate_periodic(now, sessions, handshake, sender, &mut runtime.policy)
    }

    fn next_deadline(runtime: &Self::RuntimeState) -> Option<Duration> {
        let reporting = runtime
            .policy
            .next_deadline()
            .or(runtime.policy.reply_holdoff_until());
        minimum_deadline(
            reporting,
            runtime
                .pending_tap
                .as_ref()
                .map(|pending| pending.release_at),
        )
    }

    fn automatic_due_before_ready(runtime: &Self::RuntimeState) -> bool {
        runtime.policy.next_deadline().is_some()
    }

    fn run_due(
        runtime: &mut Self::RuntimeState,
        now: Duration,
        context: ReportingDueContext<'_, M>,
    ) -> ReportingDue {
        let mut due = ReportingDue::none();
        if runtime
            .pending_tap
            .as_ref()
            .is_some_and(|pending| now >= pending.release_at)
        {
            let pending = runtime
                .pending_tap
                .take()
                .expect("checked pending Periodic tap");
            due.actions += 1;
            due.immediate = true;
            let now_ns = protocol_timestamp(now);
            due.completion = Some(WorkerCommandProgress::Complete(
                pending
                    .tap
                    .finish(
                        now_ns,
                        context.input,
                        context.protocol,
                        context.sender,
                        context.transport,
                    )
                    .map_err(WorkerCommandError::Periodic),
            ));
            if runtime
                .policy
                .next_deadline()
                .is_some_and(|deadline| now >= deadline)
            {
                due.immediate = true;
            }
            return due;
        }

        match runtime.policy.send_due(
            now,
            context.input,
            context.protocol,
            context.sender,
            context.transport,
        ) {
            Ok(AutomaticInput::Sent { .. }) => due.actions += 1,
            Ok(
                AutomaticInput::NotDue
                | AutomaticInput::HeldOff { .. }
                | AutomaticInput::Backpressured { .. },
            ) => {}
            Err(error) => {
                due.actions += 1;
                due.errors.push(WorkerOperationError::Periodic(error));
            }
        }
        due
    }

    fn cancel_for_shutdown(
        runtime: &mut Self::RuntimeState,
        _context: ReportingEventContext<'_, M>,
    ) -> Option<WorkerCommandError> {
        runtime
            .pending_tap
            .take()
            .map(|_| WorkerCommandError::Shutdown)
    }

    fn stop_session(runtime: &mut Self::RuntimeState) {
        runtime.pending_tap = None;
        runtime.policy.stop_session();
    }

    fn has_pending(runtime: &Self::RuntimeState) -> bool {
        runtime.pending_tap.is_some()
    }
}

fn periodic_tap_times(now: Duration, duration: Duration) -> (u64, Duration) {
    let release_at = deadline_after(now, duration);
    (protocol_timestamp(now), release_at)
}

impl<M: ControllerModel> WorkerReporting<M> for Direct {
    type RuntimeState = DirectRuntime<M>;

    fn build_worker(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        _config: &<Self as reporting::sealed::Sealed>::Config,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
        status: StatusPublisher<M>,
    ) -> WorkerCore<M, Self> {
        WorkerCore::new_direct_with_status(protocol, transport, budget, observe_output, status)
    }

    fn begin_session(
        _runtime: &mut Self::RuntimeState,
        sessions: &mut ConnectionSessions,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> ConnectionSessionId {
        sessions.begin_direct(sender, observed, input)
    }

    fn begin_handshake(session_id: ConnectionSessionId) -> Handshake {
        Handshake::until_protocol_ready(session_id)
    }

    fn handle_command(
        runtime: &mut Self::RuntimeState,
        command: <Self as reporting::sealed::Sealed>::Command<M>,
        context: ReportingCommandContext<'_, M>,
    ) -> WorkerCommandProgress {
        if !context.ready {
            return WorkerCommandProgress::Complete(Err(WorkerCommandError::Direct(
                DirectTapError::NotReady,
            )));
        }
        match command {
            DirectCommand::Send(candidate) => complete_direct_send(candidate, context),
            DirectCommand::Common(CommonCommand::Press(buttons)) => {
                let candidate = press_candidate(&context.input.snapshot(), buttons);
                complete_direct_candidate(candidate, context)
            }
            DirectCommand::Common(CommonCommand::Release(buttons)) => {
                let candidate = release_candidate(&context.input.snapshot(), buttons);
                complete_direct_candidate(candidate, context)
            }
            DirectCommand::Common(CommonCommand::Neutral) => {
                complete_direct_send(InputState::neutral(), context)
            }
            DirectCommand::Common(CommonCommand::Tap { buttons, duration }) => {
                let plan = match tap_plan(&context.input.snapshot(), buttons, duration) {
                    Ok(plan) => plan,
                    Err(error) => {
                        return WorkerCommandProgress::Complete(Err(WorkerCommandError::Input(
                            error,
                        )));
                    }
                };
                match begin_direct_tap(
                    context.ready,
                    plan,
                    context.now,
                    context.input,
                    context.protocol,
                    context.sender,
                    context.transport,
                ) {
                    Ok(tap) => {
                        runtime.pending_tap = Some(tap);
                        WorkerCommandProgress::Pending
                    }
                    Err(error) => {
                        WorkerCommandProgress::Complete(Err(WorkerCommandError::Direct(error)))
                    }
                }
            }
        }
    }

    fn handle_event(
        runtime: &mut Self::RuntimeState,
        event: TransportEvent,
        context: ReportingEventContext<'_, M>,
    ) -> ReportingEvent {
        let Some(tap) = runtime.pending_tap.take() else {
            return ReportingEvent::Passthrough(event);
        };
        match event {
            TransportEvent::HidOutput { channel, payload } => {
                match tap.step(
                    DirectTapStimulus::Transport(TransportEvent::HidOutput { channel, payload }),
                    DirectTapContext {
                        observe_output: context.observe_output,
                        protocol: context.protocol,
                        state: context.input,
                        observed: context.observed,
                        sender: context.sender,
                        status: context.status,
                        transport: context.transport,
                    },
                ) {
                    DirectTapStep::Pending { tap, output } => {
                        runtime.pending_tap = Some(tap);
                        ReportingEvent::Output(
                            output.expect("HID output always produces an output result"),
                        )
                    }
                    DirectTapStep::Complete(_) => {
                        unreachable!("HID output cannot complete a pending Direct tap")
                    }
                }
            }
            TransportEvent::Disconnected { reason } => {
                let completion = match tap.step(
                    DirectTapStimulus::Transport(TransportEvent::Disconnected { reason }),
                    DirectTapContext {
                        observe_output: context.observe_output,
                        protocol: context.protocol,
                        state: context.input,
                        observed: context.observed,
                        sender: context.sender,
                        status: context.status,
                        transport: context.transport,
                    },
                ) {
                    DirectTapStep::Complete(result) => result.err().map(WorkerCommandError::Direct),
                    DirectTapStep::Pending { tap, .. } => {
                        runtime.pending_tap = Some(tap);
                        None
                    }
                };
                ReportingEvent::Disconnected { reason, completion }
            }
            other => {
                runtime.pending_tap = Some(tap);
                ReportingEvent::Passthrough(other)
            }
        }
    }

    fn record_output(
        _runtime: &mut Self::RuntimeState,
        _completed_at: Duration,
        _completion: &Result<OutputHandling, OutputHandlingError>,
    ) -> Result<(), WorkerOperationError> {
        Ok(())
    }

    fn evaluate_readiness(
        _runtime: &mut Self::RuntimeState,
        now: Duration,
        sessions: &ConnectionSessions,
        handshake: &mut Option<Handshake>,
        sender: &ReportSender<M>,
        readiness: &mut ReadinessGate,
    ) -> Result<ReadinessProgress, ReadinessError> {
        readiness.evaluate_direct(now, sessions, handshake, sender)
    }

    fn next_deadline(runtime: &Self::RuntimeState) -> Option<Duration> {
        runtime
            .pending_tap
            .as_ref()
            .map(PendingDirectTap::release_at)
    }

    fn automatic_due_before_ready(_runtime: &Self::RuntimeState) -> bool {
        false
    }

    fn run_due(
        runtime: &mut Self::RuntimeState,
        now: Duration,
        context: ReportingDueContext<'_, M>,
    ) -> ReportingDue {
        let Some(tap) = runtime.pending_tap.take() else {
            return ReportingDue::none();
        };
        if now < tap.release_at() {
            runtime.pending_tap = Some(tap);
            return ReportingDue::none();
        }
        let mut due = ReportingDue::none();
        due.actions = 1;
        due.immediate = true;
        due.completion = Some(WorkerCommandProgress::Complete(
            tap.finish(
                now,
                context.input,
                context.protocol,
                context.sender,
                context.transport,
            )
            .map_err(WorkerCommandError::Direct),
        ));
        due
    }

    fn cancel_for_shutdown(
        runtime: &mut Self::RuntimeState,
        context: ReportingEventContext<'_, M>,
    ) -> Option<WorkerCommandError> {
        let tap = runtime.pending_tap.take()?;
        match tap.step(
            DirectTapStimulus::Shutdown,
            DirectTapContext {
                observe_output: context.observe_output,
                protocol: context.protocol,
                state: context.input,
                observed: context.observed,
                sender: context.sender,
                status: context.status,
                transport: context.transport,
            },
        ) {
            DirectTapStep::Complete(Err(error)) => Some(WorkerCommandError::Direct(error)),
            DirectTapStep::Complete(Ok(())) => None,
            DirectTapStep::Pending { tap, .. } => {
                runtime.pending_tap = Some(tap);
                None
            }
        }
    }

    fn stop_session(runtime: &mut Self::RuntimeState) {
        runtime.pending_tap = None;
    }

    fn has_pending(runtime: &Self::RuntimeState) -> bool {
        runtime.pending_tap.is_some()
    }
}

fn complete_input_candidate<M: ControllerModel>(
    candidate: Result<InputState<M>, Error>,
    input: &mut InputStateStore<M>,
) -> WorkerCommandProgress {
    match candidate {
        Ok(candidate) => {
            input.commit(candidate);
            WorkerCommandProgress::Complete(Ok(()))
        }
        Err(error) => WorkerCommandProgress::Complete(Err(WorkerCommandError::Input(error))),
    }
}

fn complete_direct_candidate<M: ControllerModel>(
    candidate: Result<InputState<M>, Error>,
    context: ReportingCommandContext<'_, M>,
) -> WorkerCommandProgress {
    match candidate {
        Ok(candidate) => complete_direct_send(candidate, context),
        Err(error) => WorkerCommandProgress::Complete(Err(WorkerCommandError::Input(error))),
    }
}

fn complete_direct_send<M: ControllerModel>(
    candidate: InputState<M>,
    context: ReportingCommandContext<'_, M>,
) -> WorkerCommandProgress {
    if !context.ready {
        return WorkerCommandProgress::Complete(Err(WorkerCommandError::Direct(
            DirectTapError::NotReady,
        )));
    }
    let now_ns = protocol_timestamp(context.now);
    WorkerCommandProgress::Complete(
        send_direct(
            candidate,
            now_ns,
            context.input,
            context.protocol,
            context.sender,
            context.transport,
        )
        .map(|_| ())
        .map_err(|error| WorkerCommandError::Direct(DirectTapError::Transport(error))),
    )
}

fn minimum_deadline(left: Option<Duration>, right: Option<Duration>) -> Option<Duration> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    }
}

fn nonterminal_operation_error(
    error: WorkerOperationError,
) -> Result<WorkerOperationError, OperationTermination> {
    match error {
        WorkerOperationError::Output(OutputHandlingError::Transport(error))
            if error.kind() != TransportErrorKind::SendRejected =>
        {
            Err(OperationTermination {
                error: WorkerCoreError::Transport(error.clone()),
                completion: WorkerOperationError::Output(OutputHandlingError::Transport(error)),
            })
        }
        WorkerOperationError::Periodic(error) => match classify_periodic_error(error) {
            PeriodicErrorDisposition::NonTerminal(error) => {
                Ok(WorkerOperationError::Periodic(error))
            }
            PeriodicErrorDisposition::Terminal { error, completion } => Err(OperationTermination {
                error: WorkerCoreError::Transport(error),
                completion: WorkerOperationError::Periodic(completion),
            }),
        },
        error => Ok(error),
    }
}

fn record_operation_error(
    progress: &mut StepProgress,
    error: WorkerOperationError,
) -> Result<(), WorkerCoreError> {
    match nonterminal_operation_error(error) {
        Ok(error) => {
            progress.operation_errors.push(error);
            Ok(())
        }
        Err(termination) => {
            progress.operation_errors.push(termination.completion);
            Err(termination.error)
        }
    }
}

struct OperationTermination {
    error: WorkerCoreError,
    completion: WorkerOperationError,
}

struct CommandTermination {
    error: WorkerCoreError,
    completion: WorkerCommandProgress,
}

fn nonterminal_command_progress(
    progress: WorkerCommandProgress,
) -> Result<WorkerCommandProgress, CommandTermination> {
    match progress {
        WorkerCommandProgress::Complete(Err(WorkerCommandError::Direct(
            DirectTapError::Transport(error),
        ))) if error.kind() != TransportErrorKind::SendRejected => Err(CommandTermination {
            error: WorkerCoreError::Transport(error.clone()),
            completion: WorkerCommandProgress::Complete(Err(WorkerCommandError::Direct(
                DirectTapError::Transport(error),
            ))),
        }),
        WorkerCommandProgress::Complete(Err(WorkerCommandError::Periodic(error))) => {
            match classify_periodic_error(error) {
                PeriodicErrorDisposition::NonTerminal(error) => Ok(
                    WorkerCommandProgress::Complete(Err(WorkerCommandError::Periodic(error))),
                ),
                PeriodicErrorDisposition::Terminal { error, completion } => {
                    Err(CommandTermination {
                        error: WorkerCoreError::Transport(error),
                        completion: WorkerCommandProgress::Complete(Err(
                            WorkerCommandError::Periodic(completion),
                        )),
                    })
                }
            }
        }
        progress => Ok(progress),
    }
}

enum PeriodicErrorDisposition {
    NonTerminal(PeriodicError),
    Terminal {
        error: TransportError,
        completion: PeriodicError,
    },
}

fn classify_periodic_error(error: PeriodicError) -> PeriodicErrorDisposition {
    match error {
        PeriodicError::Transport {
            error,
            later_terminal: Some(later_terminal),
        } => PeriodicErrorDisposition::Terminal {
            error: later_terminal,
            completion: PeriodicError::Transport {
                error,
                later_terminal: None,
            },
        },
        PeriodicError::Transport {
            error,
            later_terminal: None,
        } if error.kind() != TransportErrorKind::SendRejected => {
            PeriodicErrorDisposition::Terminal {
                error: error.clone(),
                completion: PeriodicError::Transport {
                    error,
                    later_terminal: None,
                },
            }
        }
        error => PeriodicErrorDisposition::NonTerminal(error),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex, MutexGuard},
        time::Duration,
    };

    use crate::{
        diagnostics::LifecycleState,
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::SwitchHidProtocol,
        reporting::{Direct, Periodic},
        runtime::{
            cleanup::{CleanupPhase, CloseMode},
            command::{CommandEnqueueError, command_channel},
            direct::{DirectTapError, DirectTapInterruption},
            output::{OutputHandlingError, OutputObservation},
            periodic::PeriodicError,
            readiness::ReadinessError,
            status::status_projection,
            transport::{
                ActivityNotifier, HidChannel, SendAcceptance, TransportCapabilities,
                TransportErrorKind, TransportEvent, TransportPort, TransportResult,
                activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
            worker::{
                ChannelWorkerWaiter, CommandSource, CommonCommand, DirectCommand, MonotonicClock,
                PairingError, PeriodicCommand, PriorityShutdown, RuntimeCommand, ShutdownRequest,
                StepProgress, WorkerBudget, WorkerCommandError, WorkerCore, WorkerCoreError,
                WorkerOperationError, WorkerStep, WorkerWaitError, WorkerWaitRequest, WorkerWaiter,
                periodic_tap_times, wait_for_next_iteration,
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
    const REPORT_PERIOD: Duration = Duration::from_millis(8);
    const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

    #[test]
    fn bounded_queue_feeds_one_worker_command_and_its_typed_response() {
        let mut harness = DirectHarness::ready();
        let (activity, wakes) = activity_channel();
        let (client, mut commands) = command_channel(activity);
        let response = client
            .try_enqueue(DirectCommand::Common(CommonCommand::Tap {
                buttons: vec![ProButton::B],
                duration: Duration::ZERO,
            }))
            .expect("first command fits");
        wakes.try_recv().expect("accepted command wakes worker");
        assert!(matches!(
            client.try_enqueue(DirectCommand::Common(CommonCommand::Press(vec![
                ProButton::A,
            ]))),
            Err(CommandEnqueueError::InvariantViolation)
        ));
        assert_eq!(wakes.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty));
        let mut shutdown = ShutdownLatch::default();

        let WorkerStep::Continue(mut progress) =
            harness
                .worker
                .step(&harness.clock, &mut shutdown, &mut commands)
        else {
            panic!("accepted command must keep worker running");
        };
        assert_eq!(progress.commands, 1);
        assert_eq!(progress.due_actions, 1);
        assert!(matches!(progress.command_result, Some(Ok(()))));
        commands
            .deliver_progress(&mut progress)
            .expect("deliver worker result to its response");
        assert!(matches!(response.try_recv(), Ok(Ok(()))));
    }

    #[test]
    fn pair_response_stays_pending_until_ready_and_blocks_following_input() {
        let (transport, control, trace) = tracing_transport();
        let clock = FakeClock::at(Duration::ZERO);
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            observer(trace),
        );
        let (activity, _wakes) = activity_channel();
        let (client, mut commands) = command_channel(activity);
        let pair = client
            .try_enqueue(RuntimeCommand::<Pro, Direct>::Pair {
                timeout: CONNECTION_TIMEOUT,
            })
            .expect("pair command fits");
        let mut shutdown = ShutdownLatch::default();

        let WorkerStep::Continue(mut accepted) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("accepted pair must keep the worker running");
        };
        assert_eq!(accepted.commands, 1);
        assert!(accepted.command_result.is_none());
        let input = client
            .try_enqueue(RuntimeCommand::Input(DirectCommand::Common(
                CommonCommand::Press(vec![ProButton::A]),
            )))
            .expect("following input fits after the worker receives pair");
        commands
            .deliver_progress(&mut accepted)
            .expect("retain the pair response");
        assert!(matches!(
            pair.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        assert!(matches!(
            input.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        let WorkerStep::Continue(blocked) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("pending pair must keep the worker running");
        };
        assert_eq!(blocked.commands, 0, "input must stay queued before Ready");

        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        control
            .inject_hid_output(HidChannel::Control, &subcommand_report(0x03, &[0x30]))
            .expect("report mode");
        control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x30, &[0x01]))
            .expect("player lights");

        let WorkerStep::Continue(handshake) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("fake topology events must keep pairing active");
        };
        assert_eq!(worker.lifecycle_state(), LifecycleState::Connecting);
        assert_eq!(handshake.commands, 0);
        assert!(handshake.command_result.is_none());
        assert!(matches!(
            pair.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        let WorkerStep::Continue(mut ready) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("fake protocol replies must reach Ready");
        };
        assert_eq!(worker.lifecycle_state(), LifecycleState::Ready);
        assert_eq!(ready.commands, 0);
        assert!(matches!(ready.command_result, Some(Ok(()))));
        commands
            .deliver_progress(&mut ready)
            .expect("complete the retained pair response");
        assert!(matches!(pair.try_recv(), Ok(Ok(()))));
        assert!(matches!(
            input.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        let WorkerStep::Continue(mut input_step) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("Ready input must keep the worker running");
        };
        assert_eq!(input_step.commands, 1);
        commands
            .deliver_progress(&mut input_step)
            .expect("deliver input after pair completion");
        assert!(matches!(input.try_recv(), Ok(Ok(()))));
    }

    #[test]
    fn pair_starts_the_transport_before_connection_state_and_preserves_begin_failure() {
        let (mut transport, _control, trace) = tracing_transport();
        transport.start_pairing_error = Some(TransportErrorKind::SourceTerminated);
        let clock = FakeClock::at(Duration::ZERO);
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            observer(Arc::clone(&trace)),
        );
        let (activity, _wakes) = activity_channel();
        let (client, mut commands) = command_channel(activity);
        let response = client
            .try_enqueue(RuntimeCommand::<Pro, Direct>::Pair {
                timeout: CONNECTION_TIMEOUT,
            })
            .expect("pair command fits");
        let mut shutdown = ShutdownLatch::default();

        let WorkerStep::Continue(mut rejected) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("pair begin rejection must remain a command failure");
        };

        assert_eq!(worker.lifecycle_state(), LifecycleState::Open);
        assert_eq!(lock(&trace).first(), Some(&Trace::StartPairing));
        assert!(matches!(
            rejected.command_result.as_ref(),
            Some(Err(
                WorkerCommandError::Pair(PairingError::Begin(WorkerCoreError::Transport(error)))
            )) if error.kind() == TransportErrorKind::SourceTerminated
        ));
        commands
            .deliver_progress(&mut rejected)
            .expect("deliver typed pair begin failure");
        assert!(matches!(
            response.try_recv(),
            Ok(Err(WorkerCommandError::Pair(PairingError::Begin(
                WorkerCoreError::Transport(error)
            )))) if error.kind() == TransportErrorKind::SourceTerminated
        ));
    }

    #[test]
    fn pair_timeout_completes_the_retained_response_once() {
        let (transport, _control, trace) = tracing_transport();
        let clock = FakeClock::at(Duration::ZERO);
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            observer(trace),
        );
        let (activity, _wakes) = activity_channel();
        let (client, mut commands) = command_channel(activity);
        let response = client
            .try_enqueue(RuntimeCommand::<Pro, Direct>::Pair {
                timeout: CONNECTION_TIMEOUT,
            })
            .expect("pair command fits");
        let mut shutdown = ShutdownLatch::default();

        let WorkerStep::Continue(mut accepted) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("accepted pair must keep the worker running");
        };
        commands
            .deliver_progress(&mut accepted)
            .expect("retain pair response");
        assert!(matches!(
            response.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        clock.set(CONNECTION_TIMEOUT);
        let WorkerStep::Continue(mut timed_out) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("pair timeout is a recoverable command failure");
        };
        assert!(matches!(
            timed_out.command_result.as_ref(),
            Some(Err(WorkerCommandError::Pair(PairingError::Readiness(
                ReadinessError::TimedOut
            ))))
        ));
        commands
            .deliver_progress(&mut timed_out)
            .expect("complete retained pair response");
        assert!(matches!(
            response.try_recv(),
            Ok(Err(WorkerCommandError::Pair(PairingError::Readiness(
                ReadinessError::TimedOut
            ))))
        ));
        assert!(matches!(
            response.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ));

        let WorkerStep::Continue(after_timeout) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("worker remains open after pair timeout");
        };
        assert!(after_timeout.command_result.is_none());
        assert_eq!(worker.lifecycle_state(), LifecycleState::Open);
    }

    #[test]
    fn queued_command_transport_and_shutdown_activity_wakes_the_idle_worker() {
        let (mut transport, control) = FakeTransport::with_limits(8, 2);
        let (activity, receiver) = activity_channel();
        transport
            .open(activity.clone())
            .expect("open with the worker activity notifier");
        let command_activity = activity.clone();
        let shutdown_activity = activity;
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(TracingTransport {
                inner: transport,
                trace: Arc::clone(&trace),
                start_pairing_error: None,
            }),
            WorkerBudget::new(1),
            observer(Arc::clone(&trace)),
        );
        let clock = FakeClock::at(Duration::ZERO);
        let mut waiter = ChannelWorkerWaiter::new(receiver);
        let mut shutdown = ShutdownLatch::default();
        let mut commands = TracedCommands::new([], Arc::clone(&trace));

        let WorkerStep::Continue(idle) = worker.step(&clock, &mut shutdown, &mut commands) else {
            panic!("an open worker without work must remain idle");
        };
        assert_eq!(
            idle.wait_request(clock.now()),
            Some(WorkerWaitRequest::Activity)
        );

        let mut commands = TracedCommands::new(
            [("neutral", DirectCommand::Common(CommonCommand::Neutral))],
            Arc::clone(&trace),
        );
        command_activity.notify();
        control
            .inject_connected()
            .expect("publish transport work before its notification");
        wait_for_next_iteration(&idle, &clock, &mut waiter)
            .expect("coalesced command and transport activity wakes the worker");

        let WorkerStep::Continue(progress) = worker.step(&clock, &mut shutdown, &mut commands)
        else {
            panic!("non-terminal activity must keep the worker running");
        };
        assert_eq!(progress.commands, 1);
        assert_eq!(progress.hci_events, 1);
        assert_eq!(commands.remaining(), 0);
        assert_eq!(progress.wait_request(clock.now()), None);

        let WorkerStep::Continue(idle) = worker.step(&clock, &mut shutdown, &mut commands) else {
            panic!("the immediate follow-up step must return to idle");
        };
        assert_eq!(
            idle.wait_request(clock.now()),
            Some(WorkerWaitRequest::Activity)
        );

        let mut shutdown = ShutdownLatch::new(ShutdownRequest::explicit(CloseMode::WithoutNeutral));
        shutdown_activity.notify();
        wait_for_next_iteration(&idle, &clock, &mut waiter)
            .expect("shutdown activity wakes the idle worker");
        assert!(matches!(
            worker.step(&clock, &mut shutdown, &mut commands),
            WorkerStep::Closed { .. }
        ));
    }

    #[test]
    fn pending_notifications_coalesce_to_one_activity_token() {
        let (activity, receiver) = activity_channel();
        let command_activity = activity.clone();
        let transport_activity = activity.clone();
        let shutdown_activity = activity;

        command_activity.notify();
        transport_activity.notify();
        shutdown_activity.notify();
        drop(command_activity);
        drop(transport_activity);
        drop(shutdown_activity);

        let clock = FakeClock::at(Duration::ZERO);
        let mut waiter = ChannelWorkerWaiter::new(receiver);
        assert_eq!(waiter.wait(WorkerWaitRequest::Activity, &clock), Ok(()));
        assert_eq!(
            waiter.wait(WorkerWaitRequest::Activity, &clock),
            Err(WorkerWaitError::Disconnected)
        );
    }

    #[test]
    fn immediate_progress_and_due_deadlines_bypass_the_waiter() {
        let clock = FakeClock::at(Duration::from_millis(310));
        let mut waiter = ScriptedWaiter::new(clock.clone());
        let immediate = progress_for_wait(true, Some(Duration::from_millis(318)));
        let due_now = progress_for_wait(false, Some(Duration::from_millis(310)));
        let overdue = progress_for_wait(false, Some(Duration::from_millis(309)));

        wait_for_next_iteration(&immediate, &clock, &mut waiter)
            .expect("immediate progress continues without waiting");
        wait_for_next_iteration(&due_now, &clock, &mut waiter)
            .expect("a due deadline continues without waiting");
        wait_for_next_iteration(&overdue, &clock, &mut waiter)
            .expect("an overdue deadline continues without waiting");

        assert!(waiter.requests.is_empty());
        assert_eq!(clock.now(), Duration::from_millis(310));
    }

    #[test]
    fn idle_and_future_deadline_waits_are_recorded_without_wall_clock_time() {
        let clock = FakeClock::at(Duration::from_millis(310));
        let mut waiter = ScriptedWaiter::new(clock.clone());
        let idle = progress_for_wait(false, None);

        wait_for_next_iteration(&idle, &clock, &mut waiter)
            .expect("idle progress requests activity");
        assert_eq!(waiter.requests, [WorkerWaitRequest::Activity]);
        assert_eq!(clock.now(), Duration::from_millis(310));

        let deadline = Duration::from_millis(318);
        let scheduled = progress_for_wait(false, Some(deadline));
        wait_for_next_iteration(&scheduled, &clock, &mut waiter)
            .expect("scripted deadline advances the fake clock");
        assert_eq!(
            waiter.requests,
            [
                WorkerWaitRequest::Activity,
                WorkerWaitRequest::ActivityOrDeadline(deadline),
            ]
        );
        assert_eq!(clock.now(), deadline);
    }

    #[test]
    fn deadline_reached_at_channel_wait_does_not_consume_activity() {
        let (activity, receiver) = activity_channel();
        activity.notify();
        drop(activity);
        let now = Duration::from_millis(318);
        let clock = FakeClock::at(now);
        let mut waiter = ChannelWorkerWaiter::new(receiver);

        assert_eq!(
            waiter.wait(WorkerWaitRequest::ActivityOrDeadline(now), &clock),
            Ok(())
        );
        assert_eq!(waiter.wait(WorkerWaitRequest::Activity, &clock), Ok(()));
        assert_eq!(
            waiter.wait(WorkerWaitRequest::Activity, &clock),
            Err(WorkerWaitError::Disconnected)
        );
    }

    #[test]
    fn shutdown_preempts_pending_tap_commands_transport_and_deadline() {
        let mut harness = DirectHarness::ready();
        let started_at = Duration::from_millis(100);
        harness.clock.set(started_at);
        let mut tap = TracedCommands::new(
            [(
                "tap-b",
                crate::runtime::worker::DirectCommand::Common(CommonCommand::Tap {
                    buttons: vec![ProButton::B],
                    duration: Duration::from_millis(80),
                }),
            )],
            Arc::clone(&harness.trace),
        );
        let mut no_shutdown = ShutdownLatch::default();
        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut tap);
        assert!(matches!(step, WorkerStep::Continue(_)));
        assert_eq!(tap.remaining(), 0);
        assert!(harness.worker.has_pending_reporting_command());
        let pressed = harness.worker.input_snapshot();
        let timer_after_press = harness.worker.sender_timer();
        let release_at = harness
            .worker
            .next_deadline()
            .expect("pending Direct tap has a release deadline");

        harness
            .control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x08, &[]))
            .expect("queue HID output");
        let mut commands = TracedCommands::new(
            [(
                "press-x",
                crate::runtime::worker::DirectCommand::Common(CommonCommand::Press(vec![
                    ProButton::X,
                ])),
            )],
            Arc::clone(&harness.trace),
        );
        harness.clock.set(started_at + Duration::from_millis(10));
        lock(&harness.trace).clear();
        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("pending tap must keep the worker running");
        };
        assert_eq!(progress.commands, 0);
        assert_eq!(progress.hci_events, 1);
        assert_eq!(progress.due_actions, 0);
        assert_eq!(commands.polls(), 0);
        assert_eq!(commands.remaining(), 1);
        assert!(harness.worker.has_pending_reporting_command());
        assert_eq!(harness.worker.input_snapshot(), pressed);
        let timer_after_reply = harness.worker.sender_timer();

        harness.clock.set(release_at);
        lock(&harness.trace).clear();
        let mut shutdown = ShutdownLatch::new(ShutdownRequest::explicit(CloseMode::WithoutNeutral));

        let step = harness
            .worker
            .step(&harness.clock, &mut shutdown, &mut commands);

        let WorkerStep::Closed {
            completion,
            interrupted,
            progress: _,
        } = step
        else {
            panic!("priority close must finish the worker core");
        };
        assert!(completion.performed());
        assert!(matches!(
            interrupted,
            Some(WorkerCommandError::Direct(DirectTapError::Interrupted(
                DirectTapInterruption::Shutdown
            )))
        ));
        assert_eq!(commands.polls(), 0);
        assert_eq!(commands.remaining(), 1);
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closing);
        assert_eq!(harness.worker.input_snapshot(), pressed);
        assert_eq!(
            harness.worker.sender_timer(),
            timer_after_press.wrapping_add(1)
        );
        assert_eq!(harness.worker.sender_timer(), timer_after_reply);
        assert!(!harness.worker.has_pending_reporting_command());
        assert_eq!(
            *lock(&harness.trace),
            [Trace::Drain, Trace::Disconnect, Trace::TransportClose]
        );
    }

    #[test]
    fn shutdown_wins_over_a_terminal_command_without_losing_its_completion() {
        let mut harness = DirectHarness::ready();
        harness
            .control
            .terminate_with(std::io::Error::other("sensitive backend detail"))
            .expect("terminate source before the command");
        let timer_before = harness.worker.sender_timer();
        let mut commands = TracedCommands::new(
            [(
                "press-a",
                DirectCommand::Common(CommonCommand::Press(vec![ProButton::A])),
            )],
            Arc::clone(&harness.trace),
        );
        let mut shutdown =
            ShutdownLatch::after_checks(ShutdownRequest::explicit(CloseMode::WithoutNeutral), 1);
        lock(&harness.trace).clear();

        let step = harness
            .worker
            .step(&harness.clock, &mut shutdown, &mut commands);

        let WorkerStep::Closed {
            completion,
            interrupted,
            progress,
        } = step
        else {
            panic!("shutdown must take priority over the terminal command failure");
        };
        assert!(completion.performed());
        assert!(interrupted.is_none());
        assert_eq!(progress.commands, 1);
        let Some(Err(WorkerCommandError::Direct(DirectTapError::Transport(error)))) =
            progress.command_result.as_ref()
        else {
            panic!("the processed command must retain its terminal completion");
        };
        assert_eq!(error.kind(), TransportErrorKind::SourceTerminated);
        assert!(!error.to_string().contains("sensitive"));
        assert_eq!(commands.remaining(), 0);
        assert_eq!(harness.worker.sender_timer(), timer_before);
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closing);
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Command("press-a"),
                Trace::Send {
                    report_id: 0x30,
                    timer: timer_before,
                    buttons: buttons([ButtonKind::A]),
                    accepted: false,
                },
                Trace::Drain,
                Trace::Disconnect,
                Trace::TransportClose,
            ]
        );
    }

    #[test]
    fn unspawned_worker_cleanup_returns_failure_and_closes_without_neutral() {
        let (transport, control, trace) = tracing_transport();
        let (status, status_reader) = status_projection::<Pro, Direct>();
        let mut worker = WorkerCore::new_direct_with_status(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            Box::new(|_| {}),
            status,
        );
        control
            .terminate_with(std::io::Error::other("sensitive backend detail"))
            .expect("terminate fake transport before cleanup");

        let failure = worker
            .cleanup_unspawned_without_neutral(Duration::from_millis(125))
            .expect_err("drain failure must reach the spawn caller");

        assert_eq!(failure.phase(), CleanupPhase::DrainInterrupt);
        assert_eq!(
            failure.source_error().kind(),
            TransportErrorKind::SourceTerminated
        );
        assert_eq!(worker.lifecycle_state(), LifecycleState::Closing);
        let status = status_reader.status();
        assert_eq!(status.lifecycle, LifecycleState::Closed);
        assert!(!status.connected);
        assert_eq!(status.report_mode, None);
        assert_eq!(
            *lock(&trace),
            [Trace::Drain, Trace::Disconnect, Trace::TransportClose]
        );
        assert!(
            control.accepted_interrupts().is_empty(),
            "without-neutral cleanup must not send an input report"
        );
    }

    #[test]
    fn one_command_per_step_yields_to_hid_reply_and_due_report() {
        let mut harness = PeriodicHarness::ready();
        let due = harness
            .worker
            .next_deadline()
            .expect("ready Periodic worker has a report deadline");
        harness.clock.set(due);
        harness
            .control
            .script_sends([ScriptedSendOutcome::Rejected, ScriptedSendOutcome::Accepted]);
        harness
            .control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x08, &[]))
            .expect("queue HID output");
        for _ in 0..3 {
            harness
                .control
                .inject_hid_output(HidChannel::Control, &rumble_report())
                .expect("queue rumble-only output");
        }
        let timer_before = harness.worker.sender_timer();
        let mut commands = TracedCommands::new(
            [
                (
                    "press-a",
                    PeriodicCommand::Common(CommonCommand::Press(vec![ProButton::A])),
                ),
                (
                    "press-b",
                    PeriodicCommand::Common(CommonCommand::Press(vec![ProButton::B])),
                ),
                (
                    "press-x",
                    PeriodicCommand::Common(CommonCommand::Press(vec![ProButton::X])),
                ),
            ],
            Arc::clone(&harness.trace),
        );
        lock(&harness.trace).clear();
        let mut no_shutdown = ShutdownLatch::default();

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Continue(progress) = step else {
            panic!("ordinary work must keep the worker running");
        };
        assert_eq!(progress.commands, 1);
        assert_eq!(progress.hci_events, 3);
        assert_eq!(progress.due_actions, 1);
        assert!(progress.immediate);
        assert_eq!(commands.remaining(), 2);
        assert_eq!(commands.polls(), 1);
        assert_eq!(
            harness
                .worker
                .input_snapshot()
                .buttons()
                .map(|button| button.kind())
                .collect::<Vec<_>>(),
            [ButtonKind::A]
        );
        assert_eq!(harness.worker.sender_timer(), timer_before.wrapping_add(1));
        assert!(
            harness
                .worker
                .next_deadline()
                .is_some_and(|deadline| deadline > due)
        );
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Command("press-a"),
                Trace::Poll(3),
                Trace::Observe {
                    channel: HidChannel::Interrupt,
                    report_id: 0x01,
                },
                Trace::Send {
                    report_id: 0x21,
                    timer: timer_before,
                    buttons: buttons([ButtonKind::A]),
                    accepted: false,
                },
                Trace::Observe {
                    channel: HidChannel::Control,
                    report_id: 0x10,
                },
                Trace::Observe {
                    channel: HidChannel::Control,
                    report_id: 0x10,
                },
                Trace::Send {
                    report_id: 0x30,
                    timer: timer_before,
                    buttons: buttons([ButtonKind::A]),
                    accepted: true,
                },
            ]
        );

        lock(&harness.trace).clear();
        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("the next command step must keep running");
        };
        assert_eq!(progress.commands, 1);
        assert_eq!(progress.hci_events, 1);
        assert!(progress.immediate);
        assert_eq!(commands.remaining(), 1);
        assert_eq!(
            harness
                .worker
                .input_snapshot()
                .buttons()
                .map(|button| button.kind())
                .collect::<Vec<_>>(),
            [ButtonKind::A, ButtonKind::B]
        );
        assert!(
            !lock(&harness.trace)
                .iter()
                .any(|event| matches!(event, Trace::Send { .. }))
        );

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("the final command step must keep running");
        };
        assert_eq!(progress.commands, 1);
        assert_eq!(commands.remaining(), 0);
        assert_eq!(
            harness
                .worker
                .input_snapshot()
                .buttons()
                .map(|button| button.kind())
                .collect::<Vec<_>>(),
            [ButtonKind::A, ButtonKind::B, ButtonKind::X]
        );
    }

    #[test]
    fn shutdown_after_hid_reply_preempts_poll_backlog_and_due_report() {
        let mut harness = PeriodicHarness::ready();
        let due = harness
            .worker
            .next_deadline()
            .expect("ready Periodic worker has a report deadline");
        harness.clock.set(due);
        harness
            .control
            .script_sends([ScriptedSendOutcome::Accepted]);
        harness
            .control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x08, &[]))
            .expect("queue HID output");
        harness
            .control
            .inject_hid_output(HidChannel::Control, &rumble_report())
            .expect("queue poll backlog");
        let timer_before = harness.worker.sender_timer();
        let mut commands = TracedCommands::new([], Arc::clone(&harness.trace));
        let mut shutdown =
            ShutdownLatch::after_checks(ShutdownRequest::explicit(CloseMode::WithoutNeutral), 1);
        lock(&harness.trace).clear();

        let step = harness
            .worker
            .step(&harness.clock, &mut shutdown, &mut commands);

        let WorkerStep::Closed {
            completion,
            interrupted,
            progress: _,
        } = step
        else {
            panic!("shutdown observed after the reply must close the worker");
        };
        assert!(completion.performed());
        assert!(interrupted.is_none());
        assert_eq!(harness.worker.sender_timer(), timer_before.wrapping_add(1));
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closing);
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Poll(2),
                Trace::Observe {
                    channel: HidChannel::Interrupt,
                    report_id: 0x01,
                },
                Trace::Send {
                    report_id: 0x21,
                    timer: timer_before,
                    buttons: buttons([]),
                    accepted: true,
                },
                Trace::Drain,
                Trace::Disconnect,
                Trace::TransportClose,
            ]
        );
    }

    #[test]
    fn topology_bootstrap_precedes_first_output_from_the_same_poll_batch() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            Box::new(|_| {}),
        );
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.script_sends([ScriptedSendOutcome::Accepted, ScriptedSendOutcome::Accepted]);
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        control
            .inject_hid_output(HidChannel::Control, &subcommand_report(0x08, &[]))
            .expect("first HID output");
        let mut commands = TracedCommands::new([], trace);
        let mut no_shutdown = ShutdownLatch::default();

        let step = worker.step(&clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Continue(progress) = step else {
            panic!("handshake input must keep the worker running");
        };
        assert_eq!(progress.hci_events, 4);
        assert_eq!(progress.due_actions, 1);
        assert_eq!(
            control
                .accepted_interrupts()
                .iter()
                .map(|report| (
                    report[0],
                    report[1],
                    (report[0] == 0x21).then_some(report[14]),
                ))
                .collect::<Vec<_>>(),
            [(0x30, 0, None), (0x21, 1, Some(0x08))]
        );
    }

    #[test]
    fn periodic_worker_sends_report_mode_input_while_readiness_is_connecting() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_periodic(
            protocol(),
            Box::new(transport),
            REPORT_PERIOD,
            WorkerBudget::new(1),
            Box::new(|_| {}),
        )
        .expect("valid Periodic worker");
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.script_sends([
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
        ]);
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        let mut commands = TracedCommands::new([], trace);
        let mut no_shutdown = ShutdownLatch::default();

        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        clock.set(Duration::from_millis(10));
        control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x03, &[0x30]))
            .expect("report mode");
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        assert_eq!(worker.lifecycle_state(), LifecycleState::Connecting);

        clock.set(Duration::from_millis(310));
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        clock.set(Duration::from_millis(318));
        let WorkerStep::Continue(progress) = worker.step(&clock, &mut no_shutdown, &mut commands)
        else {
            panic!("pre-ready automatic input keeps the worker running");
        };

        assert_eq!(worker.lifecycle_state(), LifecycleState::Connecting);
        assert_eq!(progress.due_actions, 1);
        assert_eq!(
            control
                .accepted_interrupts()
                .iter()
                .map(|report| (
                    report[0],
                    report[1],
                    (report[0] == 0x21).then_some(report[14]),
                ))
                .collect::<Vec<_>>(),
            [(0x30, 0, None), (0x21, 1, Some(0x03)), (0x30, 2, None),]
        );
    }

    #[test]
    fn direct_worker_retries_bootstrap_until_protocol_ready_then_stays_idle() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            Box::new(|_| {}),
        );
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.script_sends([
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Accepted,
        ]);
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        let mut commands = TracedCommands::new([], trace);
        let mut no_shutdown = ShutdownLatch::default();

        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        clock.set(Duration::from_millis(10));
        control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x03, &[0x30]))
            .expect("report mode");
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        assert_eq!(worker.lifecycle_state(), LifecycleState::Connecting);

        clock.set(Duration::from_secs(1));
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        assert_eq!(
            control
                .accepted_interrupts()
                .iter()
                .map(|report| (
                    report[0],
                    report[1],
                    (report[0] == 0x21).then_some(report[14]),
                ))
                .collect::<Vec<_>>(),
            [(0x30, 0, None), (0x21, 1, Some(0x03)), (0x30, 2, None)],
            "Direct must send a readiness-only bootstrap after report mode"
        );

        clock.set(Duration::from_millis(1_010));
        control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x30, &[0x01]))
            .expect("player lights");
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        assert_eq!(worker.lifecycle_state(), LifecycleState::Ready);

        let ready_report_count = control.accepted_interrupts().len();
        clock.set(Duration::from_secs(2));
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        assert_eq!(
            control.accepted_interrupts().len(),
            ready_report_count,
            "Direct must stop automatic reports after readiness"
        );
    }

    #[test]
    fn terminal_bootstrap_send_failure_marks_the_worker_failed() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            Box::new(|_| {}),
        );
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        control
            .terminate_with(std::io::Error::other("sensitive backend detail"))
            .expect("terminate fake source after topology events");
        let mut commands = TracedCommands::new([], trace);
        let mut no_shutdown = ShutdownLatch::default();

        let step = worker.step(&clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Failed {
            error: WorkerCoreError::Transport(error),
            progress: _,
        } = step
        else {
            panic!("terminal bootstrap failure must stop the worker");
        };
        assert_eq!(error.kind(), TransportErrorKind::SourceTerminated);
        assert!(!error.to_string().contains("sensitive"));
        assert_eq!(worker.lifecycle_state(), LifecycleState::Failed);
        assert!(control.accepted_interrupts().is_empty());
    }

    #[test]
    fn rejected_bootstrap_keeps_the_absolute_retry_deadline() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            Box::new(|_| {}),
        );
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.script_sends([ScriptedSendOutcome::Rejected]);
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        let mut commands = TracedCommands::new([], trace);
        let mut no_shutdown = ShutdownLatch::default();

        let step = worker.step(&clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Continue(progress) = step else {
            panic!("rejected bootstrap remains retryable");
        };
        let [WorkerOperationError::Output(OutputHandlingError::Transport(error))] =
            progress.operation_errors.as_slice()
        else {
            panic!("rejected bootstrap must retain its transport error");
        };
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(progress.due_actions, 1);
        assert_eq!(progress.skipped_deadlines, 0);
        assert_eq!(progress.next_deadline, Some(Duration::from_secs(1)));
        assert_eq!(worker.lifecycle_state(), LifecycleState::Connecting);
        assert_eq!(worker.sender_timer(), 0);
        assert!(control.accepted_interrupts().is_empty());

        clock.set(Duration::from_secs(1));
        control.script_sends([ScriptedSendOutcome::Accepted]);
        let step = worker.step(&clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("accepted retry keeps the connection pending");
        };
        assert_eq!(progress.due_actions, 1);
        assert!(progress.operation_errors.is_empty());
        assert_eq!(worker.sender_timer(), 1);
        assert_eq!(
            control
                .accepted_interrupts()
                .iter()
                .map(|report| (report[0], report[1]))
                .collect::<Vec<_>>(),
            [(0x30, 0)]
        );
    }

    #[test]
    fn disconnect_aborts_connecting_readiness_and_removes_its_retry_deadline() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            Box::new(|_| {}),
        );
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.script_sends([ScriptedSendOutcome::Rejected]);
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        let mut commands = TracedCommands::new([], trace);
        let mut no_shutdown = ShutdownLatch::default();
        let step = worker.step(&clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("rejected bootstrap keeps readiness pending");
        };
        assert_eq!(progress.next_deadline, Some(Duration::from_secs(1)));
        assert_eq!(worker.lifecycle_state(), LifecycleState::Connecting);

        control
            .inject_disconnected(Some(0x13))
            .expect("disconnect event");
        let step = worker.step(&clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("a clean disconnect keeps the worker available");
        };
        let [WorkerOperationError::Readiness] = progress.operation_errors.as_slice() else {
            panic!("connecting readiness must record its operation failure");
        };
        assert_eq!(progress.hci_events, 1);
        assert_eq!(progress.due_actions, 0);
        assert_eq!(progress.next_deadline, None);
        assert_eq!(worker.lifecycle_state(), LifecycleState::Open);
        assert_eq!(worker.sender_timer(), 0);
        assert!(control.accepted_interrupts().is_empty());

        clock.set(Duration::from_secs(1));
        let step = worker.step(&clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("the disconnected worker remains idle");
        };
        assert_eq!(progress.due_actions, 0);
        assert_eq!(progress.next_deadline, None);
        assert_eq!(worker.sender_timer(), 0);
        assert!(control.accepted_interrupts().is_empty());
    }

    #[test]
    fn accepted_reply_holds_the_due_periodic_report_in_the_worker() {
        let mut harness = PeriodicHarness::ready();
        let due = harness
            .worker
            .next_deadline()
            .expect("ready Periodic worker has a report deadline");
        let holdoff_until = due + Duration::from_millis(300);
        harness.clock.set(due);
        harness
            .control
            .script_sends([ScriptedSendOutcome::Accepted]);
        harness
            .control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x08, &[]))
            .expect("queue accepted reply");
        let timer_before = harness.worker.sender_timer();
        let mut commands = TracedCommands::new([], Arc::clone(&harness.trace));
        let mut no_shutdown = ShutdownLatch::default();
        lock(&harness.trace).clear();

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Continue(progress) = step else {
            panic!("accepted reply keeps the worker running");
        };
        assert_eq!(progress.hci_events, 1);
        assert_eq!(progress.due_actions, 0);
        assert_eq!(progress.next_deadline, Some(holdoff_until));
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Poll(1),
                Trace::Observe {
                    channel: HidChannel::Interrupt,
                    report_id: 0x01,
                },
                Trace::Send {
                    report_id: 0x21,
                    timer: timer_before,
                    buttons: buttons([]),
                    accepted: true,
                },
            ]
        );

        harness.clock.set(holdoff_until);
        lock(&harness.trace).clear();
        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("holdoff expiry keeps the worker running");
        };
        assert_eq!(progress.due_actions, 1);
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Poll(0),
                Trace::Send {
                    report_id: 0x30,
                    timer: timer_before.wrapping_add(1),
                    buttons: buttons([]),
                    accepted: true,
                },
            ]
        );
    }

    #[test]
    fn terminal_reply_send_failure_preempts_the_due_periodic_report() {
        let mut harness = PeriodicHarness::ready();
        let due = harness
            .worker
            .next_deadline()
            .expect("ready Periodic worker has a report deadline");
        harness.clock.set(due);
        harness
            .control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x08, &[]))
            .expect("queue output before source termination");
        harness
            .control
            .terminate_with(std::io::Error::other("sensitive backend detail"))
            .expect("terminate source after queued output");
        let timer_before = harness.worker.sender_timer();
        let mut commands = TracedCommands::new([], Arc::clone(&harness.trace));
        let mut no_shutdown = ShutdownLatch::default();
        lock(&harness.trace).clear();

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Failed {
            error: WorkerCoreError::Transport(error),
            progress: _,
        } = step
        else {
            panic!("terminal reply send failure must stop the worker");
        };
        assert_eq!(error.kind(), TransportErrorKind::SourceTerminated);
        assert!(!error.to_string().contains("sensitive"));
        assert_eq!(harness.worker.sender_timer(), timer_before);
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Failed);
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Poll(1),
                Trace::Observe {
                    channel: HidChannel::Interrupt,
                    report_id: 0x01,
                },
                Trace::Send {
                    report_id: 0x21,
                    timer: timer_before,
                    buttons: buttons([]),
                    accepted: false,
                },
            ]
        );
    }

    #[test]
    fn shutdown_latched_during_a_terminal_reply_runs_cleanup_before_failure() {
        let mut harness = PeriodicHarness::ready();
        let due = harness
            .worker
            .next_deadline()
            .expect("ready Periodic worker has a report deadline");
        harness.clock.set(due);
        harness
            .control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x08, &[]))
            .expect("queue output before source termination");
        harness
            .control
            .terminate_with(std::io::Error::other("sensitive backend detail"))
            .expect("terminate source after queued output");
        let timer_before = harness.worker.sender_timer();
        let mut commands = TracedCommands::new([], Arc::clone(&harness.trace));
        let mut shutdown =
            ShutdownLatch::after_checks(ShutdownRequest::explicit(CloseMode::WithoutNeutral), 1);
        lock(&harness.trace).clear();

        let step = harness
            .worker
            .step(&harness.clock, &mut shutdown, &mut commands);

        let WorkerStep::Closed {
            completion,
            interrupted,
            progress,
        } = step
        else {
            panic!("shutdown must take priority over the terminal reply failure");
        };
        assert!(completion.performed());
        assert!(interrupted.is_none());
        assert_eq!(progress.hci_events, 1);
        let [WorkerOperationError::Output(OutputHandlingError::Transport(error))] =
            progress.operation_errors.as_slice()
        else {
            panic!("the processed reply must retain its terminal operation error");
        };
        assert_eq!(error.kind(), TransportErrorKind::SourceTerminated);
        assert!(!error.to_string().contains("sensitive"));
        assert_eq!(progress.due_actions, 0);
        assert_eq!(harness.worker.sender_timer(), timer_before);
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closing);
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Poll(1),
                Trace::Observe {
                    channel: HidChannel::Interrupt,
                    report_id: 0x01,
                },
                Trace::Send {
                    report_id: 0x21,
                    timer: timer_before,
                    buttons: buttons([]),
                    accepted: false,
                },
                Trace::Drain,
                Trace::Disconnect,
                Trace::TransportClose,
            ]
        );
    }

    #[test]
    fn operation_timeout_precedes_a_same_time_bootstrap_retry() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1),
            Box::new(|_| {}),
        );
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        let mut commands = TracedCommands::new([], trace);
        let mut no_shutdown = ShutdownLatch::default();
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));

        clock.set(Duration::from_secs(1));
        assert!(matches!(
            worker.step(&clock, &mut no_shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));
        assert_eq!(control.accepted_interrupts().len(), 2);

        clock.set(CONNECTION_TIMEOUT);
        let step = worker.step(&clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Continue(progress) = step else {
            panic!("connection timeout is an operation error, not a worker failure");
        };
        let [WorkerOperationError::Readiness] = progress.operation_errors.as_slice() else {
            panic!("deadline boundary must record its operation failure");
        };
        assert_eq!(progress.due_actions, 0);
        assert_eq!(control.accepted_interrupts().len(), 2);
        assert_eq!(worker.lifecycle_state(), LifecycleState::Open);
    }

    #[test]
    fn pending_direct_tap_releases_before_the_next_queued_command() {
        let mut harness = DirectHarness::ready();
        let started_at = Duration::from_millis(100);
        harness.clock.set(started_at);
        let mut commands = TracedCommands::new(
            [
                (
                    "tap-b",
                    DirectCommand::Common(CommonCommand::Tap {
                        buttons: vec![ProButton::B],
                        duration: Duration::from_millis(80),
                    }),
                ),
                (
                    "press-x",
                    DirectCommand::Common(CommonCommand::Press(vec![ProButton::X])),
                ),
            ],
            Arc::clone(&harness.trace),
        );
        let mut no_shutdown = ShutdownLatch::default();

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("pending tap keeps the worker running");
        };
        assert_eq!(progress.commands, 1);
        assert!(progress.command_result.is_none());
        assert_eq!(commands.remaining(), 1);
        let release_at = harness
            .worker
            .next_deadline()
            .expect("pending tap deadline");

        harness.clock.set(release_at);
        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("tap release keeps the worker running");
        };
        assert_eq!(progress.commands, 0);
        assert_eq!(progress.due_actions, 1);
        assert!(progress.immediate);
        assert!(matches!(progress.command_result, Some(Ok(()))));
        assert_eq!(commands.remaining(), 1);
        assert_eq!(harness.worker.input_snapshot(), InputState::neutral());

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("queued command keeps the worker running");
        };
        assert_eq!(progress.commands, 1);
        assert_eq!(commands.remaining(), 0);
        assert_eq!(
            harness
                .worker
                .input_snapshot()
                .buttons()
                .map(|button| button.kind())
                .collect::<Vec<_>>(),
            [ButtonKind::X]
        );
    }

    #[test]
    fn periodic_tap_timing_saturates_internal_clock_boundaries() {
        assert_eq!(
            periodic_tap_times(Duration::MAX, Duration::from_nanos(1)),
            (u64::MAX, Duration::MAX)
        );
        assert_eq!(
            periodic_tap_times(Duration::from_nanos(u64::MAX), Duration::from_nanos(1)),
            (
                u64::MAX,
                Duration::from_nanos(u64::MAX) + Duration::from_nanos(1)
            )
        );
    }

    #[test]
    fn terminal_periodic_tap_release_preserves_the_first_command_error() {
        let mut harness = PeriodicHarness::ready();
        let started_at = harness.clock.now();
        let timer_before = harness.worker.sender_timer();
        harness
            .control
            .script_sends([ScriptedSendOutcome::Rejected]);
        let mut commands = TracedCommands::new(
            [
                (
                    "tap-b",
                    PeriodicCommand::Common(CommonCommand::Tap {
                        buttons: vec![ProButton::B],
                        duration: Duration::from_millis(80),
                    }),
                ),
                (
                    "press-x",
                    PeriodicCommand::Common(CommonCommand::Press(vec![ProButton::X])),
                ),
            ],
            Arc::clone(&harness.trace),
        );
        let mut no_shutdown = ShutdownLatch::default();

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("a rejected tap press remains pending until release");
        };
        assert!(progress.command_result.is_none());
        assert_eq!(commands.remaining(), 1);
        assert!(harness.worker.has_pending_reporting_command());

        harness.control.script_sends([ScriptedSendOutcome::Closed]);
        harness.clock.set(started_at + Duration::from_millis(80));
        lock(&harness.trace).clear();
        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Failed {
            error: WorkerCoreError::Transport(error),
            progress,
        } = step
        else {
            panic!("terminal release failure must stop the worker");
        };
        assert_eq!(error.kind(), TransportErrorKind::Closed);
        assert_eq!(progress.commands, 0);
        assert_eq!(progress.due_actions, 1);
        assert!(progress.immediate);
        let Some(Err(WorkerCommandError::Periodic(PeriodicError::Transport {
            error: first_error,
            later_terminal: None,
        }))) = progress.command_result.as_ref()
        else {
            panic!("the command completion must retain its first transport error");
        };
        assert_eq!(first_error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(commands.remaining(), 1);
        assert!(!harness.worker.has_pending_reporting_command());
        assert_eq!(harness.worker.sender_timer(), timer_before);
        assert_eq!(harness.worker.input_snapshot(), InputState::neutral());
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Failed);
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Poll(0),
                Trace::Send {
                    report_id: 0x30,
                    timer: timer_before,
                    buttons: buttons([]),
                    accepted: false,
                },
            ]
        );
    }

    #[test]
    fn terminal_periodic_tap_press_failure_does_not_wait_for_release() {
        let mut harness = PeriodicHarness::ready();
        let timer_before = harness.worker.sender_timer();
        harness
            .control
            .terminate_with(std::io::Error::other("sensitive backend detail"))
            .expect("terminate source before the tap command");
        let mut commands = TracedCommands::new(
            [(
                "tap-b",
                PeriodicCommand::Common(CommonCommand::Tap {
                    buttons: vec![ProButton::B],
                    duration: Duration::from_secs(24 * 60 * 60),
                }),
            )],
            Arc::clone(&harness.trace),
        );
        let mut no_shutdown = ShutdownLatch::default();
        lock(&harness.trace).clear();

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Failed {
            error: WorkerCoreError::Transport(error),
            progress,
        } = step
        else {
            panic!("terminal tap press failure must stop without a pending deadline");
        };
        assert_eq!(error.kind(), TransportErrorKind::SourceTerminated);
        assert!(!error.to_string().contains("sensitive"));
        let Some(Err(WorkerCommandError::Periodic(PeriodicError::Transport {
            error: completion_error,
            later_terminal: None,
        }))) = progress.command_result.as_ref()
        else {
            panic!("terminal tap press must retain its command completion");
        };
        assert_eq!(
            completion_error.kind(),
            TransportErrorKind::SourceTerminated
        );
        assert_eq!(commands.remaining(), 0);
        assert!(!harness.worker.has_pending_reporting_command());
        assert_eq!(harness.worker.sender_timer(), timer_before);
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Failed);
        assert_eq!(
            harness
                .worker
                .input_snapshot()
                .buttons()
                .map(|button| button.kind())
                .collect::<Vec<_>>(),
            [ButtonKind::B]
        );
        assert_eq!(
            *lock(&harness.trace),
            [
                Trace::Command("tap-b"),
                Trace::Send {
                    report_id: 0x30,
                    timer: timer_before,
                    buttons: buttons([ButtonKind::B]),
                    accepted: false,
                },
            ]
        );
    }

    struct PeriodicHarness {
        worker: WorkerCore<Pro, Periodic>,
        control: FakeTransportControl,
        clock: FakeClock,
        trace: Arc<Mutex<Vec<Trace>>>,
    }

    impl PeriodicHarness {
        fn ready() -> Self {
            let (transport, control, trace) = tracing_transport();
            let clock = FakeClock::at(Duration::ZERO);
            let observer = observer(Arc::clone(&trace));
            let mut worker = WorkerCore::new_periodic(
                protocol(),
                Box::new(transport),
                REPORT_PERIOD,
                WorkerBudget::new(1),
                observer,
            )
            .expect("valid Periodic worker");
            prime_ready(&mut worker, &control, &clock, &trace);
            Self {
                worker,
                control,
                clock,
                trace,
            }
        }
    }

    struct DirectHarness {
        worker: WorkerCore<Pro, Direct>,
        control: FakeTransportControl,
        clock: FakeClock,
        trace: Arc<Mutex<Vec<Trace>>>,
    }

    impl DirectHarness {
        fn ready() -> Self {
            let (transport, control, trace) = tracing_transport();
            let clock = FakeClock::at(Duration::ZERO);
            let observer = observer(Arc::clone(&trace));
            let mut worker = WorkerCore::new_direct(
                protocol(),
                Box::new(transport),
                WorkerBudget::new(1),
                observer,
            );
            prime_ready(&mut worker, &control, &clock, &trace);
            Self {
                worker,
                control,
                clock,
                trace,
            }
        }
    }

    fn prime_ready<R: crate::runtime::worker::WorkerReporting<Pro>>(
        worker: &mut WorkerCore<Pro, R>,
        control: &FakeTransportControl,
        clock: &FakeClock,
        trace: &Arc<Mutex<Vec<Trace>>>,
    ) {
        worker
            .begin_connection(clock.now(), CONNECTION_TIMEOUT)
            .expect("begin fake connection");
        control.inject_connected().expect("link event");
        control
            .inject_hid_channel_opened(HidChannel::Control)
            .expect("control channel");
        control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .expect("interrupt channel");
        let mut commands = TracedCommands::new([], Arc::clone(trace));
        let mut shutdown = ShutdownLatch::default();
        assert!(matches!(
            worker.step(clock, &mut shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));

        clock.set(Duration::from_millis(10));
        control
            .inject_hid_output(HidChannel::Control, &subcommand_report(0x03, &[0x30]))
            .expect("report mode");
        control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x30, &[0x01]))
            .expect("player lights");
        assert!(matches!(
            worker.step(clock, &mut shutdown, &mut commands),
            WorkerStep::Continue(_)
        ));

        if worker.lifecycle_state() != LifecycleState::Ready {
            clock.set(Duration::from_millis(310));
            assert!(matches!(
                worker.step(clock, &mut shutdown, &mut commands),
                WorkerStep::Continue(_)
            ));
        }
        assert_eq!(worker.lifecycle_state(), LifecycleState::Ready);
        lock(trace).clear();
    }

    struct TracedCommands<C> {
        queued: VecDeque<(&'static str, C)>,
        polls: usize,
        trace: Arc<Mutex<Vec<Trace>>>,
    }

    impl<C> TracedCommands<C> {
        fn new(
            commands: impl IntoIterator<Item = (&'static str, C)>,
            trace: Arc<Mutex<Vec<Trace>>>,
        ) -> Self {
            Self {
                queued: commands.into_iter().collect(),
                polls: 0,
                trace,
            }
        }

        fn remaining(&self) -> usize {
            self.queued.len()
        }

        fn polls(&self) -> usize {
            self.polls
        }
    }

    impl<C> CommandSource<C> for TracedCommands<C> {
        fn try_next(&mut self) -> Option<C> {
            self.polls += 1;
            self.queued.pop_front().map(|(name, command)| {
                lock(&self.trace).push(Trace::Command(name));
                command
            })
        }
    }

    #[derive(Default)]
    struct ShutdownLatch {
        request: Option<ShutdownRequest>,
        checks_before_request: usize,
    }

    impl ShutdownLatch {
        const fn new(request: ShutdownRequest) -> Self {
            Self {
                request: Some(request),
                checks_before_request: 0,
            }
        }

        const fn after_checks(request: ShutdownRequest, checks_before_request: usize) -> Self {
            Self {
                request: Some(request),
                checks_before_request,
            }
        }
    }

    impl PriorityShutdown for ShutdownLatch {
        fn take(&mut self) -> Option<ShutdownRequest> {
            if self.request.is_some() && self.checks_before_request > 0 {
                self.checks_before_request -= 1;
                return None;
            }
            self.request.take()
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

        fn set(&self, now: Duration) {
            let mut current = lock(&self.now);
            assert!(
                now >= *current,
                "fake monotonic clock cannot move backwards"
            );
            *current = now;
        }
    }

    impl MonotonicClock for FakeClock {
        fn now(&self) -> Duration {
            *lock(&self.now)
        }
    }

    struct ScriptedWaiter {
        clock: FakeClock,
        requests: Vec<WorkerWaitRequest>,
    }

    impl ScriptedWaiter {
        fn new(clock: FakeClock) -> Self {
            Self {
                clock,
                requests: Vec::new(),
            }
        }
    }

    impl WorkerWaiter for ScriptedWaiter {
        fn wait(
            &mut self,
            request: WorkerWaitRequest,
            _clock: &dyn MonotonicClock,
        ) -> Result<(), WorkerWaitError> {
            self.requests.push(request);
            if let WorkerWaitRequest::ActivityOrDeadline(deadline) = request {
                self.clock.set(deadline);
            }
            Ok(())
        }
    }

    fn progress_for_wait(immediate: bool, next_deadline: Option<Duration>) -> StepProgress {
        let mut progress = StepProgress::new();
        progress.immediate = immediate;
        progress.next_deadline = next_deadline;
        progress
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Trace {
        Command(&'static str),
        StartPairing,
        Poll(usize),
        Observe {
            channel: HidChannel,
            report_id: u8,
        },
        Send {
            report_id: u8,
            timer: u8,
            buttons: [u8; 3],
            accepted: bool,
        },
        Drain,
        Disconnect,
        TransportClose,
    }

    struct TracingTransport {
        inner: FakeTransport,
        trace: Arc<Mutex<Vec<Trace>>>,
        start_pairing_error: Option<TransportErrorKind>,
    }

    impl TransportPort for TracingTransport {
        fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
            self.inner.open(activity)
        }

        fn start_pairing(&mut self) -> TransportResult<()> {
            lock(&self.trace).push(Trace::StartPairing);
            match self.start_pairing_error {
                Some(kind) => Err(crate::runtime::transport::TransportError::new(kind)),
                None => self.inner.start_pairing(),
            }
        }

        fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            let events = self.inner.poll(timeout)?;
            lock(&self.trace).push(Trace::Poll(events.len()));
            Ok(events)
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
            let result = self.inner.send_interrupt(payload);
            lock(&self.trace).push(Trace::Send {
                report_id: payload[0],
                timer: payload[1],
                buttons: [payload[3], payload[4], payload[5]],
                accepted: result.is_ok(),
            });
            result
        }

        fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
            lock(&self.trace).push(Trace::Drain);
            self.inner.drain_interrupt(timeout)
        }

        fn disconnect(&mut self) -> TransportResult<()> {
            lock(&self.trace).push(Trace::Disconnect);
            self.inner.disconnect()
        }

        fn close(&mut self) -> TransportResult<()> {
            lock(&self.trace).push(Trace::TransportClose);
            self.inner.close()
        }
    }

    fn tracing_transport() -> (
        TracingTransport,
        FakeTransportControl,
        Arc<Mutex<Vec<Trace>>>,
    ) {
        let (mut inner, control) = FakeTransport::with_limits(16, 3);
        let (notifier, _receiver) = activity_channel();
        inner.open(notifier).expect("open fake transport");
        let trace = Arc::new(Mutex::new(Vec::new()));
        (
            TracingTransport {
                inner,
                trace: Arc::clone(&trace),
                start_pairing_error: None,
            },
            control,
            trace,
        )
    }

    fn observer(
        trace: Arc<Mutex<Vec<Trace>>>,
    ) -> Box<dyn FnMut(OutputObservation) + Send + 'static> {
        Box::new(move |observation| {
            lock(&trace).push(Trace::Observe {
                channel: observation.channel,
                report_id: observation.report_id,
            });
        })
    }

    fn buttons(kinds: impl IntoIterator<Item = ButtonKind>) -> [u8; 3] {
        let state = InputState::<Pro>::neutral().with_buttons(
            kinds
                .into_iter()
                .map(|kind| ProButton::try_from(kind).expect("button supported by Pro")),
        );
        let report = protocol().prepare_input_report(&state, 0, Default::default(), 0);
        report.bytes()[3..6].try_into().expect("three button bytes")
    }

    fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut raw = vec![0x01, 0];
        raw.extend_from_slice(&NEUTRAL_RUMBLE);
        raw.push(subcommand_id);
        raw.extend_from_slice(payload);
        raw
    }

    fn rumble_report() -> Vec<u8> {
        let mut raw = vec![0x10, 0];
        raw.extend_from_slice(&NEUTRAL_RUMBLE);
        raw
    }

    fn protocol() -> SwitchHidProtocol<Pro> {
        SwitchHidProtocol::new(None, DEVICE_INFO_ADDRESS)
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}
