#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Backend-independent values for `swbt`.
//!
//! Use this package for model-valid input, pairing-profile data, and shared
//! errors without linking the Bluetooth runtime, USB support, or a profile file
//! writer. The `swbt-rs` package re-exports these public values as the same Rust
//! types and adds controller runtime operations. The protocol engine in this
//! package is runtime support and is not a stable user-facing API.
//!
//! # Example
//!
//! ```
//! use swbt_core::{ButtonKind, ProButton, ProInputState, Stick};
//!
//! let state = ProInputState::neutral()
//!     .with_buttons([ProButton::A])
//!     .with_left_stick(Stick::up(0.5)?);
//! let pressed = state.buttons().map(|button| button.kind()).collect::<Vec<_>>();
//! assert_eq!(pressed, [ButtonKind::A]);
//! # Ok::<(), swbt_core::Error>(())
//! ```

pub mod error;
pub mod input;
pub mod model;
pub mod profile;
#[allow(missing_docs)]
mod protocol;

/// Runtime integration details shared with the `swbt` package.
///
/// This module is not a stable user-facing protocol API.
#[doc(hidden)]
pub mod __private {
    pub use crate::model::{
        ButtonWirePosition, HidSdpPolicySpec, ModelProtocolSpec, SensorCalibration,
        button_wire_position,
    };
    pub use crate::profile::__private::{ProfileDocument, StoredClassicBond};
    pub use crate::protocol::{
        ImuEncodingState, InputPreparation, OutputReport, PreparedOutputAction,
        PreparedSessionReply, PreparedSubcommandReply, ProtocolError, ProtocolSession, RawRumble,
        SubcommandRequest, SwitchHidProtocol, parse_output_report,
    };
}

pub use error::{Error, ErrorKind, Result};
pub use input::{
    Button, ButtonKind, ImuFrame, ImuSamples, InputState, JoyConLButton, JoyConLInputState,
    JoyConRButton, JoyConRInputState, ProButton, ProInputState, Stick,
};
pub use model::{
    ControllerKind, ControllerModel, HasDualSticks, HasLeftStick, HasRightStick, JoyConL, JoyConR,
    Pro,
};
pub use profile::{
    ControllerColors, LocalAddress, PairingProfile, ProfileIdentity, ProfileIdentityKind,
    ProfileSummary, Rgb24, inspect_profile,
};
