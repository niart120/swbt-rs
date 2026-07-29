//! Controller identity values and the internal pairing-profile envelope.

mod colors;
mod document;

#[cfg(test)]
mod document_tests;

pub use crate::model::ControllerKind;
pub use colors::{ControllerColors, Rgb24};
#[cfg_attr(
    not(test),
    allow(
        unused_imports,
        reason = "T29 consumes the raw and typed profile through the profile boundary"
    )
)]
pub(crate) use document::{PairingProfile, ProfileDocument};
