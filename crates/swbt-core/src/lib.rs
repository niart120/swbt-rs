#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! Backend-independent values and protocol core for `swbt`.

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
