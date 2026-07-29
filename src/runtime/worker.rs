use std::{
    marker::PhantomData,
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::Duration,
};

use crate::{
    controller::input::{neutral_candidate, press_candidate, release_candidate, tap_plan},
    error::Error,
    input::{Button, InputState},
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    reporting::{Direct, Periodic, ReportingMode},
    runtime::{
        cleanup::{CleanupContext, CleanupSequence, CloseCompletion, CloseMode},
        connection::ObservedSubcommands,
        direct::{
            DirectTapContext, DirectTapError, DirectTapStep, DirectTapStimulus, PendingDirectTap,
            begin_tap as begin_direct_tap, send_candidate as send_direct,
        },
        handshake::{Handshake, HandshakeError, HandshakeProgress},
        lifecycle::{
            LifecycleAction, LifecycleCommandError, LifecycleState, LifecycleStateMachine,
        },
        output::{
            OutputHandling, OutputHandlingContext, OutputHandlingError, OutputObservation,
            handle_output,
        },
        periodic::{
            AutomaticInput, PendingPeriodicTap, PeriodicError, PeriodicPolicy,
            begin_tap as begin_periodic_tap, commit_candidate as commit_periodic,
        },
        readiness::{ReadinessError, ReadinessGate, ReadinessProgress},
        scheduler::SchedulerError,
        sender::ReportSender,
        session::{ConnectionSessionId, ConnectionSessions, SessionError, SessionEvent},
        state::InputStateStore,
        transport::{TransportError, TransportErrorKind, TransportEvent, TransportPort},
    },
};

const EXPLICIT_CLOSE_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

pub(crate) trait MonotonicClock {
    fn now(&self) -> Duration;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerWaitRequest {
    Activity,
    ActivityOrDeadline(Duration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerWaitError {
    Disconnected,
}

pub(crate) trait WorkerWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError>;
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 worker thread construction owns the activity receiver"
    )
)]
pub(crate) struct ChannelWorkerWaiter {
    receiver: Receiver<()>,
}

impl ChannelWorkerWaiter {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T24 worker thread construction owns the activity receiver"
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
pub(crate) struct ExplicitCloseRequest {
    mode: CloseMode,
}

impl ExplicitCloseRequest {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "T25 and T33 construct priority close requests")
    )]
    pub(crate) const fn new(mode: CloseMode) -> Self {
        Self { mode }
    }
}

pub(crate) trait PriorityShutdown {
    fn take(&mut self) -> Option<ExplicitCloseRequest>;
}

pub(crate) trait CommandSource<C> {
    fn try_next(&mut self) -> Option<C>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WorkerBudget {
    command_batch: usize,
    poll_batches: usize,
}

impl WorkerBudget {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "T33 configures bounded worker batches")
    )]
    pub(crate) const fn new(command_batch: usize, poll_batches: usize) -> Self {
        assert!(command_batch > 0, "worker command batch must be positive");
        assert!(poll_batches > 0, "worker poll batch count must be positive");
        Self {
            command_batch,
            poll_batches,
        }
    }
}

#[allow(
    dead_code,
    reason = "T33 controller input methods construct the complete command surface"
)]
pub(crate) enum CommonCommand<M: ControllerModel> {
    Press(Vec<Button<M>>),
    Release(Vec<Button<M>>),
    Tap {
        buttons: Vec<Button<M>>,
        duration: Duration,
    },
    Neutral,
}

#[allow(
    dead_code,
    reason = "T33 Periodic controller methods construct reporting-specific commands"
)]
pub(crate) enum PeriodicCommand<M: ControllerModel> {
    Common(CommonCommand<M>),
    Apply(InputState<M>),
}

#[allow(
    dead_code,
    reason = "T33 Direct controller methods construct reporting-specific commands"
)]
pub(crate) enum DirectCommand<M: ControllerModel> {
    Common(CommonCommand<M>),
    Send(InputState<M>),
}

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "T23 and T26 response delivery maps typed command failures"
)]
pub(crate) enum WorkerCommandError {
    Input(Error),
    Lifecycle(LifecycleCommandError),
    Periodic(PeriodicError),
    Direct(DirectTapError),
    ClockOverflow,
    DeadlineOverflow,
    Shutdown,
    Disconnected { reason: Option<u8> },
}

