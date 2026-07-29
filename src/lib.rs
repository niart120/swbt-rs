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
//! [`ErrorKind::TransportClosed`] from input operations until [`Controller::open`]
//! installs a runtime. With the `bumble` feature, [`list_adapters`] performs
//! descriptor-only Bluetooth HCI USB discovery without opening or claiming a
//! device, while `open` claims and initializes the selected HCI adapter and
//! starts an owned worker. Without that feature, `open` returns
//! [`ErrorKind::UnsupportedCapability`] before transport side effects. Pairing
//! and profile creation remain unavailable; pairing leaves an open HCI runtime
//! unchanged, and an otherwise valid profile-creation request stops after its
//! read-only target preflight without creating a file. Explicit close waits for
//! cleanup completion, joins the worker, and returns cleanup or join failures.
//! Dropping a controller instead uses bounded best-effort shutdown: it omits
//! neutral reporting and pending-send draining and cannot report failures. Its
//! internal wait duration is not a public timing guarantee. A new connection
//! session resets the input snapshot to neutral and does not carry
//! pre-connection or previous-session input state or stale events forward.
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

pub use adapter::{AdapterInfo, AdapterSelector, list_adapters};
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
