use std::{sync::mpsc::Receiver, time::Duration};

#[cfg(feature = "bumble")]
use std::time::Instant;

use crate::{
    diagnostics::event::WorkerFailureCategory,
    error::{Error, ErrorKind},
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    reporting::ReportingMode,
    runtime::{
        cleanup::{CleanupFailure, CleanupPhase, CloseMode},
        command::{CommandEnqueueError, CommandResponseError},
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

use super::create::with_cleanup_error;
#[cfg(feature = "bumble")]
use crate::runtime::transport::{ProfileKeyStoreFactory, TransportConfig};
#[cfg(feature = "bumble")]
use crate::runtime::worker::ChannelWorkerWaiter;

use super::{config::ControllerConfig, create::ControllerRuntime};

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
pub(super) const fn default_runtime_tuning() -> usize {
    POLL_BATCHES
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime components"
    )
)]
pub(super) struct RuntimeComponents<C, W> {
    transport: Box<dyn TransportPort>,
    clock: C,
    waiter: W,
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime components"
    )
)]
impl<C, W> RuntimeComponents<C, W> {
    pub(super) fn new(transport: Box<dyn TransportPort>, clock: C, waiter: W) -> Self {
        Self {
            transport,
            clock,
            waiter,
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
    #[cfg(feature = "bumble")]
    identity: crate::ProfileIdentity,
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
                identity: config.profile.identity(),
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

    #[cfg(all(test, feature = "bumble"))]
    pub(super) const fn identity(&self) -> crate::ProfileIdentity {
        self.identity
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not own partially opened transports"
    )
)]
struct UnownedTransportGuard {
    transport: Option<Box<dyn TransportPort>>,
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not own partially opened transports"
    )
)]
impl UnownedTransportGuard {
    fn new(transport: Box<dyn TransportPort>) -> Self {
        Self {
            transport: Some(transport),
        }
    }

    fn transport_mut(&mut self) -> &mut dyn TransportPort {
        self.transport
            .as_deref_mut()
            .expect("unowned transport guard is armed")
    }

    fn into_transport(mut self) -> Box<dyn TransportPort> {
        self.transport
            .take()
            .expect("unowned transport guard is armed")
    }

    fn cleanup(mut self) -> crate::Result<()> {
        let Some(mut transport) = self.transport.take() else {
            return Ok(());
        };
        cleanup_unowned_transport(transport.as_mut())
    }
}

impl Drop for UnownedTransportGuard {
    fn drop(&mut self) {
        if let Some(mut transport) = self.transport.take() {
            cleanup_unowned_transport_for_drop(transport.as_mut());
        }
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct runtime owner guards"
    )
)]
pub(super) struct RuntimeOwnerGuard<M, R>
where
    M: ControllerModel,
    R: ReportingMode,
{
    owner: Option<WorkerOwner<RuntimeCommand<M, R>>>,
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not use runtime owner guards"
    )
)]
impl<M, R> RuntimeOwnerGuard<M, R>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    fn new(owner: WorkerOwner<RuntimeCommand<M, R>>) -> Self {
        Self { owner: Some(owner) }
    }

    #[cfg(test)]
    pub(super) fn worker_is_finished(&self) -> bool {
        self.owner.as_ref().is_some_and(WorkerOwner::is_finished)
    }

    pub(super) fn pair_to_ready(&mut self, pair_timeout: Duration) -> crate::Result<()> {
        let enqueue = self
            .owner
            .as_ref()
            .ok_or_else(|| Error::new(ErrorKind::WorkerFailed, "runtime owner is unavailable"))?
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

    pub(super) fn cleanup_without_neutral(mut self) -> crate::Result<()> {
        if let Some(owner) = self.owner.take() {
            return map_worker_outcome(owner.finish_explicit(CloseMode::WithoutNeutral));
        }
        Ok(())
    }

    pub(super) fn into_runtime(mut self) -> ControllerRuntime<M, R> {
        let owner = self
            .owner
            .take()
            .expect("ready runtime retains its worker owner");
        ControllerRuntime::new(owner)
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not open controller runtime owners"
    )
)]
pub(super) fn open_runtime_owner<M, R, F, C, W>(
    config: &ControllerConfig<M, R>,
    status: StatusPublisher<M>,
    factory: F,
) -> crate::Result<RuntimeOwnerGuard<M, R>>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    F: FnOnce(
        RuntimeFactoryConfig,
        ActivityNotifier,
        Receiver<()>,
    ) -> crate::Result<RuntimeComponents<C, W>>,
    C: MonotonicClock + 'static,
    W: WorkerWaiter + 'static,
{
    let (activity, activity_receiver) = activity_channel();
    let RuntimeComponents {
        transport,
        clock,
        waiter,
    } = factory(
        RuntimeFactoryConfig::from_controller(config),
        activity.clone(),
        activity_receiver,
    )?;
    let mut transport = UnownedTransportGuard::new(transport);
    let capabilities = match transport.transport_mut().open(activity.clone()) {
        Ok(capabilities) => capabilities,
        Err(source) => {
            return Err(with_cleanup_error(
                map_transport_open_error(source),
                transport.cleanup(),
            ));
        }
    };
    if !capabilities.classic_capable() {
        let primary = Error::with_source(
            ErrorKind::TransportOpen,
            "controller transport does not support the required Classic ACL operations",
            TransportError::new(TransportErrorKind::UnsupportedController),
        );
        return Err(with_cleanup_error(primary, transport.cleanup()));
    }
    let protocol = SwitchHidProtocol::new(Some(config.colors), capabilities.local_address());
    let worker = R::build_worker(
        protocol,
        transport.into_transport(),
        &config.mode,
        WorkerBudget::new(POLL_BATCHES),
        Box::new(|_| {}),
        status.clone(),
    );
    let (commands, command_receiver) =
        crate::runtime::command::command_channel::<RuntimeCommand<M, R>>(activity.clone());
    let (shutdown, shutdown_receiver) = priority_shutdown_channel(activity);
    let thread = spawn_worker_thread(worker, clock, shutdown_receiver, command_receiver, waiter)
        .map_err(|error| {
            status.fail("worker spawn failed", WorkerFailureCategory::Internal);
            map_worker_spawn_error(error)
        })?;

    Ok(RuntimeOwnerGuard::new(WorkerOwner::new(
        commands, shutdown, thread,
    )))
}

