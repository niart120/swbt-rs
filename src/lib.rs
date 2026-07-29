#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Rust library for NX-compatible virtual Bluetooth HID input devices.
//!
//! The current package surface provides typed controller identities,
//! model-valid input values, configured controller construction, and read-only
//! status and input snapshots, typed input operations, and explicit close
//! operations. Building a controller without a profile is ephemeral; selecting
//! an existing profile reads and validates that document. Construction does not
//! open an adapter or start a worker. A configured controller therefore returns
//! [`ErrorKind::TransportClosed`] from input operations until a later lifecycle
//! entrypoint installs a ready runtime. Public open, pair, and profile-creation
//! entrypoints are available, but the current package has no concrete
//! Bluetooth transport backend. Open and pair return
//! [`ErrorKind::UnsupportedCapability`]. An otherwise valid profile-creation
//! request returns the same error after its read-only target preflight and does
//! not create a file.
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

mod adapter;
mod connection;
pub mod controller;
mod diagnostics;
pub mod error;
pub mod input;
pub mod model;
pub mod profile;
mod protocol;
pub mod reporting;
mod runtime;

pub use adapter::AdapterSelector;
pub use connection::CreateProfileOptions;
pub use controller::{
    Controller, ControllerBuilder, DirectJoyConL, DirectJoyConR, DirectProController, JoyConL,
    JoyConR, ProController,
};
pub use diagnostics::{GamepadStatus, LifecycleState};
pub use error::{Error, ErrorKind, Result};
pub use input::{
    Button, ButtonKind, ImuFrame, ImuSamples, InputState, JoyConLButton, JoyConLInputState,
    JoyConRButton, JoyConRInputState, ProButton, ProInputState, Stick,
};
pub use model::{ControllerModel, HasDualSticks, HasLeftStick, HasRightStick};
pub use profile::{ControllerColors, ControllerKind, LocalAddress, ProfileIdentity, Rgb24};
pub use reporting::{ReportingKind, ReportingMode};
