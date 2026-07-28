#![forbid(unsafe_code)]
//! Rust library for NX-compatible virtual Bluetooth HID input devices.
//!
//! The controller model, input values, protocol, and runtime are implemented in
//! roadmap order. The M0 foundation establishes this library target before any
//! Bluetooth transport is exposed.

pub mod error;
pub mod input;
pub mod model;
pub mod profile;
pub mod reporting;

pub use error::{Error, ErrorKind, Result};
pub use input::{Button, ButtonKind, JoyConLButton, JoyConRButton, ProButton, Stick};
pub use model::{ControllerModel, HasDualSticks, HasLeftStick, HasRightStick};
pub use profile::ControllerKind;
pub use reporting::{ReportingKind, ReportingMode};
