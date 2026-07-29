//! Controller identity values and the internal pairing-profile envelope.

mod colors;
mod document;
mod identity;
mod store;

#[cfg(test)]
mod document_tests;
#[cfg(test)]
mod identity_tests;

pub use crate::model::ControllerKind;
pub use colors::{ControllerColors, Rgb24};
pub(crate) use document::{PairingProfile, ProfileDocument};
pub use identity::{LocalAddress, ProfileIdentity};
pub(crate) use store::{
    ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState, ProfileReadPort,
};
