use std::{marker::PhantomData, sync::mpsc::Receiver, time::Duration};

use crate::{
    error::{Error, ErrorKind},
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    reporting::{self, ReportingMode},
    runtime::{
        cleanup::{CleanupFailure, CleanupPhase, CloseMode},
        command::{CommandEnqueueError, CommandResponse, CommandResponseError},
        error_map::{
            map_cleanup_error, map_command_error, map_enqueue_error, map_response_error,
            map_worker_outcome,
        },
        status::StatusPublisher,
        transport::{ActivityNotifier, TransportError, TransportPort, activity_channel},
        worker::{
            MonotonicClock, PairingError, RuntimeCommand, WorkerBudget, WorkerCommandError,
            WorkerReporting, WorkerWaiter,
        },
        worker_thread::{
            WorkerOwner, WorkerSpawnError, priority_shutdown_channel, spawn_worker_thread,
        },
    },
};

use super::{
    config::{BuilderConfig, ControllerConfig},
    create::{
        CreateProfileRuntimeAttempt, CreateProfileRuntimeBackend, ReadyRuntime, ReadyRuntimePort,
    },
};

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M3 supplies the first non-test concrete runtime factory"
    )
)]
const COMMAND_CAPACITY: usize = 16;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M3 supplies the first non-test concrete runtime factory"
    )
)]
const COMMAND_BATCH: usize = 16;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M3 supplies the first non-test concrete runtime factory"
    )
)]
const POLL_BATCHES: usize = 4;
const UNOWNED_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T33 test injection drives pairing before M3 supplies a backend"
    )
)]
pub(super) trait PairDriver {
    fn after_pair_enqueued(&mut self) -> crate::Result<()>;
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T33 test injection supplies concrete resources before M3"
    )
)]
pub(super) struct RuntimeComponents<C, W, D> {
    transport: Box<dyn TransportPort>,
    clock: C,
    waiter: W,
    pair_driver: D,
    device_info_address: [u8; 6],
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T33 test injection constructs components before M3"
    )
)]
impl<C, W, D> RuntimeComponents<C, W, D> {
    pub(super) fn new(
        transport: Box<dyn TransportPort>,
        clock: C,
        waiter: W,
        pair_driver: D,
        device_info_address: [u8; 6],
    ) -> Self {
        Self {
            transport,
            clock,
            waiter,
            pair_driver,
            device_info_address,
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T33 tests inject the concrete backend before M3 supplies its factory"
    )
)]
pub(super) struct ConcreteRuntimeBackend<F> {
    factory: Option<F>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T33 tests inject the concrete backend before M3 supplies its factory"
    )
)]
impl<F> ConcreteRuntimeBackend<F> {
    pub(super) const fn new(factory: F) -> Self {
        Self {
            factory: Some(factory),
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T33 tests exercise the concrete attempt before M3 supplies a backend"
    )
)]
pub(super) struct ConcreteRuntimeAttempt<M, R, F, C, W, D>
where
    M: ControllerModel,
    R: ReportingMode,
{
    factory: Option<F>,
    status: StatusPublisher<M>,
    owner: Option<WorkerOwner<RuntimeCommand<M, R>>>,
    unowned_transport: Option<Box<dyn TransportPort>>,
    pair_driver: Option<D>,
    _resources: PhantomData<fn() -> (C, W)>,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T33 tests construct the concrete attempt before M3 supplies a backend"
    )
)]
impl<M, R, F, C, W, D> ConcreteRuntimeAttempt<M, R, F, C, W, D>
where
    M: ControllerModel,
    R: ReportingMode,
{
    pub(super) fn new(factory: F, status: StatusPublisher<M>) -> Self {
        Self {
            factory: Some(factory),
            status,
            owner: None,
            unowned_transport: None,
            pair_driver: None,
            _resources: PhantomData,
        }
    }

    #[cfg(test)]
    pub(super) fn worker_is_finished(&self) -> bool {
        self.owner.as_ref().is_some_and(WorkerOwner::is_finished)
    }
}

