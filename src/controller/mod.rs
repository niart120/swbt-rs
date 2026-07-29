//! Typed controller and builder identities.

pub(crate) mod input;

use std::cell::Cell;
use std::marker::PhantomData;

use crate::diagnostics::GamepadStatus;
use crate::input::InputState;
use crate::model::{self, ControllerModel};
use crate::reporting::{self, ReportingMode};
use crate::runtime::status::StatusReader;

/// A controller whose model and reporting mode are fixed by its type.
///
/// Read-only status and input snapshots do not wait for transport I/O.
/// Construction and lifecycle-changing operations are not exposed in the
/// current package surface. Controllers are transferable between threads but
/// intentionally cannot be shared between threads.
pub struct Controller<M: ControllerModel, R: ReportingMode> {
    status_reader: StatusReader<M>,
    _types: PhantomData<fn() -> (M, R)>,
    _not_sync: PhantomData<Cell<()>>,
}

impl<M: ControllerModel, R: ReportingMode> Controller<M, R> {
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
/// Adapter, profile, and runtime construction methods are not exposed in the
/// current package surface.
pub struct ControllerBuilder<M: ControllerModel, R: ReportingMode> {
    _types: PhantomData<fn() -> (M, R)>,
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
