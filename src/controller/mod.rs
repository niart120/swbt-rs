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
use crate::model::{self, ControllerModel};
use crate::profile::{ControllerColors, FileProfileCreateTarget};
use crate::reporting::{self, ReportingMode};
use crate::runtime::{
    cleanup::CloseMode,
    status::{StatusPublisher, StatusReader, status_projection},
    worker::{CommonCommand, DirectCommand, PeriodicCommand},
};
use crate::{AdapterSelector, CreateProfileOptions};

use build::{FileProfileReader, ProfileReadPort, read_typed_profile};
#[cfg(test)]
use config::ProfileConfig;
use config::{BuilderConfig, ControllerConfig};
use create::{CreateProfilePlan, CreateProfileRuntimeBackend, ReadyRuntime};

/// A controller whose model and reporting mode are fixed by its type.
///
/// Read-only status and input snapshots do not wait for transport I/O.
/// A builder can be created without opening an adapter or starting a worker.
/// Input and close operations wait for the owned worker when a runtime is
/// active. Controllers are transferable between threads but intentionally
/// cannot be shared between threads.
pub struct Controller<M: ControllerModel, R: ReportingMode> {
    _runtime: Option<ReadyRuntime<M, R>>,
    #[allow(
        dead_code,
        reason = "T31 consumes the validated configuration when opening the runtime"
    )]
    config: ControllerConfig<M, R>,
    #[allow(
        dead_code,
        reason = "T31 shares the configured projection writer with the worker"
    )]
    status_publisher: StatusPublisher<M>,
    status_reader: StatusReader<M>,
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
        let (status_publisher, status_reader) = status_projection();

        Self::from_parts(config, status_publisher, status_reader, None)
    }

    fn from_ready_runtime(
        config: ControllerConfig<M, R>,
        status_publisher: StatusPublisher<M>,
        status_reader: StatusReader<M>,
        runtime: ReadyRuntime<M, R>,
    ) -> Self {
        Self::from_parts(config, status_publisher, status_reader, Some(runtime))
    }

    fn from_parts(
        config: ControllerConfig<M, R>,
        status_publisher: StatusPublisher<M>,
        status_reader: StatusReader<M>,
        runtime: Option<ReadyRuntime<M, R>>,
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
        self.status_reader.status::<R>()
    }

    /// Returns the latest committed model-valid input without waiting for transport I/O.
    #[must_use]
    pub fn snapshot(&self) -> InputState<M> {
        self.status_reader.snapshot()
    }

    /// Opens the configured controller transport.
    ///
    /// The current package has no concrete Bluetooth transport backend, so
    /// this operation leaves the configured controller and its diagnostics
    /// unchanged without starting a worker or opening an adapter.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnsupportedCapability`] while the concrete
    /// Bluetooth transport is unavailable.
    pub fn open(&mut self) -> crate::Result<()> {
        Err(crate::runtime::error_map::unsupported_capability(
            "Bluetooth transport",
        ))
    }

    /// Pairs the configured controller within `timeout`.
    ///
    /// The current package has no concrete Bluetooth transport backend, so
    /// this operation leaves the configured controller and its diagnostics
    /// unchanged without starting a worker or opening an adapter.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::UnsupportedCapability`] while the concrete
    /// Bluetooth transport is unavailable.
    pub fn pair(&mut self, _timeout: Duration) -> crate::Result<()> {
        Err(crate::runtime::error_map::unsupported_capability(
            "Bluetooth transport",
        ))
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
    /// button set returns [`ErrorKind::InvalidInput`]. Queue, shutdown,
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
    /// button set returns [`ErrorKind::InvalidInput`]. Queue, shutdown,
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
    /// [`ErrorKind::InvalidInput`]. Queue, shutdown, transport, and worker
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
    /// runtime, or in Direct mode when the transport is not Ready. Queue,
    /// shutdown, transport, and worker failures are returned as structured
    /// [`crate::Error`] values.
    pub fn neutral(&mut self) -> crate::Result<()> {
        self.request_common(CommonCommand::Neutral)
    }

    /// Closes the runtime, attempting a trailing neutral report first.
    ///
    /// Cleanup continues through drain, disconnect, transport close, worker
    /// completion, and join if sending the neutral report fails. Calling this
    /// method when no runtime remains records
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
    /// Bounded drain, disconnect, transport close, worker completion, and join
    /// still run. Calling this method when no runtime remains records
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
        self.ready_runtime()?.request(command)
    }

    fn ready_runtime(&mut self) -> crate::Result<&mut ReadyRuntime<M, R>> {
        self._runtime.as_mut().ok_or_else(|| {
            Error::new(
                ErrorKind::TransportClosed,
                "controller runtime is not ready",
            )
        })
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

impl<M: ControllerModel> Controller<M, reporting::Periodic> {
    /// Replaces the committed input used by periodic reporting.
    ///
    /// The call blocks until the worker has committed the model-valid state.
    /// Transport delivery is performed later by the periodic scheduler.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::TransportClosed`] when the controller has no active
    /// runtime. Queue, shutdown, and worker failures are returned as structured
    /// [`crate::Error`] values.
    pub fn apply(&mut self, state: InputState<M>) -> crate::Result<()> {
        self.ready_runtime()?.request(PeriodicCommand::Apply(state))
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
    /// runtime or the transport is not Ready. Queue, shutdown, transport, and
    /// worker failures are returned as structured [`crate::Error`] values.
    pub fn send(&mut self, state: InputState<M>) -> crate::Result<()> {
        self.ready_runtime()?.request(DirectCommand::Send(state))
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
        not(test),
        allow(
            dead_code,
            reason = "crate-private tests exercise successful profile creation before M5 supplies production persistence"
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
    /// never replaced. The current package has no concrete Bluetooth
    /// transport backend, so an otherwise valid request stops before creating
    /// the profile file.
    ///
    /// # Errors
    ///
    /// Returns [`crate::ErrorKind::InvalidInput`] for invalid builder settings,
    /// [`crate::ErrorKind::ProfilePathRequired`] when no target path was
    /// selected, [`crate::ErrorKind::UnsupportedCapability`] for an unsupported
    /// identity or unavailable Bluetooth transport,
    /// [`crate::ErrorKind::ProfileAlreadyExists`] when the target already
    /// exists, or [`crate::ErrorKind::Internal`] when the target cannot be
    /// inspected.
    pub fn create_profile(self, options: CreateProfileOptions) -> crate::Result<Controller<M, R>> {
        let mut target = FileProfileCreateTarget;
        let plan = self.validate_create_profile_target(options, &mut target)?;
        create::reject_unavailable_backend(plan)
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
        let mut reader = FileProfileReader;
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

/// Periodic Pro Controller.
pub type ProController = Controller<model::Pro, reporting::Periodic>;

/// Direct-reporting Pro Controller.
pub type DirectProController = Controller<model::Pro, reporting::Direct>;

/// Periodic left Joy-Con.
pub type JoyConL = Controller<model::JoyConL, reporting::Periodic>;

/// Direct-reporting left Joy-Con.
pub type DirectJoyConL = Controller<model::JoyConL, reporting::Direct>;

/// Periodic right Joy-Con.
pub type JoyConR = Controller<model::JoyConR, reporting::Periodic>;

/// Direct-reporting right Joy-Con.
pub type DirectJoyConR = Controller<model::JoyConR, reporting::Direct>;