#[derive(Debug)]
pub(crate) enum WorkerCommandProgress {
    Complete(Result<(), WorkerCommandError>),
    Pending,
}

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "T24 and T26 termination handling maps worker operation failures"
)]
pub(crate) enum WorkerOperationError {
    Output(OutputHandlingError),
    Periodic(PeriodicError),
    Readiness(ReadinessError),
}

#[derive(Debug)]
#[allow(
    dead_code,
    reason = "T24 and T26 termination handling maps worker core failures"
)]
pub(crate) enum WorkerCoreError {
    DeadlineOverflow,
    InvalidLifecycle,
    Session(SessionError),
    Handshake(HandshakeError),
    Transport(TransportError),
}

pub(crate) struct StepProgress {
    commands: usize,
    hci_events: usize,
    due_actions: usize,
    skipped_deadlines: u64,
    immediate: bool,
    next_deadline: Option<Duration>,
    command_results: Vec<WorkerCommandProgress>,
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
            command_results: Vec::new(),
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

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T24 worker loop delivers command results after each step"
        )
    )]
    pub(crate) fn take_command_results(&mut self) -> Vec<WorkerCommandProgress> {
        std::mem::take(&mut self.command_results)
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 worker thread loop consumes each completed step"
    )
)]
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

#[allow(
    dead_code,
    reason = "T24 worker loop and join handling consume step outcomes"
)]
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
    type RuntimeState;
    type Command;

    fn begin_session(
        runtime: &mut Self::RuntimeState,
        sessions: &mut ConnectionSessions,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> Result<ConnectionSessionId, SessionError>;

    fn handle_command(
        runtime: &mut Self::RuntimeState,
        command: Self::Command,
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
    completions: Vec<WorkerCommandProgress>,
    errors: Vec<WorkerOperationError>,
}

impl ReportingDue {
    fn none() -> Self {
        Self {
            actions: 0,
            immediate: false,
            completions: Vec::new(),
            errors: Vec::new(),
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 and T33 controller orchestration own the worker core"
    )
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
    connected: bool,
    transport: Box<dyn TransportPort>,
    budget: WorkerBudget,
    observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
    reporting_marker: PhantomData<fn() -> R>,
}

impl<M: ControllerModel> WorkerCore<M, Periodic> {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 and T33 controller orchestration construct the worker core"
        )
    )]
    pub(crate) fn new_periodic(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        period: Duration,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
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
        ))
    }
}

