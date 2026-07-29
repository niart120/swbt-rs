//! Typed controller and builder identities.

mod config;
pub(crate) mod input;

#[cfg(test)]
mod config_tests;

use std::cell::Cell;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::Duration;

use crate::AdapterSelector;
use crate::diagnostics::GamepadStatus;
use crate::input::InputState;
use crate::model::{self, ControllerModel};
use crate::profile::ControllerColors;
use crate::reporting::{self, ReportingMode};
use crate::runtime::status::StatusReader;

use config::BuilderConfig;

/// A controller whose model and reporting mode are fixed by its type.
///
/// Read-only status and input snapshots do not wait for transport I/O.
/// A builder can be created without opening an adapter or starting a worker.
/// Lifecycle-changing operations are not exposed in the current package
/// surface. Controllers are transferable between threads but intentionally
/// cannot be shared between threads.
pub struct Controller<M: ControllerModel, R: ReportingMode> {
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

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 controller orchestration constructs the public controller"
        )
    )]
    pub(crate) fn from_status(status_reader: StatusReader<M>) -> Self {
        Self {
            status_reader,
            _types: PhantomData,
            _not_sync: PhantomData,
        }
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

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T29 build validates settings before reading a profile"
        )
    )]
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
