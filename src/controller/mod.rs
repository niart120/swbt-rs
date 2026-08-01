//! Typed controller and builder identities.

mod build;
mod config;
mod create;
pub(crate) mod input;
mod runtime;

#[cfg(test)]
mod build_tests;
#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod create_profile_tests;
#[cfg(test)]
mod runtime_measurement;
#[cfg(test)]
mod runtime_tests;

use std::cell::Cell;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::Duration;

use crate::diagnostics::GamepadStatus;
use crate::error::{Error, ErrorKind};
use crate::input::{Button, InputState};
use crate::model::{self, ButtonKind, ControllerModel};
use crate::profile::{ControllerColors, FileProfileStore};
use crate::reporting::{self, ReportingMode};
use crate::runtime::{
    cleanup::CloseMode,
    status::{StatusPublisher, StatusReader, status_projection},
    worker::{CommonCommand, DirectCommand, PeriodicCommand, WorkerReporting},
};
use crate::{
    AdapterSelector, ConnectOptions, ConnectionPath, ConnectionResult, ConnectionStatus,
    CreateProfileOptions,
};

use build::{ProfileReadPort, read_typed_profile};
#[cfg(test)]
use config::ProfileConfig;
use config::{BuilderConfig, ControllerConfig};
use create::{ControllerRuntime, CreateProfilePlan, CreateProfileRuntimeBackend};

/// A controller whose model and reporting mode are fixed by its type.
///
/// Read-only status and input snapshots do not wait for transport I/O.
/// A builder can be created without opening an adapter or starting a worker.
/// Input and close operations wait for the owned worker when a runtime is
/// active. Controllers are transferable between threads but intentionally
/// cannot be shared between threads.
///
/// Explicit close waits for cleanup and worker completion, joins the worker,
/// and returns cleanup or join failures. Dropping a controller instead requests
/// bounded best-effort shutdown without a trailing neutral report or pending
/// send drain. Drop cannot return shutdown failures, and its internal wait
/// duration is not a public timing guarantee.
pub struct Controller<M: ControllerModel, R: ReportingMode> {
    _runtime: Option<ControllerRuntime<M, R>>,
    config: ControllerConfig<M, R>,
    status_publisher: StatusPublisher<M>,
    status_reader: StatusReader<M, R>,
    _types: PhantomData<fn() -> (M, R)>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
    /// Creates a side-effect-free builder for the selected adapter.
    ///
    /// The adapter selector is stored verbatim. Its backend-specific syntax is
    /// interpreted only when a later lifecycle operation opens the adapter.
    #[must_use]
    pub fn builder(adapter: impl Into<AdapterSelector>) -> ControllerBuilder<M, R> {
        ControllerBuilder {
            adapter: adapter.into(),
            profile_path: None,
            colors: M::SPEC.protocol.default_colors,
            mode: <R as reporting::sealed::Sealed>::default_options(),
            _model: PhantomData,
        }
    }

    fn from_config(config: ControllerConfig<M, R>) -> Self {
        let (status_publisher, status_reader) = status_projection::<M, R>();

        Self::from_parts(config, status_publisher, status_reader, None)
    }

    fn from_ready_runtime(
        config: ControllerConfig<M, R>,
        status_publisher: StatusPublisher<M>,
        status_reader: StatusReader<M, R>,
        runtime: ControllerRuntime<M, R>,
    ) -> Self {
        Self::from_parts(config, status_publisher, status_reader, Some(runtime))
    }

    fn from_parts(
        config: ControllerConfig<M, R>,
        status_publisher: StatusPublisher<M>,
        status_reader: StatusReader<M, R>,
        runtime: Option<ControllerRuntime<M, R>>,
    ) -> Self {
        Self {
            _runtime: runtime,
            config,
            status_publisher,
            status_reader,
            _types: PhantomData,
            _not_sync: PhantomData,
        }
    }

    #[cfg(test)]
    fn config(&self) -> &ControllerConfig<M, R> {
        &self.config
    }

    #[cfg(test)]
    pub(crate) fn status_publisher(&self) -> StatusPublisher<M> {
        self.status_publisher.clone()
    }

