//! Controller identity values and the internal pairing-profile envelope.

mod colors;
mod document;

#[cfg(test)]
mod document_tests;

pub use crate::model::ControllerKind;
pub use colors::{ControllerColors, Rgb24};
pub(crate) use document::{PairingProfile, ProfileDocument};
