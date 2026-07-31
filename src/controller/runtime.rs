use std::{marker::PhantomData, sync::mpsc::Receiver, time::Duration};

#[cfg(feature = "bumble")]
use std::time::Instant;

use crate::{
    diagnostics::event::WorkerFailureCategory,
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
        transport::{
            ActivityNotifier, TransportError, TransportErrorKind, TransportPort, activity_channel,
        },
        worker::{
            MonotonicClock, PairingError, RuntimeCommand, WorkerBudget, WorkerCommandError,
            WorkerReporting, WorkerWaiter,
        },
        worker_thread::{
            WorkerOwner, WorkerSpawnError, priority_shutdown_channel, spawn_worker_thread,
        },
    },
};

#[cfg(any(test, feature = "bumble"))]
use super::create::with_cleanup_error;
#[cfg(feature = "bumble")]
use crate::runtime::transport::{ProfileKeyStoreFactory, TransportConfig};
#[cfg(feature = "bumble")]
use crate::runtime::worker::ChannelWorkerWaiter;

use super::{
    config::{BuilderConfig, ControllerConfig},
    create::{
        ControllerRuntime, ControllerRuntimePort, CreateProfileRuntimeAttempt,
        CreateProfileRuntimeBackend,
    },
};

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not create command channels"
    )
)]
const COMMAND_CAPACITY: usize = 16;
#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct worker budgets"
    )
)]
const COMMAND_BATCH: usize = 16;
#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct worker budgets"
    )
)]
const POLL_BATCHES: usize = 4;
#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not own partially opened transports"
    )
)]
const UNOWNED_DRAIN_TIMEOUT: Duration = Duration::from_secs(1);

#[cfg(test)]
pub(super) const fn default_runtime_tuning() -> (usize, usize, usize) {
    (COMMAND_CAPACITY, COMMAND_BATCH, POLL_BATCHES)
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime factories"
    )
)]
pub(super) trait PairDriver {
    fn after_pair_enqueued(&mut self) -> crate::Result<()>;
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime components"
    )
)]
pub(super) struct RuntimeComponents<C, W, D> {
    transport: Box<dyn TransportPort>,
    clock: C,
    waiter: W,
    pair_driver: D,
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime components"
    )
)]
impl<C, W, D> RuntimeComponents<C, W, D> {
    pub(super) fn new(
        transport: Box<dyn TransportPort>,
        clock: C,
        waiter: W,
        pair_driver: D,
    ) -> Self {
        Self {
            transport,
            clock,
            waiter,
            pair_driver,
        }
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime factory projections"
    )
)]
pub(super) struct RuntimeFactoryConfig {
    #[cfg(feature = "bumble")]
    selector: crate::AdapterSelector,
    #[cfg(feature = "bumble")]
    transport: TransportConfig,
    #[cfg(feature = "bumble")]
    profile_key_store: Option<ProfileKeyStoreFactory>,
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime factory projections"
    )
)]
impl RuntimeFactoryConfig {
    pub(super) fn from_controller<M, R>(config: &ControllerConfig<M, R>) -> Self
    where
        M: ControllerModel,
        R: ReportingMode,
    {
        #[cfg(feature = "bumble")]
        {
            Self {
                selector: config.adapter.clone(),
                transport: config.transport_config(),
                profile_key_store: config
                    .profile
                    .persistent_path()
                    .map(|path| ProfileKeyStoreFactory::for_model::<M>(path.to_owned())),
            }
        }
        #[cfg(not(feature = "bumble"))]
        {
            let _ = config;
            Self {}
        }
    }

    #[cfg(all(test, feature = "bumble"))]
    pub(super) const fn has_profile_key_store(&self) -> bool {
        self.profile_key_store.is_some()
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct concrete runtime backends"
    )
)]
pub(super) struct ConcreteRuntimeBackend<F> {
    factory: Option<F>,
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct concrete runtime backends"
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
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not own concrete runtime attempts"
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
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct concrete runtime attempts"
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

    #[cfg(test)]
    pub(super) const fn owns_worker(&self) -> bool {
        self.owner.is_some()
    }

    fn into_open_runtime(mut self) -> ControllerRuntime<M, R>
    where
        R: WorkerReporting<M>,
    {
        let owner = self
            .owner
            .take()
            .expect("open runtime attempt retains its worker owner");
        ControllerRuntime::from_port(owner)
    }
}