    /// Returns the latest runtime diagnostics without waiting for transport I/O.
    #[must_use]
    pub fn status(&self) -> GamepadStatus {
        self.status_reader.status()
    }

    /// Returns the latest committed model-valid input without waiting for transport I/O.
    ///
    /// Periodic input is committed when its command is processed; the latest
    /// state before the next report deadline is used. Direct input is committed
    /// only after transport acceptance. A new connection session resets the
    /// snapshot to neutral, and stale events from an earlier session cannot
    /// mutate it.
    #[must_use]
    pub fn snapshot(&self) -> InputState<M> {
        self.status_reader.snapshot()
    }

    /// Converts a model-independent button identity into a button for this
    /// controller model.
    ///
    /// When an active connection session exists, rejecting an unsupported
    /// button also emits the stable `unsupported_button` diagnostics event for
    /// that session. Calling this method does not open a transport or change
    /// controller input.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnsupportedInput`] when `kind` is not available on
    /// model `M`.
    pub fn button(&self, kind: ButtonKind) -> crate::Result<Button<M>> {
        match Button::try_from(kind) {
            Ok(button) => Ok(button),
            Err(error) => {
                self.status_publisher.record_unsupported_button(kind);
                Err(error)
            }
        }
    }

    /// Opens the configured controller transport.
    ///
    /// With the `bumble` feature, this operation opens and initializes the
    /// selected Bluetooth HCI adapter and starts the owned worker. A persistent
    /// explicit-local-address profile first applies and reads back its volatile
    /// CSR adapter identity, then verifies the same address after normal HCI
    /// initialization. Repeated calls while that runtime is open succeed
    /// without opening another adapter or starting another worker.
    ///
    /// # Errors
    ///
    /// Without the `bumble` feature, returns
    /// [`ErrorKind::UnsupportedCapability`] before transport side effects.
    /// Returns [`ErrorKind::AdapterIdentityRecoveryRequired`] when an explicit
    /// identity write started but the final adapter state could not be
    /// verified; physically power-cycle the USB adapter and verify its
    /// original identity before retrying.
    /// With the feature enabled, returns a typed transport or worker error when
    /// initialization or worker startup fails.
    pub fn open(&mut self) -> crate::Result<()> {
        <R as reporting::sealed::Sealed>::open_controller(self)
    }

    /// Pairs the configured controller within `timeout`.
    ///
    /// The call starts the open transport's pairing window and blocks until
    /// the same connection session completes the NX readiness handshake.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when no runtime is open.
    /// Timeout, disconnect, transport, protocol, and worker failures are
    /// returned as structured [`crate::Error`] values.
    pub fn pair(&mut self, timeout: Duration) -> crate::Result<()> {
        self.runtime_mut()?.pair(timeout)
    }

    /// Reconnects with the configured profile's stored Classic bond.
    ///
    /// The call blocks until the same connection session completes the NX
    /// readiness handshake. It never deletes a failed bond or falls back to
    /// fresh pairing.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when no runtime is open,
    /// [`ErrorKind::NoBond`] when the profile has no usable Classic bond,
    /// [`ErrorKind::ConnectionTimeout`] when readiness misses `timeout`, and
    /// [`ErrorKind::ConnectionFailed`] when the stored-key connection ends
    /// before readiness. Profile, protocol, shutdown, and worker failures
    /// remain errors.
    pub fn reconnect(&mut self, timeout: Duration) -> crate::Result<()> {
        self.runtime_mut()?.reconnect(timeout)
    }

    /// Connects by trying a stored bond before any allowed pairing attempt.
    ///
    /// Pairing is attempted only when reconnect returns
    /// [`ErrorKind::NoBond`] and [`ConnectOptions::allow_pairing`] is `true`.
    /// Timeout, stale-bond, protocol, and worker failures do not fall back to
    /// pairing.
    ///
    /// # Errors
    ///
    /// Returns the reconnect or pairing error when the selected path does not
    /// reach readiness. A missing bond remains [`ErrorKind::NoBond`] when
    /// pairing is disabled.
    pub fn connect(&mut self, options: ConnectOptions) -> crate::Result<ConnectionPath> {
        match self.reconnect(options.timeout) {
            Ok(()) => Ok(ConnectionPath::Reconnected),
            Err(error) if error.kind() == ErrorKind::NoBond && options.allow_pairing => {
                self.pair(options.timeout)?;
                Ok(ConnectionPath::Paired)
            }
            Err(error) => Err(error),
        }
    }

