use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::{
        Arc, Mutex,
        mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use crate::{
    model::ControllerModel,
    runtime::{
        cleanup::{CloseCompletion, ExplicitCloseError},
        command::{CommandClient, CommandDeliveryError, CommandReceiver},
        transport::ActivityNotifier,
        worker::{
            MonotonicClock, PriorityShutdown, ShutdownRequest, WorkerCore, WorkerCoreError,
            WorkerReporting, WorkerStep, WorkerWaitError, WorkerWaiter, wait_for_next_iteration,
        },
    },
};

const DROP_COMPLETION_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Debug)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T26 maps worker thread failures to the public error boundary"
    )
)]
pub(crate) enum WorkerFailureCause {
    Core(WorkerCoreError),
    Wait(WorkerWaitError),
    #[allow(
        dead_code,
        reason = "T26 preserves the typed command-delivery invariant failure"
    )]
    CommandDelivery(CommandDeliveryError),
    Panicked,
    CompletionDisconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T26 maps the joined worker panic without exposing its payload"
    )
)]
pub(crate) enum WorkerJoinError {
    Panicked,
}

#[derive(Debug)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T26 consumes joined worker outcomes at the controller boundary"
    )
)]
pub(crate) enum WorkerThreadOutcome {
    Closed {
        result: Result<(), ExplicitCloseError<WorkerJoinError>>,
        delivery_error: Option<CommandDeliveryError>,
    },
    Failed {
        cause: WorkerFailureCause,
        delivery_error: Option<CommandDeliveryError>,
        join_error: Option<WorkerJoinError>,
    },
}

enum WorkerTerminal {
    Closed(CloseCompletion),
    Failed(WorkerFailureCause),
}

struct WorkerCompletion {
    terminal: WorkerTerminal,
    delivery_error: Option<CommandDeliveryError>,
}

enum WorkerCompletionWait {
    Completed(WorkerCompletion),
    #[allow(
        dead_code,
        reason = "T35 supplies the deterministic timeout outcome without wall-clock waiting"
    )]
    TimedOut,
    Disconnected,
}

trait WorkerCompletionWaiter: Send {
    fn wait(
        &mut self,
        completion: &Receiver<WorkerCompletion>,
        timeout: Duration,
    ) -> WorkerCompletionWait;
}

#[allow(
    dead_code,
    reason = "T31 constructs the production waiter; T25 and T35 inject scripted waiters"
)]
struct ChannelWorkerCompletionWaiter;

impl WorkerCompletionWaiter for ChannelWorkerCompletionWaiter {
    fn wait(
        &mut self,
        completion: &Receiver<WorkerCompletion>,
        timeout: Duration,
    ) -> WorkerCompletionWait {
        match completion.recv_timeout(timeout) {
            Ok(completion) => WorkerCompletionWait::Completed(completion),
            Err(RecvTimeoutError::Timeout) => WorkerCompletionWait::TimedOut,
            Err(RecvTimeoutError::Disconnected) => WorkerCompletionWait::Disconnected,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShutdownState {
    Running,
    Requested(ShutdownRequest),
    Taken,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 constructs the shared priority shutdown channel"
    )
)]
pub(crate) struct PriorityShutdownClient {
    state: Arc<Mutex<ShutdownState>>,
    activity: ActivityNotifier,
}

pub(crate) struct PriorityShutdownReceiver {
    state: Arc<Mutex<ShutdownState>>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 constructs the shared priority shutdown channel"
    )
)]
pub(crate) fn priority_shutdown_channel(
    activity: ActivityNotifier,
) -> (PriorityShutdownClient, PriorityShutdownReceiver) {
    let state = Arc::new(Mutex::new(ShutdownState::Running));
    (
        PriorityShutdownClient {
            state: Arc::clone(&state),
            activity,
        },
        PriorityShutdownReceiver { state },
    )
}

