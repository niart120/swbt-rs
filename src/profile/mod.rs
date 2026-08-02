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
#[cfg(feature = "bumble")]
pub(crate) use document::ProfileClassicBond;
pub(crate) use document::ProfileDocument;
pub use identity::{LocalAddress, ProfileIdentity};
pub(crate) use store::{FileProfileStore, ProfileStore};
pub use summary::{ProfileIdentityKind, ProfileSummary, inspect_profile};