    /// Attempts stored-key reconnect and returns recoverable connection
    /// outcomes as data.
    ///
    /// No-bond, timeout, and pre-readiness disconnect become a
    /// [`ConnectionResult`]. Profile corruption, protocol inconsistency,
    /// shutdown, and worker failures remain errors.
    pub fn try_reconnect(&mut self, timeout: Duration) -> crate::Result<ConnectionResult> {
        recoverable_connection_result(
            self.reconnect(timeout)
                .map(|()| ConnectionPath::Reconnected),
        )
    }

    /// Runs [`Self::connect`] and returns recoverable connection outcomes as
    /// data.
    ///
    /// Profile corruption, protocol inconsistency, shutdown, and worker
    /// failures remain errors.
    pub fn try_connect(&mut self, options: ConnectOptions) -> crate::Result<ConnectionResult> {
        recoverable_connection_result(self.connect(options))
    }

    /// Presses one or more model-valid buttons.
    ///
    /// The call blocks until the worker has completed the reporting-specific
    /// state transition.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when the controller has no active
    /// runtime, or in Direct mode when the transport is not Ready. An empty
    /// button set returns [`ErrorKind::InvalidInput`]. Shutdown,
    /// transport, and worker failures are returned as structured
    /// [`crate::Error`] values.
    pub fn press(&mut self, buttons: impl IntoIterator<Item = Button<M>>) -> crate::Result<()> {
        self.request_common(CommonCommand::Press(buttons.into_iter().collect()))
    }

    /// Releases one or more model-valid buttons.
    ///
    /// The call blocks until the worker has completed the reporting-specific
    /// state transition.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when the controller has no active
    /// runtime, or in Direct mode when the transport is not Ready. An empty
    /// button set returns [`ErrorKind::InvalidInput`]. Shutdown,
    /// transport, and worker failures are returned as structured
    /// [`crate::Error`] values.
    pub fn release(&mut self, buttons: impl IntoIterator<Item = Button<M>>) -> crate::Result<()> {
        self.request_common(CommonCommand::Release(buttons.into_iter().collect()))
    }

    /// Presses buttons, waits for `duration`, and then releases them.
    ///
    /// The worker remains responsive to protocol traffic while this blocking
    /// call waits for the release. Durations from zero through 24 hours are
    /// accepted.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when the controller has no active
    /// runtime or the transport is not Ready. Empty buttons or a duration
    /// greater than 24 hours return
    /// [`ErrorKind::InvalidInput`]. Shutdown, transport, and worker
    /// failures are returned as structured [`crate::Error`] values.
    pub fn tap(
        &mut self,
        buttons: impl IntoIterator<Item = Button<M>>,
        duration: Duration,
    ) -> crate::Result<()> {
        self.request_common(CommonCommand::Tap {
            buttons: buttons.into_iter().collect(),
            duration,
        })
    }

    /// Restores the model-valid neutral input state.
    ///
    /// The call blocks until the worker has completed the reporting-specific
    /// state transition.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when the controller has no active
    /// runtime, or in Direct mode when the transport is not Ready.
    /// Shutdown, transport, and worker failures are returned as structured
    /// [`crate::Error`] values.
    pub fn neutral(&mut self) -> crate::Result<()> {
        self.request_common(CommonCommand::Neutral)
    }

    /// Closes the runtime, attempting a trailing neutral report first.
    ///
    /// The bounded drain waits until host-side interrupt packets, including the
    /// trailing neutral report, have entered the controller's flow-control
    /// window. It does not wait for completion credit for every packet already
    /// in flight. Cleanup continues through disconnect, transport close, worker
    /// completion, and join if sending or draining the neutral report fails.
    /// Calling this method when no runtime remains records
    /// [`crate::LifecycleState::Closed`] and succeeds.
    ///
    /// # Errors
    ///
    /// Returns a cleanup, worker-termination, or worker-join error after all
    /// remaining cleanup phases have been attempted. An additional failure
    /// from the same close operation is available through
    /// [`crate::Error::related_error`].
    pub fn close(&mut self) -> crate::Result<()> {
        self.close_with_mode(CloseMode::WithNeutral)
    }