impl PriorityShutdownClient {
    fn request(&self, request: ShutdownRequest) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = match (*state, request) {
            (ShutdownState::Running, _) => {
                *state = ShutdownState::Requested(request);
                true
            }
            (ShutdownState::Requested(ShutdownRequest::Explicit(_)), ShutdownRequest::Dropped) => {
                *state = ShutdownState::Requested(ShutdownRequest::Dropped);
                true
            }
            (
                ShutdownState::Requested(ShutdownRequest::Dropped),
                ShutdownRequest::Explicit(_) | ShutdownRequest::Dropped,
            )
            | (
                ShutdownState::Requested(ShutdownRequest::Explicit(_)),
                ShutdownRequest::Explicit(_),
            )
            | (ShutdownState::Taken, _) => false,
        };
        drop(state);
        if changed {
            self.activity.notify();
        }
        changed
    }
}

impl PriorityShutdown for PriorityShutdownReceiver {
    fn take(&mut self) -> Option<ShutdownRequest> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match *state {
            ShutdownState::Requested(request) => {
                *state = ShutdownState::Taken;
                Some(request)
            }
            ShutdownState::Running | ShutdownState::Taken => None,
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T26 and T31 own the spawned worker until explicit completion"
    )
)]
pub(crate) struct WorkerThread {
    completion: Receiver<WorkerCompletion>,
    join: JoinHandle<()>,
}

impl WorkerThread {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T26 receives completion before consuming the join handle"
        )
    )]
    pub(crate) fn finish(self) -> WorkerThreadOutcome {
        let Self { completion, join } = self;
        let Ok(completion) = completion.recv() else {
            return finish_disconnected(join);
        };
        finish_completed(completion, join)
    }

    fn finish_with_waiter(
        self,
        waiter: &mut dyn WorkerCompletionWaiter,
        timeout: Duration,
    ) -> Option<WorkerThreadOutcome> {
        let Self { completion, join } = self;
        match waiter.wait(&completion, timeout) {
            WorkerCompletionWait::Completed(completion) => Some(finish_completed(completion, join)),
            WorkerCompletionWait::TimedOut | WorkerCompletionWait::Disconnected => None,
        }
    }
}

fn finish_completed(completion: WorkerCompletion, join: JoinHandle<()>) -> WorkerThreadOutcome {
    match completion.terminal {
        WorkerTerminal::Closed(close) => {
            let result = if close.performed() {
                close.finish_with_join(|| join.join().map_err(|_| WorkerJoinError::Panicked))
            } else {
                join.join()
                    .map_err(|_| ExplicitCloseError::Join(WorkerJoinError::Panicked))
            };
            WorkerThreadOutcome::Closed {
                result,
                delivery_error: completion.delivery_error,
            }
        }
        WorkerTerminal::Failed(cause) => WorkerThreadOutcome::Failed {
            cause,
            delivery_error: completion.delivery_error,
            join_error: join.join().err().map(|_| WorkerJoinError::Panicked),
        },
    }
}

fn finish_disconnected(join: JoinHandle<()>) -> WorkerThreadOutcome {
    WorkerThreadOutcome::Failed {
        cause: WorkerFailureCause::CompletionDisconnected,
        delivery_error: None,
        join_error: join.join().err().map(|_| WorkerJoinError::Panicked),
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 owns the command and worker thread handles in the controller"
    )
)]
pub(crate) struct WorkerOwner<C> {
    commands: Option<CommandClient<C>>,
    shutdown: PriorityShutdownClient,
    thread: Option<WorkerThread>,
    drop_timeout: Duration,
    completion_waiter: Box<dyn WorkerCompletionWaiter>,
}

impl<C> WorkerOwner<C> {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "T31 constructs the controller worker owner")
    )]
    #[cfg_attr(
        test,
        allow(dead_code, reason = "T31 constructs the controller worker owner")
    )]
    pub(crate) fn new(
        commands: CommandClient<C>,
        shutdown: PriorityShutdownClient,
        thread: WorkerThread,
    ) -> Self {
        Self {
            commands: Some(commands),
            shutdown,
            thread: Some(thread),
            drop_timeout: DROP_COMPLETION_TIMEOUT,
            completion_waiter: Box::new(ChannelWorkerCompletionWaiter),
        }
    }

    #[cfg(test)]
    fn with_completion_waiter(
        commands: CommandClient<C>,
        shutdown: PriorityShutdownClient,
        thread: WorkerThread,
        drop_timeout: Duration,
        completion_waiter: Box<dyn WorkerCompletionWaiter>,
    ) -> Self {
        Self {
            commands: Some(commands),
            shutdown,
            thread: Some(thread),
            drop_timeout,
            completion_waiter,
        }
    }
}

