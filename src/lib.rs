#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Rust library for NX-compatible virtual Bluetooth HID input devices.
//!
//! The crate provides model-typed controllers, model-valid input values,
//! read-only status snapshots, pairing profiles, and Periodic or Direct input
//! reporting. Building a controller does not open an adapter or start a worker;
//! input operations return [`ErrorKind::TransportClosed`] until
//! [`Controller::open`] succeeds. Selecting an existing profile reads and
//! validates it during construction.
//!
//! The `bumble` feature enables descriptor-only USB adapter discovery and the
//! Bluetooth HCI runtime. Without it, adapter discovery, opening, and profile
//! creation return [`ErrorKind::UnsupportedCapability`] before transport or
//! filesystem side effects. [`Controller::pair`] waits for one session to reach
//! protocol readiness. [`Controller::reconnect`] uses the stored Classic bond
//! without deleting it or falling back to pairing; [`Controller::connect`] can
//! permit that fallback only when configured by the caller.
//!
//! [`PairingProfile`] preserves unknown schema-v2 extension fields. Profile
//! creation does not replace an existing target, and pairing-key updates replace
//! the complete profile atomically. A new connection session resets committed
//! input to neutral and excludes events from earlier sessions.
//!
//! Explicit close drains accepted interrupt sends to the controller flow-control
//! window, disconnects, closes the transport, joins the worker, and reports
//! cleanup or join failures. It does not wait for completion credit for every
//! in-flight packet. Dropping a controller is bounded best-effort shutdown: it
//! omits neutral reporting and draining and cannot report failures. Runtime
//! With the `diagnostics-schema` feature, runtime changes emit secret-free
//! schema-v1 `tracing` events on the `swbt::diagnostics` target. Featureless
//! builds do not compile that stable event emitter. Accepted-report counters
//! indicate transport acceptance, not radio delivery or console behavior.
//! Platform support and hardware verification limits are documented in
//! `docs/platform-support.md`.
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
pub use connection::{
    ConnectOptions, ConnectionPath, ConnectionResult, ConnectionStatus, CreateProfileOptions,
};
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
    ControllerColors, ControllerKind, LocalAddress, PairingProfile, ProfileIdentity,
    ProfileIdentityKind, ProfileSummary, Rgb24, inspect_profile,
};
pub use reporting::{ReportingKind, ReportingMode};
