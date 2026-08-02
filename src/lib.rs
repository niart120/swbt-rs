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
//! Descriptor-only USB adapter discovery and the Bumble Bluetooth HCI runtime
//! are always included. Backend-independent model, input, profile, and protocol
//! values are provided by the `swbt-core` package and re-exported here as the
//! same types. [`Controller::pair`] waits for one session to reach protocol
//! readiness. [`Controller::reconnect`] uses the stored Classic bond without
//! deleting it or falling back to pairing; [`Controller::connect`] can permit
//! that fallback only when configured by the caller.
//!
//! [`PairingProfile`] accepts the strict Classic pairing subset emitted by
//! swbt-python 0.6.0; unknown fields, legacy Rust peer names, and LE key fields
//! are rejected. Profile creation uses one no-replace publication attempt, and
//! pairing-key updates atomically replace the complete profile for one live
//! writer. Multiple processes or controllers updating the same profile path are
//! unsupported. A new connection session resets committed input to neutral and
//! excludes events from earlier sessions.
//!
//! Explicit close drains accepted interrupt sends to the controller flow-control
//! window, disconnects, closes the transport, joins the worker, and reports
//! cleanup or join failures. It does not wait for completion credit for every
//! in-flight packet. Dropping a controller is bounded best-effort shutdown: it
//! omits neutral reporting and draining and cannot report failures. With the
//! `diagnostics-schema` feature, runtime changes emit secret-free
//! schema-v1 `tracing` events on the `swbt::diagnostics` target. Builds without
//! that feature do not compile the stable event emitter. Accepted-report counters
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
pub use swbt_core::{error, input, model};
pub mod profile;
mod protocol;
pub mod reporting;
mod runtime;

pub use adapter::{AdapterInfo, AdapterSelector, list_adapters};
pub use connection::{ConnectOptions, ConnectionPath, CreateProfileOptions};
pub use controller::{
    Controller, ControllerBuilder, DirectJoyConL, DirectJoyConR, DirectProController, JoyConL,
    JoyConR, ProController,
};
pub use diagnostics::{GamepadStatus, LifecycleState};
pub use reporting::{ReportingKind, ReportingMode};
pub use swbt_core::{
    Button, ButtonKind, ControllerColors, ControllerKind, ControllerModel, Error, ErrorKind,
    HasDualSticks, HasLeftStick, HasRightStick, ImuFrame, ImuSamples, InputState, JoyConLButton,
    JoyConLInputState, JoyConRButton, JoyConRInputState, LocalAddress, PairingProfile, ProButton,
    ProInputState, ProfileIdentity, ProfileIdentityKind, ProfileSummary, Result, Rgb24, Stick,
    inspect_profile,
};
