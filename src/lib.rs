#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Rust library for NX-compatible virtual Bluetooth HID input devices.
//!
//! The current package surface provides typed controller identities and
//! model-valid input values. Bluetooth transport and runtime operations are not
//! exposed yet.
//!
//! # Model-valid input
//!
//! ```
//! use swbt::{ButtonKind, ProButton, ProInputState, Stick};
//!
//! # fn main() -> swbt::Result<()> {
//! let state = ProInputState::neutral()
//!     .with_buttons([ProButton::A])
//!     .with_left_stick(Stick::up(0.5)?);
//!
//! assert_eq!(
//!     state.buttons().map(|button| button.kind()).collect::<Vec<_>>(),
//!     [ButtonKind::A]
//! );
//! # Ok(())
//! # }
//! ```

pub mod controller;
pub mod error;
pub mod input;
pub mod model;
pub mod profile;
pub mod reporting;

pub use controller::{
    Controller, ControllerBuilder, DirectJoyConL, DirectJoyConR, DirectProController, JoyConL,
    JoyConR, ProController,
};
pub use error::{Error, ErrorKind, Result};
pub use input::{
    Button, ButtonKind, ImuFrame, ImuSamples, InputState, JoyConLButton, JoyConLInputState,
    JoyConRButton, JoyConRInputState, ProButton, ProInputState, Stick,
};
pub use model::{ControllerModel, HasDualSticks, HasLeftStick, HasRightStick};
pub use profile::ControllerKind;
pub use reporting::{ReportingKind, ReportingMode};