impl<C> Drop for WorkerOwner<C> {
    fn drop(&mut self) {
        drop(self.commands.take());
        let _ = self.shutdown.request(ShutdownRequest::dropped());
        if let Some(thread) = self.thread.take() {
            let _ = thread.finish_with_waiter(self.completion_waiter.as_mut(), self.drop_timeout);
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 constructs the worker after transport and profile validation"
    )
)]
pub(crate) fn spawn_worker_thread<M, R, C, S, W>(
    mut worker: WorkerCore<M, R>,
    clock: C,
    mut shutdown: S,
    mut commands: CommandReceiver<R::Command>,
    mut waiter: W,
) -> io::Result<WorkerThread>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    C: MonotonicClock + 'static,
    S: PriorityShutdown + 'static,
    W: WorkerWaiter + 'static,
{
    let (completion_sender, completion) = sync_channel(1);
    let status = worker.status_publisher();
    let join = thread::Builder::new()
        .name("swbt-worker".to_owned())
        .spawn(move || {
            let caught = catch_unwind(AssertUnwindSafe(|| {
                run_worker_loop(
                    &mut worker,
                    &clock,
                    &mut shutdown,
                    &mut commands,
                    &mut waiter,
                )
            }));

            let (completion, mut panic_payload) = match caught {
                Ok(completion) => (completion, None),
                Err(payload) => (
                    WorkerCompletion {
                        terminal: WorkerTerminal::Failed(WorkerFailureCause::Panicked),
                        delivery_error: None,
                    },
                    Some(payload),
                ),
            };
            let teardown = catch_unwind(AssertUnwindSafe(move || {
                drop(waiter);
                drop(shutdown);
                drop(clock);
                drop(worker);
            }));
            let teardown_panicked = teardown.is_err();
            if let Err(payload) = teardown {
                if panic_payload.is_none() {
                    panic_payload = Some(payload);
                }
            }
            if teardown_panicked {
                status.fail("worker panicked");
            } else if let WorkerTerminal::Failed(cause) = &completion.terminal {
                status.fail(worker_failure_status(cause));
            }

            publish_completion(&completion_sender, completion);
            drop(commands);

            if let Some(payload) = panic_payload {
                resume_unwind(payload);
            }
        })?;
    Ok(WorkerThread { completion, join })
}

fn worker_failure_status(cause: &WorkerFailureCause) -> &'static str {
    match cause {
        WorkerFailureCause::Core(error) => error.status_message(),
        WorkerFailureCause::Wait(_) => "worker wait failed",
        WorkerFailureCause::CommandDelivery(_) => "worker command delivery failed",
        WorkerFailureCause::Panicked => "worker panicked",
        WorkerFailureCause::CompletionDisconnected => "worker completion disconnected",
    }
}

fn run_worker_loop<M, R, C, S, W>(
    worker: &mut WorkerCore<M, R>,
    clock: &C,
    shutdown: &mut S,
    commands: &mut CommandReceiver<R::Command>,
    waiter: &mut W,
) -> WorkerCompletion
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    C: MonotonicClock,
    S: PriorityShutdown,
    W: WorkerWaiter,
{
    loop {
        match worker.step(clock, shutdown, commands) {
            WorkerStep::Continue(mut progress) => {
                if let Err(error) = commands.deliver_progress(&mut progress) {
                    return WorkerCompletion {
                        terminal: WorkerTerminal::Failed(WorkerFailureCause::CommandDelivery(
                            error,
                        )),
                        delivery_error: None,
                    };
                }
                if let Err(error) = wait_for_next_iteration(&progress, clock, waiter) {
                    return WorkerCompletion {
                        terminal: WorkerTerminal::Failed(WorkerFailureCause::Wait(error)),
                        delivery_error: None,
                    };
                }
            }
            WorkerStep::Closed {
                completion,
                interrupted,
                mut progress,
            } => {
                let mut delivery_error = commands.deliver_progress(&mut progress).err();
                if delivery_error.is_none()
                    && let Some(error) = interrupted
                {
                    delivery_error = commands.deliver_completion(Err(error)).err();
                }
                return WorkerCompletion {
                    terminal: WorkerTerminal::Closed(completion),
                    delivery_error,
                };
            }
            WorkerStep::Failed {
                error,
                mut progress,
            } => {
                let delivery_error = commands.deliver_progress(&mut progress).err();
                return WorkerCompletion {
                    terminal: WorkerTerminal::Failed(WorkerFailureCause::Core(error)),
                    delivery_error,
                };
            }
        }
    }
}

