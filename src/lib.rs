#![forbid(unsafe_code)]
//! Rust library for NX-compatible virtual Bluetooth HID input devices.
//!
//! The controller model, input values, protocol, and runtime are implemented in
//! roadmap order. The M0 foundation establishes this library target before any
//! Bluetooth transport is exposed.

pub mod model;
pub mod profile;
pub mod reporting;

pub use model::ControllerModel;
pub use profile::ControllerKind;
pub use reporting::{ReportingKind, ReportingMode};