#[cfg(any(test, feature = "bumble"))]
pub(super) fn open_controller_runtime<M, R, F, C, W>(
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
    ) -> crate::Result<RuntimeComponents<C, W>>,
    C: MonotonicClock + 'static,
    W: WorkerWaiter + 'static,
{
    Ok(open_runtime_owner(config, status, factory)?.into_runtime())
}

#[cfg(any(test, feature = "bumble"))]
pub(super) fn create_controller_runtime<M, R, F, C, W>(
    config: &ControllerConfig<M, R>,
    status: StatusPublisher<M>,
    pair_timeout: Duration,
    factory: F,
) -> crate::Result<ControllerRuntime<M, R>>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
    F: FnOnce(
        RuntimeFactoryConfig,
        ActivityNotifier,
        Receiver<()>,
    ) -> crate::Result<RuntimeComponents<C, W>>,
    C: MonotonicClock + 'static,
    W: WorkerWaiter + 'static,
{
    let mut owner = open_runtime_owner(config, status, factory)?;
    if let Err(primary) = owner.pair_to_ready(pair_timeout) {
        return Err(with_cleanup_error(primary, owner.cleanup_without_neutral()));
    }
    Ok(owner.into_runtime())
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
pub(super) fn create_bumble_runtime<M, R>(
    config: &ControllerConfig<M, R>,
    status: StatusPublisher<M>,
    pair_timeout: Duration,
) -> crate::Result<ControllerRuntime<M, R>>
where
    M: ControllerModel,
    R: WorkerReporting<M>,
{
    create_controller_runtime(config, status, pair_timeout, bumble_runtime_components)
}

#[cfg(feature = "bumble")]
fn bumble_runtime_components(
    config: RuntimeFactoryConfig,
    _activity: ActivityNotifier,
    activity_receiver: Receiver<()>,
) -> crate::Result<RuntimeComponents<SystemClock, ChannelWorkerWaiter>> {
    Ok(RuntimeComponents::new(
        Box::new(
            crate::runtime::transport::BumbleTransportPort::with_profile_key_store(
                config.selector,
                config.transport,
                config.identity,
                config.profile_key_store,
            ),
        ),
        SystemClock::new(),
        ChannelWorkerWaiter::new(activity_receiver),
    ))
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not map transport open errors"
    )
)]
fn map_transport_open_error(source: TransportError) -> Error {
    if source.kind() == TransportErrorKind::AdapterIdentityRecoveryRequired {
        return Error::with_source(
            ErrorKind::AdapterIdentityRecoveryRequired,
            "adapter identity is uncertain; physically power cycle the USB adapter and verify its original identity before retrying",
            source,
        );
    }
    Error::with_source(
        ErrorKind::TransportOpen,
        "controller transport could not be opened and initialized",
        source,
    )
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

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not finish terminal runtime owners"
    )
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
