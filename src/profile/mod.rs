//! Controller identity, color, and typed pairing-profile values.

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
pub use document::PairingProfile;
pub(crate) use document::ProfileDocument;
pub use identity::{LocalAddress, ProfileIdentity};
pub(crate) use store::{
    FileProfileStore, ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState,
    ProfileReadPort,
};
