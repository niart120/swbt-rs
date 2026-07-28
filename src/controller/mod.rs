//! Typed controller and builder identities.

use std::marker::PhantomData;

use crate::model::{self, ControllerModel};
use crate::reporting::{self, ReportingMode};

/// A controller whose model and reporting mode are fixed by its type.
///
/// Runtime construction and lifecycle operations are introduced by the typed
/// runtime milestone.
pub struct Controller<M: ControllerModel, R: ReportingMode> {
    _types: PhantomData<fn() -> (M, R)>,
}

/// Immutable construction settings for [`Controller<M, R>`].
///
/// Adapter, profile, and runtime construction methods are introduced with
/// their I/O contracts by the typed runtime milestone.
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