impl<M, R, F, C, W, D> CreateProfileRuntimeBackend<M, R> for ConcreteRuntimeBackend<F>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    F: FnOnce(ActivityNotifier, Receiver<()>) -> crate::Result<RuntimeComponents<C, W, D>>,
    C: MonotonicClock + 'static,
    W: WorkerWaiter + 'static,
    D: PairDriver,
{
    type Attempt = ConcreteRuntimeAttempt<M, R, F, C, W, D>;

    fn ensure_supported(&mut self, _config: &BuilderConfig<M, R>) -> crate::Result<()> {
        if self.factory.is_some() {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::WorkerFailed,
                "concrete runtime backend has already been consumed",
            ))
        }
    }

    fn begin_attempt(&mut self, status: StatusPublisher<M>) -> Self::Attempt {
        let factory = self
            .factory
            .take()
            .expect("supported concrete runtime retains one factory");
        ConcreteRuntimeAttempt::new(factory, status)
    }
}

impl<M, R, F, C, W, D> CreateProfileRuntimeAttempt<M, R>
    for ConcreteRuntimeAttempt<M, R, F, C, W, D>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    F: FnOnce(ActivityNotifier, Receiver<()>) -> crate::Result<RuntimeComponents<C, W, D>>,
    C: MonotonicClock + 'static,
    W: WorkerWaiter + 'static,
    D: PairDriver,
{
    fn open(&mut self, config: &ControllerConfig<M, R>) -> crate::Result<()> {
        if self.owner.is_some() || self.unowned_transport.is_some() || self.pair_driver.is_some() {
            return Err(Error::new(
                ErrorKind::WorkerFailed,
                "runtime attempt is already open",
            ));
        }
        let factory = self.factory.take().ok_or_else(|| {
            Error::new(
                ErrorKind::WorkerFailed,
                "runtime attempt factory is unavailable",
            )
        })?;
        let (activity, activity_receiver) = activity_channel();
        let RuntimeComponents {
            transport,
            clock,
            waiter,
            pair_driver,
            device_info_address,
        } = factory(activity.clone(), activity_receiver)?;

        self.unowned_transport = Some(transport);
        if let Err(source) = self
            .unowned_transport
            .as_deref_mut()
            .expect("runtime attempt retains its transport during open")
            .open(activity.clone())
        {
            return Err(Error::with_source(
                ErrorKind::ConnectionFailed,
                "controller transport could not be opened",
                source,
            ));
        }
        let transport = self
            .unowned_transport
            .take()
            .expect("opened attempt retains its transport until worker transfer");
        let protocol = SwitchHidProtocol::new(Some(config.colors), device_info_address);
        let worker = R::build_worker(
            protocol,
            transport,
            &config.mode,
            WorkerBudget::new(COMMAND_BATCH, POLL_BATCHES),
            Box::new(|_| {}),
            self.status.clone(),
        );
        let (commands, command_receiver) = crate::runtime::command::command_channel::<
            RuntimeCommand<M, R>,
        >(COMMAND_CAPACITY, activity.clone());
        let (shutdown, shutdown_receiver) = priority_shutdown_channel(activity);
        let thread =
            spawn_worker_thread(worker, clock, shutdown_receiver, command_receiver, waiter)
                .map_err(|error| {
                    self.status.fail("worker spawn failed");
                    map_worker_spawn_error(error)
                })?;

        self.owner = Some(WorkerOwner::new(commands, shutdown, thread));
        self.pair_driver = Some(pair_driver);
        Ok(())
    }

    fn pair_to_ready(&mut self, pair_timeout: Duration) -> crate::Result<()> {
        let enqueue = self
            .owner
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::WorkerFailed, "runtime attempt is not open"))?
            .try_enqueue(RuntimeCommand::Pair {
                timeout: pair_timeout,
            });
        let response = match enqueue {
            Ok(response) => response,
            Err(error @ CommandEnqueueError::Disconnected) => {
                return finish_terminal_owner(&mut self.owner, map_enqueue_error(error));
            }
            Err(error) => return Err(map_enqueue_error(error)),
        };
        self.pair_driver
            .as_mut()
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::WorkerFailed,
                    "runtime pair driver is unavailable",
                )
            })?
            .after_pair_enqueued()?;
        match response.recv() {
            Ok(Err(error @ WorkerCommandError::Pair(PairingError::WorkerFailed))) => {
                finish_terminal_owner(&mut self.owner, map_command_error(error))
            }
            Err(error @ CommandResponseError::WorkerFailed) => {
                finish_terminal_owner(&mut self.owner, map_response_error(error))
            }
            Ok(result) => result.map_err(map_command_error),
        }
    }

    fn cleanup_without_neutral(mut self) -> crate::Result<()> {
        if let Some(owner) = self.owner.take() {
            return map_worker_outcome(owner.finish_explicit(CloseMode::WithoutNeutral));
        }
        let Some(mut transport) = self.unowned_transport.take() else {
            return Ok(());
        };
        cleanup_unowned_transport(transport.as_mut())
    }

    fn into_ready(mut self) -> ReadyRuntime<M, R> {
        let owner = self
            .owner
            .take()
            .expect("paired runtime attempt retains its worker owner");
        ReadyRuntime::from_port(owner)
    }
}