fn publish_completion(sender: &SyncSender<WorkerCompletion>, completion: WorkerCompletion) {
    let _ = sender.send(completion);
}

#[cfg(test)]
mod tests {
    use std::{
        error::Error as StdError,
        fmt,
        sync::{
            Arc, Mutex, MutexGuard,
            atomic::{AtomicBool, Ordering},
            mpsc::{Receiver as MpscReceiver, SyncSender, TryRecvError, sync_channel},
        },
        thread as std_thread,
        time::Duration,
    };

    use crate::{
        input::ProButton,
        model::Pro,
        protocol::SwitchHidProtocol,
        reporting::Direct,
        runtime::{
            cleanup::CloseMode,
            command::{CommandResponseError, command_channel},
            direct::{DirectTapError, DirectTapInterruption},
            transport::{
                ActivityNotifier, HidChannel, SendAcceptance, TransportEvent, TransportPort,
                TransportResult, activity_channel,
                fake::{FakeTransport, FakeTransportControl},
            },
            worker::{
                ChannelWorkerWaiter, CommandSource, CommonCommand, DirectCommand, MonotonicClock,
                PriorityShutdown, ShutdownRequest, WorkerBudget, WorkerCommandError, WorkerCore,
                WorkerCoreError, WorkerStep,
            },
        },
    };

    use super::{
        DROP_COMPLETION_TIMEOUT, WorkerCompletion, WorkerCompletionWait, WorkerCompletionWaiter,
        WorkerFailureCause, WorkerJoinError, WorkerOwner, WorkerThreadOutcome,
        priority_shutdown_channel, spawn_worker_thread,
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
    const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
    const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

    #[test]
    fn core_failure_preserves_completed_response_and_fails_the_queued_waiter() {
        let (activity, activity_receiver) = activity_channel();
        let (mut transport, control) = FakeTransport::with_limits(8, 3);
        transport
            .open(activity.clone())
            .expect("open fake transport");
        let worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1, 1),
            Box::new(|_| {}),
        );
        let (client, commands) = command_channel(2, activity);
        let completed = client
            .try_enqueue(DirectCommand::Common(CommonCommand::Neutral))
            .expect("enqueue first command");
        let waiting = client
            .try_enqueue(DirectCommand::Common(CommonCommand::Neutral))
            .expect("enqueue queued command");
        control
            .terminate_with(TestSourceError)
            .expect("terminate fake source");

        let worker_thread = spawn_worker_thread(
            worker,
            FakeClock::at(Duration::ZERO),
            ShutdownScript::default(),
            commands,
            ChannelWorkerWaiter::new(activity_receiver),
        )
        .expect("spawn worker thread");

        assert!(matches!(
            completed.recv(),
            Ok(Err(WorkerCommandError::Direct(DirectTapError::NotReady)))
        ));
        assert!(matches!(
            waiting.recv(),
            Err(CommandResponseError::WorkerFailed)
        ));

