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
//! [`ErrorKind::UnsupportedCapability`] before transport side effects.
//! [`Controller::pair`] requires an open runtime and waits for one connection
//! session to complete NX readiness; timeout and pre-readiness disconnect are
//! returned as failures. The production USB runtime owns the same Classic
//! pairing, SDP, and HID session exercised by the virtual packet-path tests;
//! pairing and input have been exercised on Windows 11 with a CSR8510 A10 and
//! Switch 2 system version 22.5.0 (user-reported). Other operating systems,
//! adapters, system versions, and long-run reliability remain unverified. With
//! the `bumble` feature, profile creation publishes a valid empty envelope
//! without replacing an existing target before it opens the adapter and waits
//! for pairing readiness. Feature-disabled profile creation stops before
//! creating a file. Pairing-key persistence and reconnect from an existing
//! profile remain unavailable. [`PairingProfile`] parses and writes complete
//! schema v2 JSON without discarding unknown extension fields; filesystem
//! update is not part of that value API. Explicit close waits for cleanup
//! completion, joins the worker, and returns cleanup or join failures. Pending
//! interrupt sends are drained until they enter the controller's flow-control
//! window; close does not wait for the controller to return completion credit
//! for every in-flight packet.
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
pub use profile::{
    ControllerColors, ControllerKind, LocalAddress, PairingProfile, ProfileIdentity, Rgb24,
};
pub use reporting::{ReportingKind, ReportingMode};