impl<M, R, F, C, W, D> Drop for ConcreteRuntimeAttempt<M, R, F, C, W, D>
where
    M: ControllerModel,
    R: ReportingMode,
{
    fn drop(&mut self) {
        if let Some(mut transport) = self.unowned_transport.take() {
            cleanup_unowned_transport_for_drop(transport.as_mut());
        }
    }
}

impl<M, R> ReadyRuntimePort<M, R> for WorkerOwner<RuntimeCommand<M, R>>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    fn request(
        &mut self,
        command: <R as reporting::sealed::Sealed>::Command<M>,
    ) -> crate::Result<()> {
        let response = self
            .try_enqueue(RuntimeCommand::Input(command))
            .map_err(map_enqueue_error)?;
        receive_response(response)
    }

    fn close(self: Box<Self>, mode: CloseMode) -> crate::Result<()> {
        map_worker_outcome((*self).finish_explicit(mode))
    }
}

fn receive_response(response: CommandResponse) -> crate::Result<()> {
    response
        .recv()
        .map_err(map_response_error)?
        .map_err(map_command_error)
}

fn finish_terminal_owner<C>(
    owner: &mut Option<WorkerOwner<C>>,
    fallback: Error,
) -> crate::Result<()> {
    let Some(owner) = owner.take() else {
        return Err(fallback);
    };
    match map_worker_outcome(owner.finish_terminal()) {
        Ok(()) => Err(fallback),
        Err(error) => Err(error),
    }
}

pub(super) fn map_worker_spawn_error(error: WorkerSpawnError) -> Error {
    let (source, cleanup) = error.into_parts();
    let mut error = Error::with_source(
        ErrorKind::WorkerFailed,
        "controller worker thread could not be started",
        source,
    );
    if let Some(cleanup) = cleanup {
        error = error.with_related(map_cleanup_error(cleanup));
    }
    error
}

fn cleanup_unowned_transport(transport: &mut dyn TransportPort) -> crate::Result<()> {
    let mut first_failure = None;
    record_cleanup_failure(
        &mut first_failure,
        CleanupPhase::DrainInterrupt,
        transport.drain_interrupt(UNOWNED_DRAIN_TIMEOUT),
    );
    record_cleanup_failure(
        &mut first_failure,
        CleanupPhase::Disconnect,
        transport.disconnect(),
    );
    record_cleanup_failure(
        &mut first_failure,
        CleanupPhase::TransportClose,
        transport.close(),
    );
    first_failure.map_or(Ok(()), |failure| Err(map_cleanup_error(failure)))
}

fn cleanup_unowned_transport_for_drop(transport: &mut dyn TransportPort) {
    let _ = transport.disconnect();
    let _ = transport.close();
}

fn record_cleanup_failure(
    first_failure: &mut Option<CleanupFailure>,
    phase: CleanupPhase,
    result: Result<(), TransportError>,
) {
    if first_failure.is_none() {
        if let Err(error) = result {
            *first_failure = Some(CleanupFailure::new(phase, error));
        }
    }
}