    /// Closes the runtime without sending a trailing neutral report.
    ///
    /// Bounded host-side interrupt drain, disconnect, transport close, worker
    /// completion, and join still run. The drain waits for packets to enter the
    /// controller's flow-control window, not for completion credit for every
    /// in-flight packet. Calling this method when no runtime remains records
    /// [`crate::LifecycleState::Closed`] and succeeds.
    ///
    /// # Errors
    ///
    /// Returns a cleanup, worker-termination, or worker-join error after all
    /// remaining cleanup phases have been attempted. An additional failure
    /// from the same close operation is available through
    /// [`crate::Error::related_error`].
    pub fn close_without_neutral(&mut self) -> crate::Result<()> {
        self.close_with_mode(CloseMode::WithoutNeutral)
    }

    fn request_common(&mut self, command: CommonCommand<M>) -> crate::Result<()> {
        let command = <R as reporting::sealed::Sealed>::common(command);
        self.runtime_mut()?.request(command)
    }

    fn runtime_mut(&mut self) -> crate::Result<&mut ControllerRuntime<M, R>> {
        self._runtime
            .as_mut()
            .ok_or_else(|| Error::new(ErrorKind::TransportClosed, "controller runtime is not open"))
    }

    pub(crate) fn open_supported_runtime(&mut self) -> crate::Result<()>
    where
        R: WorkerReporting<M>,
    {
        #[cfg(feature = "bumble")]
        {
            self.open_with(runtime::open_bumble_runtime::<M, R>)
        }
        #[cfg(not(feature = "bumble"))]
        {
            Err(crate::runtime::error_map::unsupported_capability(
                "Bluetooth transport",
            ))
        }
    }

    #[cfg(any(test, feature = "bumble"))]
    fn open_with(
        &mut self,
        open: impl FnOnce(
            &ControllerConfig<M, R>,
            StatusPublisher<M>,
        ) -> crate::Result<ControllerRuntime<M, R>>,
    ) -> crate::Result<()> {
        if self._runtime.is_some() {
            return Ok(());
        }
        let runtime = open(&self.config, self.status_publisher.clone())?;
        self._runtime = Some(runtime);
        Ok(())
    }

    fn close_with_mode(&mut self, mode: CloseMode) -> crate::Result<()> {
        let Some(runtime) = self._runtime.take() else {
            self.status_publisher
                .set_lifecycle(crate::LifecycleState::Closed);
            return Ok(());
        };
        runtime.close(mode)
    }
}

fn recoverable_connection_result(
    result: crate::Result<ConnectionPath>,
) -> crate::Result<ConnectionResult> {
    match result {
        Ok(path) => Ok(ConnectionResult {
            status: ConnectionStatus::Connected,
            path: Some(path),
            message: None,
        }),
        Err(error) => {
            let status = match error.kind() {
                ErrorKind::NoBond => ConnectionStatus::NoBond,
                ErrorKind::ConnectionTimeout => ConnectionStatus::TimedOut,
                ErrorKind::ConnectionFailed => ConnectionStatus::Failed,
                _ => return Err(error),
            };
            Ok(ConnectionResult {
                status,
                path: None,
                message: Some(error.to_string()),
            })
        }
    }
}

impl<M: ControllerModel> Controller<M, reporting::Periodic> {
    /// Replaces the committed input used by periodic reporting.
    ///
    /// An active runtime is required, but the transport need not be Ready. The
    /// call returns after the worker commits the model-valid state; transport
    /// delivery is performed later by the periodic scheduler. If multiple
    /// states are committed before the next report deadline, only the latest
    /// state is sent.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when the controller has no active
    /// runtime. Shutdown and worker failures are returned as structured
    /// [`crate::Error`] values.
    pub fn apply(&mut self, state: InputState<M>) -> crate::Result<()> {
        self.runtime_mut()?.request(PeriodicCommand::Apply(state))
    }

