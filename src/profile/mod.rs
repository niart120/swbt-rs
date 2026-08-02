//! Controller identity, color, and typed pairing-profile values.

mod colors;
mod document;
mod identity;
mod store;
mod summary;

#[cfg(test)]
mod document_tests;
#[cfg(test)]
mod identity_tests;

pub use crate::model::ControllerKind;
pub use colors::{ControllerColors, Rgb24};
pub use document::PairingProfile;
pub(crate) use document::{ProfileClassicBond, ProfileDocument};
pub use identity::{LocalAddress, ProfileIdentity};
#[cfg(feature = "bumble")]
pub(crate) use store::ProfileUpdatePort;
pub(crate) use store::{
    FileProfileStore, ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState,
    ProfileReadPort,
};
pub use summary::{ProfileIdentityKind, ProfileSummary, inspect_profile};