        let WorkerThreadOutcome::Failed {
            cause: WorkerFailureCause::Core(WorkerCoreError::Transport(error)),
            delivery_error: None,
            join_error: None,
        } = worker_thread.finish()
        else {
            panic!("terminal transport failure must be joined");
        };
        assert_eq!(
            error.kind(),
            crate::runtime::transport::TransportErrorKind::SourceTerminated
        );
    }

    #[test]
    fn panic_fails_pending_and_queued_waiters_before_join_collects_the_panic() {
        let (activity, activity_receiver) = activity_channel();
        let (mut transport, control) = FakeTransport::with_limits(16, 3);
        transport
            .open(activity.clone())
            .expect("open fake transport");
        let panic_on_poll = Arc::new(AtomicBool::new(false));
        let clock = FakeClock::at(Duration::ZERO);
        let controller = crate::Controller::<Pro, Direct>::builder("test:worker-thread")
            .build()
            .expect("ephemeral test controller");
        let status = controller.status_publisher();
        let mut worker = WorkerCore::new_direct_with_status(
            protocol(),
            Box::new(PanickingTransport {
                inner: transport,
                panic_on_poll: Arc::clone(&panic_on_poll),
            }),
            WorkerBudget::new(2, 1),
            Box::new(|_| {}),
            status,
        );
        prime_ready(&mut worker, &control, &clock);
        panic_on_poll.store(true, Ordering::Release);

        let (client, commands) = command_channel(2, activity);
        let pending = client
            .try_enqueue(DirectCommand::Common(CommonCommand::Tap {
                buttons: vec![ProButton::B],
                duration: Duration::from_secs(1),
            }))
            .expect("enqueue pending tap");
        let queued = client
            .try_enqueue(DirectCommand::Common(CommonCommand::Neutral))
            .expect("enqueue command behind the tap");

        let worker_thread = spawn_worker_thread(
            worker,
            clock,
            ShutdownScript::default(),
            commands,
            ChannelWorkerWaiter::new(activity_receiver),
        )
        .expect("spawn worker thread");

        assert!(matches!(
            pending.recv(),
            Err(CommandResponseError::WorkerFailed)
        ));
        assert!(matches!(
            queued.recv(),
            Err(CommandResponseError::WorkerFailed)
        ));

        let outcome = worker_thread.finish();
        assert!(matches!(
            outcome,
            WorkerThreadOutcome::Failed {
                cause: WorkerFailureCause::Panicked,
                delivery_error: None,
                join_error: Some(WorkerJoinError::Panicked),
            }
        ));
        assert!(!format!("{outcome:?}").contains("secret panic payload"));
        let status = controller.status();
        assert_eq!(status.lifecycle, crate::LifecycleState::Failed);
        assert!(!status.connected);
        assert_eq!(status.report_mode, None);
        assert_eq!(status.worker_failure.as_deref(), Some("worker panicked"));
        assert!(
            !status
                .worker_failure
                .as_deref()
                .unwrap_or_default()
                .contains("secret panic payload")
        );
    }

    #[test]
    fn teardown_panic_preserves_cleanup_failure_and_replaces_closed_status() {
        let (activity, activity_receiver) = activity_channel();
        let (mut transport, _control) = FakeTransport::with_limits(8, 3);
        transport
            .open(activity.clone())
            .expect("open fake transport");
        let controller = crate::Controller::<Pro, Direct>::builder("test:worker-thread")
            .build()
            .expect("ephemeral test controller");
        let status = controller.status_publisher();
        let worker = WorkerCore::new_direct_with_status(
            protocol(),
            Box::new(CleanupFailureAndPanicOnDropTransport { inner: transport }),
            WorkerBudget::new(1, 1),
            Box::new(|_| {}),
            status,
        );
        let (_client, commands) = command_channel(1, activity);
        let shutdown =
            ShutdownScript::after_checks(ShutdownRequest::explicit(CloseMode::WithoutNeutral), 0);

        let worker_thread = spawn_worker_thread(
            worker,
            FakeClock::at(Duration::ZERO),
            shutdown,
            commands,
            ChannelWorkerWaiter::new(activity_receiver),
        )
        .expect("spawn worker thread");

        let outcome = worker_thread.finish();
        assert!(matches!(
            outcome,
            WorkerThreadOutcome::Closed {
                result: Err(
                    crate::runtime::cleanup::ExplicitCloseError::CleanupAndJoin {
                        join: WorkerJoinError::Panicked,
                        ..
                    }
                ),
                delivery_error: None,
            }
        ));
        assert!(!format!("{outcome:?}").contains("secret teardown panic"));
        let status = controller.status();
        assert_eq!(status.lifecycle, crate::LifecycleState::Failed);
        assert!(!status.connected);
        assert_eq!(status.report_mode, None);
        assert_eq!(status.worker_failure.as_deref(), Some("worker panicked"));
    }

    #[test]
    fn explicit_close_delivers_pending_shutdown_before_completion_and_join() {
        let (activity, activity_receiver) = activity_channel();
        let (mut transport, control) = FakeTransport::with_limits(16, 3);
        transport
            .open(activity.clone())
            .expect("open fake transport");
        let clock = FakeClock::at(Duration::ZERO);
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(2, 1),
            Box::new(|_| {}),
        );
        prime_ready(&mut worker, &control, &clock);
        let (client, commands) = command_channel(1, activity);
        let pending = client
            .try_enqueue(DirectCommand::Common(CommonCommand::Tap {
                buttons: vec![ProButton::B],
                duration: Duration::from_secs(1),
            }))
            .expect("enqueue pending tap");
        let shutdown =
            ShutdownScript::after_checks(ShutdownRequest::explicit(CloseMode::WithNeutral), 1);

        let worker_thread = spawn_worker_thread(
            worker,
            clock,
            shutdown,
            commands,
            ChannelWorkerWaiter::new(activity_receiver),
        )
        .expect("spawn worker thread");

        assert!(matches!(
            pending.recv(),
            Ok(Err(WorkerCommandError::Direct(
                DirectTapError::Interrupted(DirectTapInterruption::Shutdown)
            )))
        ));
        assert!(matches!(
            worker_thread.finish(),
            WorkerThreadOutcome::Closed {
                result: Ok(()),
                delivery_error: None,
            }
        ));
    }

    #[test]
    fn disconnected_activity_source_completes_before_the_worker_is_joined() {
        let (activity, activity_receiver) = activity_channel();
        let (client, commands) = command_channel::<DirectCommand<Pro>>(1, activity.clone());
        drop(client);
        let mut transport = IgnoringActivityTransport;
        transport
            .open(activity.clone())
            .expect("open idle transport");
        drop(activity);
        let worker = WorkerCore::new_direct(
            protocol(),
            Box::new(transport),
            WorkerBudget::new(1, 1),
            Box::new(|_| {}),
        );

        let worker_thread = spawn_worker_thread(
            worker,
            FakeClock::at(Duration::ZERO),
            ShutdownScript::default(),
            commands,
            ChannelWorkerWaiter::new(activity_receiver),
        )
        .expect("spawn worker thread");

        assert!(matches!(
            worker_thread.finish(),
            WorkerThreadOutcome::Failed {
                cause: WorkerFailureCause::Wait(
                    crate::runtime::worker::WorkerWaitError::Disconnected
                ),
                delivery_error: None,
                join_error: None,
            }
        ));
    }

    #[test]
    fn drop_skips_neutral_and_drain_then_joins_the_completed_worker() {
        let (activity, activity_receiver) = activity_channel();
        let (mut transport, control) = FakeTransport::with_limits(16, 3);
        transport
            .open(activity.clone())
            .expect("open fake transport");
        let trace = Arc::new(Mutex::new(Vec::new()));
        let (teardown_started, teardown_started_receiver) = sync_channel(1);
        let (teardown_release, teardown_release_receiver) = sync_channel(1);
        let clock = FakeClock::at(Duration::ZERO);
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(DropTracingTransport {
                inner: transport,
                trace: Arc::clone(&trace),
                teardown_started,
                teardown_release: teardown_release_receiver,
            }),
            WorkerBudget::new(2, 1),
            Box::new(|_| {}),
        );
        prime_ready(&mut worker, &control, &clock);
        lock(&trace).clear();

        let (shutdown_client, shutdown_receiver) = priority_shutdown_channel(activity.clone());
        let (command_client, command_receiver) = command_channel(1, activity);
        let worker_thread = spawn_worker_thread(
            worker,
            clock,
            shutdown_receiver,
            command_receiver,
            ChannelWorkerWaiter::new(activity_receiver),
        )
        .expect("spawn worker thread");
        let requested_timeouts = Arc::new(Mutex::new(Vec::new()));
        let (completion_received, completion_received_receiver) = sync_channel(1);
        let owner = WorkerOwner::with_completion_waiter(
            command_client,
            shutdown_client,
            worker_thread,
            DROP_COMPLETION_TIMEOUT,
            Box::new(RecordingCompletionWaiter {
                requested_timeouts: Arc::clone(&requested_timeouts),
                completion_received,
            }),
        );
        let (drop_finished, drop_finished_receiver) = sync_channel(1);

        let drop_thread = std_thread::spawn(move || {
            drop(owner);
            drop_finished.send(()).expect("report completed owner drop");
        });

        teardown_started_receiver
            .recv()
            .expect("worker reaches resource teardown before completion");
        assert_eq!(
            *lock(&requested_timeouts),
            [DROP_COMPLETION_TIMEOUT],
            "Drop must request the bounded completion wait"
        );
        assert_eq!(
            *lock(&trace),
            [DropTrace::Disconnect, DropTrace::TransportClose],
            "Drop must skip neutral input and interrupt drain"
        );
        assert_eq!(
            completion_received_receiver.try_recv(),
            Err(TryRecvError::Empty),
            "completion must not publish before worker-owned resources finish teardown"
        );
        assert_eq!(
            drop_finished_receiver.try_recv(),
            Err(TryRecvError::Empty),
            "Drop must still be waiting for bounded completion during resource teardown"
        );

        teardown_release
            .send(())
            .expect("release worker thread teardown");
        completion_received_receiver
            .recv()
            .expect("completion follows worker-owned resource teardown");
        drop_finished_receiver
            .recv()
            .expect("owner Drop finishes after join");
        drop_thread.join().expect("join Drop test thread");
    }

    #[test]
    fn drop_request_replaces_only_an_untaken_explicit_shutdown() {
        let (activity, wakes) = activity_channel();
        let (shutdown_client, mut shutdown_receiver) = priority_shutdown_channel(activity);

        assert!(shutdown_client.request(ShutdownRequest::explicit(CloseMode::WithNeutral)));
        assert!(shutdown_client.request(ShutdownRequest::dropped()));
        assert!(
            !shutdown_client.request(ShutdownRequest::explicit(CloseMode::WithoutNeutral)),
            "an explicit request must not replace the higher-priority Drop request"
        );
        assert_eq!(shutdown_receiver.take(), Some(ShutdownRequest::dropped()));
        assert_eq!(shutdown_receiver.take(), None);
        wakes.try_recv().expect("latched shutdown wakes the worker");
        assert_eq!(
            wakes.try_recv(),
            Err(TryRecvError::Empty),
            "wake notifications coalesce while the first token is pending"
        );

        let (activity, _) = activity_channel();
        let (shutdown_client, mut shutdown_receiver) = priority_shutdown_channel(activity);
        let explicit = ShutdownRequest::explicit(CloseMode::WithNeutral);
        assert!(shutdown_client.request(explicit));
        assert_eq!(shutdown_receiver.take(), Some(explicit));
        assert!(
            !shutdown_client.request(ShutdownRequest::dropped()),
            "Drop cannot replace a request that the worker already took"
        );
    }

    fn prime_ready(
        worker: &mut WorkerCore<Pro, Direct>,
        control: &FakeTransportControl,
        clock: &FakeClock,
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
        let mut commands = EmptyCommands;
        let mut shutdown = ShutdownScript::default();
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
    }

    struct EmptyCommands;

    impl<C> CommandSource<C> for EmptyCommands {
        fn try_next(&mut self) -> Option<C> {
            None
        }
    }

    #[derive(Default)]
    struct ShutdownScript {
        request: Option<ShutdownRequest>,
        checks_before_request: usize,
    }

    impl ShutdownScript {
        const fn after_checks(request: ShutdownRequest, checks_before_request: usize) -> Self {
            Self {
                request: Some(request),
                checks_before_request,
            }
        }
    }

    impl PriorityShutdown for ShutdownScript {
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
            assert!(now >= *current, "fake clock cannot move backwards");
            *current = now;
        }
    }

    impl MonotonicClock for FakeClock {
        fn now(&self) -> Duration {
            *lock(&self.now)
        }
    }

    struct PanickingTransport {
        inner: FakeTransport,
        panic_on_poll: Arc<AtomicBool>,
    }

    impl TransportPort for PanickingTransport {
        fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
            self.inner.open(activity)
        }

        fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            assert!(
                !self.panic_on_poll.load(Ordering::Acquire),
                "secret panic payload"
            );
            self.inner.poll(timeout)
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
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

    struct CleanupFailureAndPanicOnDropTransport {
        inner: FakeTransport,
    }

    impl TransportPort for CleanupFailureAndPanicOnDropTransport {
        fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
            self.inner.open(activity)
        }

        fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            self.inner.poll(timeout)
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
            self.inner.send_interrupt(payload)
        }

        fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
            self.inner.drain_interrupt(timeout)
        }

        fn disconnect(&mut self) -> TransportResult<()> {
            Err(crate::runtime::transport::TransportError::new(
                crate::runtime::transport::TransportErrorKind::SourceTerminated,
            ))
        }

        fn close(&mut self) -> TransportResult<()> {
            self.inner.close()
        }
    }

    impl Drop for CleanupFailureAndPanicOnDropTransport {
        fn drop(&mut self) {
            panic!("secret teardown panic");
        }
    }

    struct IgnoringActivityTransport;

    impl TransportPort for IgnoringActivityTransport {
        fn open(&mut self, _activity: ActivityNotifier) -> TransportResult<()> {
            Ok(())
        }

        fn poll(&mut self, _timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            Ok(Vec::new())
        }

        fn send_interrupt(&mut self, _payload: &[u8]) -> TransportResult<SendAcceptance> {
            unreachable!("idle test never sends")
        }

        fn drain_interrupt(&mut self, _timeout: Duration) -> TransportResult<()> {
            Ok(())
        }

        fn disconnect(&mut self) -> TransportResult<()> {
            Ok(())
        }

        fn close(&mut self) -> TransportResult<()> {
            Ok(())
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum DropTrace {
        Send,
        Drain,
        Disconnect,
        TransportClose,
    }

    struct DropTracingTransport {
        inner: FakeTransport,
        trace: Arc<Mutex<Vec<DropTrace>>>,
        teardown_started: SyncSender<()>,
        teardown_release: MpscReceiver<()>,
    }

    impl TransportPort for DropTracingTransport {
        fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
            self.inner.open(activity)
        }

        fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
            self.inner.poll(timeout)
        }

        fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
            lock(&self.trace).push(DropTrace::Send);
            self.inner.send_interrupt(payload)
        }

        fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
            lock(&self.trace).push(DropTrace::Drain);
            self.inner.drain_interrupt(timeout)
        }

        fn disconnect(&mut self) -> TransportResult<()> {
            lock(&self.trace).push(DropTrace::Disconnect);
            self.inner.disconnect()
        }

        fn close(&mut self) -> TransportResult<()> {
            lock(&self.trace).push(DropTrace::TransportClose);
            self.inner.close()
        }
    }

    impl Drop for DropTracingTransport {
        fn drop(&mut self) {
            self.teardown_started
                .send(())
                .expect("report worker thread teardown");
            self.teardown_release
                .recv()
                .expect("test releases worker thread teardown");
        }
    }

    struct RecordingCompletionWaiter {
        requested_timeouts: Arc<Mutex<Vec<Duration>>>,
        completion_received: SyncSender<()>,
    }

    impl WorkerCompletionWaiter for RecordingCompletionWaiter {
        fn wait(
            &mut self,
            completion: &MpscReceiver<WorkerCompletion>,
            timeout: Duration,
        ) -> WorkerCompletionWait {
            lock(&self.requested_timeouts).push(timeout);
            match completion.recv() {
                Ok(completion) => {
                    self.completion_received
                        .send(())
                        .expect("record completion reception");
                    WorkerCompletionWait::Completed(completion)
                }
                Err(_) => WorkerCompletionWait::Disconnected,
            }
        }
    }

    #[derive(Debug)]
    struct TestSourceError;

    impl fmt::Display for TestSourceError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("test source stopped")
        }
    }

    impl StdError for TestSourceError {}

    fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
        let mut raw = vec![0x01, 0];
        raw.extend_from_slice(&NEUTRAL_RUMBLE);
        raw.push(subcommand_id);
        raw.extend_from_slice(payload);
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