impl<M: ControllerModel> WorkerCore<M, Direct> {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 and T33 controller orchestration construct the worker core"
        )
    )]
    pub(crate) fn new_direct(
        protocol: SwitchHidProtocol<M>,
        transport: Box<dyn TransportPort>,
        budget: WorkerBudget,
        observe_output: Box<dyn FnMut(OutputObservation) + Send + 'static>,
    ) -> Self {
        Self::from_open_transport(
            protocol,
            transport,
            DirectRuntime { pending_tap: None },
            budget,
            observe_output,
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
    ) -> Self {
        let mut lifecycle = LifecycleStateMachine::new();
        let open = lifecycle.request_open();
        debug_assert_eq!(open, Ok(LifecycleAction::OpenTransport));
        let opened = lifecycle.complete_open();
        debug_assert_eq!(opened, LifecycleAction::Opened);
        Self {
            lifecycle,
            input: InputStateStore::new(),
            reporting,
            sender: ReportSender::new(),
            protocol,
            observed: ObservedSubcommands::default(),
            sessions: ConnectionSessions::new(),
            connection: None,
            connected: false,
            transport,
            budget,
            observe_output,
            reporting_marker: PhantomData,
        }
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T21 exposes the logical connection seam before T31 pairing commands"
        )
    )]
    pub(crate) fn begin_connection(
        &mut self,
        now: Duration,
        timeout: Duration,
    ) -> Result<ConnectionSessionId, WorkerCoreError> {
        let operation_deadline = now
            .checked_add(timeout)
            .ok_or(WorkerCoreError::DeadlineOverflow)?;
        if !self.lifecycle.begin_connection() {
            return Err(WorkerCoreError::InvalidLifecycle);
        }
        let session_id = match R::begin_session(
            &mut self.reporting,
            &mut self.sessions,
            &mut self.sender,
            &mut self.observed,
            &mut self.input,
        ) {
            Ok(session_id) => session_id,
            Err(error) => {
                self.lifecycle.mark_connection_ended();
                return Err(WorkerCoreError::Session(error));
            }
        };
        self.connection = Some(ConnectionWork {
            session_id,
            handshake: Some(Handshake::new(session_id)),
            readiness: ReadinessGate::new(session_id, operation_deadline),
        });
        self.connected = false;
        Ok(session_id)
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T24 worker thread loop consumes deterministic steps"
        )
    )]
    pub(crate) fn step(
        &mut self,
        clock: &dyn MonotonicClock,
        shutdown: &mut dyn PriorityShutdown,
        commands: &mut dyn CommandSource<R::Command>,
    ) -> WorkerStep {
        let mut progress = StepProgress::new();
        if let Some(request) = shutdown.take() {
            return self.close(request, clock.now(), progress);
        }

        if !R::has_pending(&self.reporting) {
            for _ in 0..self.budget.command_batch {
                let Some(command) = commands.try_next() else {
                    break;
                };
                progress.commands += 1;
                let result = match self.lifecycle.ensure_input_command() {
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
                    Err(error) => {
                        WorkerCommandProgress::Complete(Err(WorkerCommandError::Lifecycle(error)))
                    }
                };
                let (result, terminal) = match nonterminal_command_progress(result) {
                    Ok(result) => (result, None),
                    Err(termination) => (termination.completion, Some(termination.error)),
                };
                let pending = matches!(result, WorkerCommandProgress::Pending);
                progress.command_results.push(result);
                if let Some(request) = shutdown.take() {
                    return self.close(request, clock.now(), progress);
                }
                if let Some(error) = terminal {
                    return self.fail(error, progress);
                }
                if pending {
                    break;
                }
            }
        }
        if progress.commands == self.budget.command_batch && !R::has_pending(&self.reporting) {
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

        if self.lifecycle.state() == LifecycleState::Ready {
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
            for completion in due.completions {
                match nonterminal_command_progress(completion) {
                    Ok(completion) => progress.command_results.push(completion),
                    Err(termination) => {
                        progress.command_results.push(termination.completion);
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
                transport: self.transport.as_mut(),
            },
        );
        match event {
            ReportingEvent::Passthrough(TransportEvent::Connected) => {
                self.connected = true;
                if let Some(connection) = self.connection.as_mut()
                    && let Some(handshake) = connection.handshake.as_mut()
                {
                    handshake.observe_link(connection.session_id);
                }
                Ok(true)
            }
            ReportingEvent::Passthrough(TransportEvent::HidChannelOpened { channel }) => {
                if let Some(connection) = self.connection.as_mut()
                    && let Some(handshake) = connection.handshake.as_mut()
                {
                    handshake.observe_channel(connection.session_id, channel);
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
            progress
                .command_results
                .push(WorkerCommandProgress::Complete(Err(error)));
            progress.immediate = true;
        }
        if let Some(mut connection) = self.connection.take() {
            let error = connection.readiness.abort(
                &mut connection.handshake,
                ReadinessError::Disconnected { reason },
            );
            progress
                .operation_errors
                .push(WorkerOperationError::Readiness(error));
        }
        if let Some(session_id) = self.sessions.current() {
            self.sessions.end_current(session_id);
        }
        self.connected = false;
        R::stop_session(&mut self.reporting);
        self.lifecycle.mark_connection_ended();
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
                if ready.session_id() != connection.session_id || !self.lifecycle.mark_ready(ready)
                {
                    self.connection = Some(connection);
                    return Err(WorkerCoreError::InvalidLifecycle);
                }
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
        progress
            .operation_errors
            .push(WorkerOperationError::Readiness(error));
        if let Some(session_id) = self.sessions.current() {
            self.sessions.end_current(session_id);
        }
        self.connected = false;
        R::stop_session(&mut self.reporting);
        self.lifecycle.mark_connection_ended();
    }

    fn close(
        &mut self,
        request: ExplicitCloseRequest,
        now: Duration,
        progress: StepProgress,
    ) -> WorkerStep {
        let interrupted = R::cancel_for_shutdown(
            &mut self.reporting,
            ReportingEventContext {
                observe_output: self.observe_output.as_mut(),
                protocol: &self.protocol,
                input: &mut self.input,
                observed: &mut self.observed,
                sender: &mut self.sender,
                transport: self.transport.as_mut(),
            },
        );
        R::stop_session(&mut self.reporting);
        let now_ns = u64::try_from(now.as_nanos()).unwrap_or(u64::MAX);
        let completion =
            CleanupSequence::new(request.mode, EXPLICIT_CLOSE_DRAIN_TIMEOUT).run(CleanupContext {
                connected: self.connected,
                now_ns,
                lifecycle: &mut self.lifecycle,
                protocol: &self.protocol,
                sender: &mut self.sender,
                transport: self.transport.as_mut(),
            });
        self.connected = false;
        self.connection = None;
        WorkerStep::Closed {
            completion,
            interrupted,
            progress,
        }
    }

    fn fail(&mut self, error: WorkerCoreError, progress: StepProgress) -> WorkerStep {
        self.lifecycle.mark_failed();
        WorkerStep::Failed { error, progress }
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

impl<M: ControllerModel> WorkerReporting<M> for Periodic {
    type RuntimeState = PeriodicRuntime<M>;
    type Command = PeriodicCommand<M>;

    fn begin_session(
        runtime: &mut Self::RuntimeState,
        sessions: &mut ConnectionSessions,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> Result<ConnectionSessionId, SessionError> {
        sessions.begin_periodic(sender, &mut runtime.policy, observed, input)
    }

    fn handle_command(
        runtime: &mut Self::RuntimeState,
        command: Self::Command,
        context: ReportingCommandContext<'_, M>,
    ) -> WorkerCommandProgress {
        match command {
            PeriodicCommand::Apply(candidate) => {
                commit_periodic(candidate, context.input);
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
                commit_periodic(neutral_candidate(), context.input);
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
                let (now_ns, release_at) = match periodic_tap_times(context.now, plan.duration()) {
                    Ok(times) => times,
                    Err(error) => {
                        return WorkerCommandProgress::Complete(Err(error));
                    }
                };
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
                .map(|_| WorkerCommandError::Disconnected { reason });
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
            let now_ns = match u64::try_from(now.as_nanos()) {
                Ok(now_ns) => now_ns,
                Err(_) => {
                    due.completions.push(WorkerCommandProgress::Complete(Err(
                        WorkerCommandError::ClockOverflow,
                    )));
                    return due;
                }
            };
            due.completions.push(WorkerCommandProgress::Complete(
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
            Ok(AutomaticInput::NotDue | AutomaticInput::HeldOff { .. }) => {}
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

fn periodic_tap_times(
    now: Duration,
    duration: Duration,
) -> Result<(u64, Duration), WorkerCommandError> {
    let release_at = now
        .checked_add(duration)
        .ok_or(WorkerCommandError::DeadlineOverflow)?;
    let now_ns = u64::try_from(now.as_nanos()).map_err(|_| WorkerCommandError::ClockOverflow)?;
    u64::try_from(release_at.as_nanos()).map_err(|_| WorkerCommandError::ClockOverflow)?;
    Ok((now_ns, release_at))
}

impl<M: ControllerModel> WorkerReporting<M> for Direct {
    type RuntimeState = DirectRuntime<M>;
    type Command = DirectCommand<M>;

    fn begin_session(
        _runtime: &mut Self::RuntimeState,
        sessions: &mut ConnectionSessions,
        sender: &mut ReportSender<M>,
        observed: &mut ObservedSubcommands,
        input: &mut InputStateStore<M>,
    ) -> Result<ConnectionSessionId, SessionError> {
        sessions.begin_direct(sender, observed, input)
    }

    fn handle_command(
        runtime: &mut Self::RuntimeState,
        command: Self::Command,
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
                complete_direct_send(neutral_candidate(), context)
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
        due.completions.push(WorkerCommandProgress::Complete(
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
            commit_periodic(candidate, input);
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
    let now_ns = match u64::try_from(context.now.as_nanos()) {
        Ok(now_ns) => now_ns,
        Err(_) => {
            return WorkerCommandProgress::Complete(Err(WorkerCommandError::ClockOverflow));
        }
    };
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
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::SwitchHidProtocol,
        reporting::{Direct, Periodic},
        runtime::{
            cleanup::CloseMode,
            command::{CommandEnqueueError, command_channel},
            direct::{DirectTapError, DirectTapInterruption},
            lifecycle::LifecycleState,
            output::{OutputHandlingError, OutputObservation},
            periodic::PeriodicError,
            readiness::ReadinessError,
            transport::{
                ActivityNotifier, HidChannel, SendAcceptance, TransportErrorKind, TransportEvent,
                TransportPort, TransportResult, activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
            worker::{
                ChannelWorkerWaiter, CommandSource, CommonCommand, DirectCommand,
                ExplicitCloseRequest, MonotonicClock, PeriodicCommand, PriorityShutdown,
                StepProgress, WorkerBudget, WorkerCommandError, WorkerCommandProgress, WorkerCore,
                WorkerCoreError, WorkerOperationError, WorkerStep, WorkerWaitError,
                WorkerWaitRequest, WorkerWaiter, periodic_tap_times, wait_for_next_iteration,
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
        let (client, mut commands) = command_channel(1, activity);
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
            Err(CommandEnqueueError::Busy)
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
        assert!(matches!(
            progress.command_results.as_slice(),
            [
                WorkerCommandProgress::Pending,
                WorkerCommandProgress::Complete(Ok(()))
            ]
        ));
        commands
            .deliver_progress(&mut progress)
            .expect("deliver worker result to its response");
        assert!(matches!(response.try_recv(), Ok(Ok(()))));
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
            }),
            WorkerBudget::new(2, 1),
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

        let mut shutdown = ShutdownLatch::new(ExplicitCloseRequest::new(CloseMode::WithoutNeutral));
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
        let mut shutdown = ShutdownLatch::new(ExplicitCloseRequest::new(CloseMode::WithoutNeutral));

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
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closed);
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
            ShutdownLatch::after_checks(ExplicitCloseRequest::new(CloseMode::WithoutNeutral), 1);
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
        let [
            WorkerCommandProgress::Complete(Err(WorkerCommandError::Direct(
                DirectTapError::Transport(error),
            ))),
        ] = progress.command_results.as_slice()
        else {
            panic!("the processed command must retain its terminal completion");
        };
        assert_eq!(error.kind(), TransportErrorKind::SourceTerminated);
        assert!(!error.to_string().contains("sensitive"));
        assert_eq!(commands.remaining(), 0);
        assert_eq!(harness.worker.sender_timer(), timer_before);
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closed);
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
    fn bounded_command_batch_yields_to_hid_reply_and_due_report() {
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
        assert_eq!(progress.commands, 2);
        assert_eq!(progress.hci_events, 3);
        assert_eq!(progress.due_actions, 1);
        assert!(progress.immediate);
        assert_eq!(commands.remaining(), 1);
        assert_eq!(commands.polls(), 2);
        assert_eq!(
            harness
                .worker
                .input_snapshot()
                .buttons()
                .map(|button| button.kind())
                .collect::<Vec<_>>(),
            [ButtonKind::A, ButtonKind::B]
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
                Trace::Command("press-b"),
                Trace::Poll(3),
                Trace::Observe {
                    channel: HidChannel::Interrupt,
                    report_id: 0x01,
                },
                Trace::Send {
                    report_id: 0x21,
                    timer: timer_before,
                    buttons: buttons([ButtonKind::A, ButtonKind::B]),
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
                    buttons: buttons([ButtonKind::A, ButtonKind::B]),
                    accepted: true,
                },
            ]
        );

        lock(&harness.trace).clear();
        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);
        let WorkerStep::Continue(progress) = step else {
            panic!("the next batch must keep running");
        };
        assert_eq!(progress.commands, 1);
        assert_eq!(progress.hci_events, 1);
        assert!(progress.immediate);
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
        assert!(
            !lock(&harness.trace)
                .iter()
                .any(|event| matches!(event, Trace::Send { .. }))
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
            ShutdownLatch::after_checks(ExplicitCloseRequest::new(CloseMode::WithoutNeutral), 1);
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
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closed);
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
            WorkerBudget::new(1, 1),
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
    fn terminal_bootstrap_send_failure_marks_the_worker_failed() {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1, 1),
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
            WorkerBudget::new(1, 1),
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
            WorkerBudget::new(1, 1),
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
        let [WorkerOperationError::Readiness(ReadinessError::Disconnected { reason: Some(0x13) })] =
            progress.operation_errors.as_slice()
        else {
            panic!("connecting readiness must complete with the disconnect reason");
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
            ShutdownLatch::after_checks(ExplicitCloseRequest::new(CloseMode::WithoutNeutral), 1);
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
        assert_eq!(harness.worker.lifecycle_state(), LifecycleState::Closed);
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
            WorkerBudget::new(1, 1),
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
        let [WorkerOperationError::Readiness(ReadinessError::TimedOut)] =
            progress.operation_errors.as_slice()
        else {
            panic!("deadline boundary must return the readiness timeout");
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
        assert!(matches!(
            progress.command_results.as_slice(),
            [WorkerCommandProgress::Pending]
        ));
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
        assert!(matches!(
            progress.command_results.as_slice(),
            [WorkerCommandProgress::Complete(Ok(()))]
        ));
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
    fn periodic_tap_timing_limits_are_rejected_before_press() {
        assert!(matches!(
            periodic_tap_times(Duration::MAX, Duration::from_nanos(1)),
            Err(WorkerCommandError::DeadlineOverflow)
        ));
        assert!(matches!(
            periodic_tap_times(Duration::from_nanos(u64::MAX), Duration::from_nanos(1)),
            Err(WorkerCommandError::ClockOverflow)
        ));

        let mut harness = PeriodicHarness::ready();
        harness.clock.set(Duration::MAX);
        let timer_before = harness.worker.sender_timer();
        let mut commands = TracedCommands::new(
            [(
                "tap-b",
                PeriodicCommand::Common(CommonCommand::Tap {
                    buttons: vec![ProButton::B],
                    duration: Duration::from_nanos(1),
                }),
            )],
            Arc::clone(&harness.trace),
        );
        let mut no_shutdown = ShutdownLatch::default();
        lock(&harness.trace).clear();

        let step = harness
            .worker
            .step(&harness.clock, &mut no_shutdown, &mut commands);

        let WorkerStep::Continue(progress) = step else {
            panic!("tap timing validation is a command error");
        };
        assert!(matches!(
            progress.command_results.as_slice(),
            [WorkerCommandProgress::Complete(Err(
                WorkerCommandError::DeadlineOverflow
            ))]
        ));
        assert_eq!(commands.remaining(), 0);
        assert!(!harness.worker.has_pending_reporting_command());
        assert_eq!(harness.worker.sender_timer(), timer_before);
        assert_eq!(harness.worker.input_snapshot(), InputState::neutral());
        assert!(
            !lock(&harness.trace)
                .iter()
                .any(|event| matches!(event, Trace::Send { .. }))
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
        assert!(matches!(
            progress.command_results.as_slice(),
            [WorkerCommandProgress::Pending]
        ));
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
        let [
            WorkerCommandProgress::Complete(Err(WorkerCommandError::Periodic(
                PeriodicError::Transport {
                    error: first_error,
                    later_terminal: None,
                },
            ))),
        ] = progress.command_results.as_slice()
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
        let [
            WorkerCommandProgress::Complete(Err(WorkerCommandError::Periodic(
                PeriodicError::Transport {
                    error: completion_error,
                    later_terminal: None,
                },
            ))),
        ] = progress.command_results.as_slice()
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
                WorkerBudget::new(2, 1),
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
                WorkerBudget::new(2, 1),
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
        request: Option<ExplicitCloseRequest>,
        checks_before_request: usize,
    }

    impl ShutdownLatch {
        const fn new(request: ExplicitCloseRequest) -> Self {
            Self {
                request: Some(request),
                checks_before_request: 0,
            }
        }

        const fn after_checks(request: ExplicitCloseRequest, checks_before_request: usize) -> Self {
            Self {
                request: Some(request),
                checks_before_request,
            }
        }
    }

    impl PriorityShutdown for ShutdownLatch {
        fn take(&mut self) -> Option<ExplicitCloseRequest> {
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
    }

    impl TransportPort for TracingTransport {
        fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
            self.inner.open(activity)
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
