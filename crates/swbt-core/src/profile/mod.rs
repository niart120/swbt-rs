//! Controller identity, color, and typed pairing-profile values.

mod colors;
mod document;
mod identity;
mod summary;

#[cfg(test)]
mod document_tests;
#[cfg(test)]
mod identity_tests;

pub use crate::model::ControllerKind;
pub use colors::{ControllerColors, Rgb24};
pub use document::PairingProfile;
pub use identity::{LocalAddress, ProfileIdentity};
pub use summary::{ProfileIdentityKind, ProfileSummary, inspect_profile};

#[doc(hidden)]
pub mod __private {
    pub use super::document::{ProfileDocument, StoredClassicBond};
}
