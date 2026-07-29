use std::{
    io,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    thread::{self, JoinHandle},
};

use crate::{
    model::ControllerModel,
    runtime::{
        cleanup::{CloseCompletion, ExplicitCloseError},
        command::{CommandDeliveryError, CommandReceiver},
        worker::{
            MonotonicClock, PriorityShutdown, WorkerCore, WorkerCoreError, WorkerReporting,
            WorkerStep, WorkerWaitError, WorkerWaiter, wait_for_next_iteration,
        },
    },
};

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
            return WorkerThreadOutcome::Failed {
                cause: WorkerFailureCause::CompletionDisconnected,
                delivery_error: None,
                join_error: join.join().err().map(|_| WorkerJoinError::Panicked),
            };
        };

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

            match caught {
                Ok(completion) => {
                    publish_completion(&completion_sender, completion);
                    drop(commands);
                }
                Err(payload) => {
                    publish_completion(
                        &completion_sender,
                        WorkerCompletion {
                            terminal: WorkerTerminal::Failed(WorkerFailureCause::Panicked),
                            delivery_error: None,
                        },
                    );
                    drop(commands);
                    resume_unwind(payload);
                }
            }
        })?;
    Ok(WorkerThread { completion, join })
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
        },
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
                ChannelWorkerWaiter, CommandSource, CommonCommand, DirectCommand,
                ExplicitCloseRequest, MonotonicClock, PriorityShutdown, WorkerBudget,
                WorkerCommandError, WorkerCore, WorkerCoreError, WorkerStep,
            },
        },
    };

    use super::{WorkerFailureCause, WorkerJoinError, WorkerThreadOutcome, spawn_worker_thread};

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
        let mut worker = WorkerCore::new_direct(
            protocol(),
            Box::new(PanickingTransport {
                inner: transport,
                panic_on_poll: Arc::clone(&panic_on_poll),
            }),
            WorkerBudget::new(2, 1),
            Box::new(|_| {}),
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
            ShutdownScript::after_checks(ExplicitCloseRequest::new(CloseMode::WithNeutral), 1);

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
        request: Option<ExplicitCloseRequest>,
        checks_before_request: usize,
    }

    impl ShutdownScript {
        const fn after_checks(request: ExplicitCloseRequest, checks_before_request: usize) -> Self {
            Self {
                request: Some(request),
                checks_before_request,
            }
        }
    }

    impl PriorityShutdown for ShutdownScript {
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