    /// Returns the validated periodic input-report interval.
    #[must_use]
    pub fn report_period(&self) -> Duration {
        self.config.report_period()
    }
}

impl<M: ControllerModel> Controller<M, reporting::Direct> {
    /// Sends and commits one model-valid input state.
    ///
    /// The snapshot is committed only after the transport accepts the report.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when the controller has no active
    /// runtime or the transport is not Ready. Shutdown, transport, and
    /// worker failures are returned as structured [`crate::Error`] values.
    pub fn send(&mut self, state: InputState<M>) -> crate::Result<()> {
        self.runtime_mut()?.request(DirectCommand::Send(state))
    }
}

/// Immutable construction settings for [`Controller<M, R>`].
///
/// Creating and modifying a builder does not open an adapter, read a profile,
/// or start a worker.
pub struct ControllerBuilder<M: ControllerModel, R: ReportingMode> {
    adapter: AdapterSelector,
    profile_path: Option<PathBuf>,
    colors: ControllerColors,
    mode: <R as reporting::sealed::Sealed>::BuilderOptions,
    _model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel, R: ReportingMode> ControllerBuilder<M, R> {
    /// Selects an existing pairing profile path.
    ///
    /// This setter only stores the path. Profile existence and contents are
    /// checked when the builder is consumed by a construction operation.
    #[must_use]
    pub fn profile_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.profile_path = Some(path.into());
        self
    }

    /// Overrides the model's default controller colors.
    #[must_use]
    pub fn controller_colors(mut self, colors: ControllerColors) -> Self {
        self.colors = colors;
        self
    }

    fn validate_create_profile_target(
        self,
        options: CreateProfileOptions,
        target: &mut impl crate::profile::ProfileCreateTargetPort,
    ) -> crate::Result<CreateProfilePlan<M, R>> {
        create::validate_target(self.validate()?, options, target)
    }

    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds cannot construct a successful profile runtime backend"
        )
    )]
    fn create_profile_with(
        self,
        options: CreateProfileOptions,
        store: &mut impl crate::profile::ProfileCreatePort,
        backend: &mut impl CreateProfileRuntimeBackend<M, R>,
    ) -> crate::Result<Controller<M, R>> {
        let plan = self.validate_create_profile_target(options, store)?;
        create::create_profile(plan, store, backend)
    }

    /// Attempts to create a new pairing profile and return a paired controller.
    ///
    /// Builder settings, the required profile path, the requested identity,
    /// and target existence are checked in that order. An existing target is
    /// never replaced. With the `bumble` feature, a valid empty profile is
    /// persisted before opening the selected adapter, then pairing waits for
    /// normal-input readiness.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ErrorKind::InvalidInput`] for invalid builder settings,
    /// [`crate::ErrorKind::ProfilePathRequired`] when no target path was
    /// selected, [`crate::ErrorKind::UnsupportedCapability`] for an unsupported
    /// identity or unavailable Bluetooth transport,
    /// [`crate::ErrorKind::ProfileAlreadyExists`] when the target already
    /// exists, [`crate::ErrorKind::TransportOpen`] when the adapter cannot be
    /// opened and initialized,
    /// [`crate::ErrorKind::AdapterIdentityRecoveryRequired`] when an explicit
    /// identity write started but could not be verified, or a structured
    /// profile I/O, pairing, connection, transport, protocol, cleanup, or
    /// worker error from the corresponding stage.
    pub fn create_profile(self, options: CreateProfileOptions) -> crate::Result<Controller<M, R>> {
        <R as reporting::sealed::Sealed>::create_profile(self, options)
    }

    pub(crate) fn create_profile_supported(
        self,
        options: CreateProfileOptions,
    ) -> crate::Result<Controller<M, R>>
    where
        R: WorkerReporting<M>,
    {
        #[cfg(feature = "bumble")]
        {
            let mut store = FileProfileStore;
            let mut backend = runtime::bumble_runtime_backend::<M, R>();
            self.create_profile_with(options, &mut store, &mut backend)
        }
        #[cfg(not(feature = "bumble"))]
        {
            let mut store = FileProfileStore;
            let plan = self.validate_create_profile_target(options, &mut store)?;
            create::reject_unavailable_backend(plan)
        }
    }

    /// Builds a configured controller without opening its adapter or starting a worker.
    ///
    /// With no profile path, the controller is ephemeral and this method
    /// performs no profile I/O. With a profile path, the file is read once and
    /// validated for the selected controller model.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ErrorKind::InvalidInput`] for invalid builder settings,
    /// [`crate::ErrorKind::ProfileNotFound`] when the selected profile does not
    /// exist, [`crate::ErrorKind::InvalidProfile`] for an invalid profile
    /// document, [`crate::ErrorKind::ProfileControllerMismatch`] for a profile
    /// belonging to another model, or [`crate::ErrorKind::Internal`] when the
    /// profile cannot be read for another filesystem reason.
    pub fn build(self) -> crate::Result<Controller<M, R>> {
        let mut reader = FileProfileStore;
        self.build_with_profile_reader(&mut reader)
    }

    fn build_with_profile_reader(
        self,
        reader: &mut impl ProfileReadPort,
    ) -> crate::Result<Controller<M, R>> {
        let config = self
            .validate()?
            .finalize_with_profile(|path| read_typed_profile(reader, path))?;

        Ok(Controller::from_config(config))
    }

    fn validate(self) -> crate::Result<BuilderConfig<M, R>> {
        let mode = <R as reporting::sealed::Sealed>::validate(self.mode)?;
        Ok(BuilderConfig::new(
            self.adapter,
            self.profile_path,
            self.colors,
            mode,
        ))
    }
}