impl<M, R, F, C, W, D> CreateProfileRuntimeBackend<M, R> for ConcreteRuntimeBackend<F>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    F: FnOnce(
        RuntimeFactoryConfig,
        ActivityNotifier,
        Receiver<()>,
    ) -> crate::Result<RuntimeComponents<C, W, D>>,
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
    F: FnOnce(
        RuntimeFactoryConfig,
        ActivityNotifier,
        Receiver<()>,
    ) -> crate::Result<RuntimeComponents<C, W, D>>,
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
        } = factory(
            RuntimeFactoryConfig::from_controller(config),
            activity.clone(),
            activity_receiver,
        )?;

        self.unowned_transport = Some(transport);
        let capabilities = match self
            .unowned_transport
            .as_deref_mut()
            .expect("runtime attempt retains its transport during open")
            .open(activity.clone())
        {
            Ok(capabilities) => capabilities,
            Err(source) => {
                return Err(Error::with_source(
                    ErrorKind::TransportOpen,
                    "controller transport could not be opened and initialized",
                    source,
                ));
            }
        };
        if !capabilities.classic_capable() {
            return Err(Error::with_source(
                ErrorKind::TransportOpen,
                "controller transport does not support the required Classic ACL operations",
                TransportError::new(TransportErrorKind::UnsupportedController),
            ));
        }
        let transport = self
            .unowned_transport
            .take()
            .expect("opened attempt retains its transport until worker transfer");
        let protocol = SwitchHidProtocol::new(Some(config.colors), capabilities.local_address());
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
                    self.status
                        .fail("worker spawn failed", WorkerFailureCategory::Internal);
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

    fn into_ready(self) -> ControllerRuntime<M, R> {
        self.into_open_runtime()
    }
}

#[cfg(any(test, feature = "bumble"))]
pub(super) fn open_controller_runtime<M, R, F, C, W, D>(
    config: &ControllerConfig<M, R>,
    status: StatusPublisher<M>,
    factory: F,
) -> crate::Result<ControllerRuntime<M, R>>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    F: FnOnce(
        RuntimeFactoryConfig,
        ActivityNotifier,
        Receiver<()>,
    ) -> crate::Result<RuntimeComponents<C, W, D>>,
    C: MonotonicClock + 'static,
    W: WorkerWaiter + 'static,
    D: PairDriver,
{
    let mut attempt = ConcreteRuntimeAttempt::new(factory, status);
    if let Err(primary) = attempt.open(config) {
        return Err(with_cleanup_error(
            primary,
            attempt.cleanup_without_neutral(),
        ));
    }
    Ok(attempt.into_open_runtime())
}

#[cfg(feature = "bumble")]
pub(super) fn open_bumble_runtime<M, R>(
    config: &ControllerConfig<M, R>,
    status: StatusPublisher<M>,
) -> crate::Result<ControllerRuntime<M, R>>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    open_controller_runtime(config, status, bumble_runtime_components)
}

#[cfg(feature = "bumble")]
pub(super) fn bumble_runtime_backend<M, R>() -> impl CreateProfileRuntimeBackend<M, R>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    ConcreteRuntimeBackend::new(bumble_runtime_components)
}

#[cfg(feature = "bumble")]
fn bumble_runtime_components(
    config: RuntimeFactoryConfig,
    _activity: ActivityNotifier,
    activity_receiver: Receiver<()>,
) -> crate::Result<RuntimeComponents<SystemClock, ChannelWorkerWaiter, ProductionPairDriver>> {
    Ok(RuntimeComponents::new(
        Box::new(
            crate::runtime::transport::BumbleTransportPort::with_profile_key_store(
                config.selector,
                config.transport,
                config.profile_key_store,
            ),
        ),
        SystemClock::new(),
        ChannelWorkerWaiter::new(activity_receiver),
        ProductionPairDriver,
    ))
}

#[cfg(feature = "bumble")]
struct SystemClock {
    origin: Instant,
}

#[cfg(feature = "bumble")]
impl SystemClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

#[cfg(feature = "bumble")]
impl MonotonicClock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[cfg(feature = "bumble")]
pub(super) struct ProductionPairDriver;

#[cfg(feature = "bumble")]
impl PairDriver for ProductionPairDriver {
    fn after_pair_enqueued(&mut self) -> crate::Result<()> {
        Ok(())
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

impl<M, R> ControllerRuntimePort<M, R> for WorkerOwner<RuntimeCommand<M, R>>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    fn pair(&mut self, timeout: Duration) -> crate::Result<()> {
        let response = self
            .try_enqueue(RuntimeCommand::Pair { timeout })
            .map_err(map_enqueue_error)?;
        receive_response(response)
    }

    fn reconnect(&mut self, timeout: Duration) -> crate::Result<()> {
        let response = self
            .try_enqueue(RuntimeCommand::Reconnect { timeout })
            .map_err(map_enqueue_error)?;
        receive_response(response)
    }

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

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds cannot deliver runtime commands"
    )
)]
fn receive_response(response: CommandResponse) -> crate::Result<()> {
    response
        .recv()
        .map_err(map_response_error)?
        .map_err(map_command_error)
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "M4 activates terminal recovery while pairing")
)]
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

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not spawn controller workers"
    )
)]
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

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not own partially opened transports"
    )
)]
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

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not own partially opened transports"
    )
)]
fn cleanup_unowned_transport_for_drop(transport: &mut dyn TransportPort) {
    let _ = transport.disconnect();
    let _ = transport.close();
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not aggregate transport cleanup"
    )
)]
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