impl<M: ControllerModel> ControllerBuilder<M, reporting::Periodic> {
    /// Sets the periodic input-report interval.
    ///
    /// The accepted range is 1 millisecond through 1 second, inclusive. The
    /// range is checked when a construction operation consumes the builder.
    #[must_use]
    pub fn report_period(mut self, period: Duration) -> Self {
        self.mode = self.mode.with_report_period(period);
        self
    }
}

/// Pro Controller using scheduled periodic reports.
///
/// This is [`Controller<model::Pro, reporting::Periodic>`]. Input operations
/// commit the local state for the periodic worker to emit on a later tick.
pub type ProController = Controller<model::Pro, reporting::Periodic>;

/// Pro Controller using caller-driven direct reports.
///
/// This is [`Controller<model::Pro, reporting::Direct>`]. Input operations
/// require a Ready transport and commit state after transport acceptance.
pub type DirectProController = Controller<model::Pro, reporting::Direct>;

/// Left Joy-Con using scheduled periodic reports.
///
/// This is [`Controller<model::JoyConL, reporting::Periodic>`]. Its typed input
/// exposes the left-side buttons and stick only. Input operations commit local
/// state for the periodic worker to emit on a later tick.
pub type JoyConL = Controller<model::JoyConL, reporting::Periodic>;

/// Left Joy-Con using caller-driven direct reports.
///
/// This is [`Controller<model::JoyConL, reporting::Direct>`]. Its typed input
/// exposes the left-side buttons and stick only. Input operations require a
/// Ready transport and commit state after transport acceptance.
pub type DirectJoyConL = Controller<model::JoyConL, reporting::Direct>;

/// Right Joy-Con using scheduled periodic reports.
///
/// This is [`Controller<model::JoyConR, reporting::Periodic>`]. Its typed input
/// exposes the right-side buttons and stick only. Input operations commit local
/// state for the periodic worker to emit on a later tick.
pub type JoyConR = Controller<model::JoyConR, reporting::Periodic>;

/// Right Joy-Con using caller-driven direct reports.
///
/// This is [`Controller<model::JoyConR, reporting::Direct>`]. Its typed input
/// exposes the right-side buttons and stick only. Input operations require a
/// Ready transport and commit state after transport acceptance.
pub type DirectJoyConR = Controller<model::JoyConR, reporting::Direct>;
