use std::{fs, io, path::Path};

use crate::error::{Error, ErrorKind};

use super::{ControllerKind, ProfileDocument};

/// Address-free identity category reported by dynamic profile inspection.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileIdentityKind {
    /// The profile uses the Bluetooth adapter's current local address.
    AdapterDefault,
    /// The profile contains an explicit locally administered address.
    ///
    /// Inspection reports only this category and never returns the address.
    LocalAddress,
}

/// Secret-free metadata from a validated pairing profile.
///
/// The summary contains counts and closed identity values only. It does not
/// retain profile JSON, Bluetooth addresses, pairing keys, paths, or unknown
/// extension fields.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileSummary {
    schema_version: u64,
    controller_kind: ControllerKind,
    identity_kind: ProfileIdentityKind,
    namespace_count: usize,
    bond_count: usize,
}

impl ProfileSummary {
    pub(super) const fn new(
        schema_version: u64,
        controller_kind: ControllerKind,
        identity_kind: ProfileIdentityKind,
        namespace_count: usize,
        bond_count: usize,
    ) -> Self {
        Self {
            schema_version,
            controller_kind,
            identity_kind,
            namespace_count,
            bond_count,
        }
    }

    /// Validates schema v2 JSON and returns only secret-free metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidProfile`] when `bytes` is not a valid
    /// pairing-profile document.
    pub fn from_json(bytes: &[u8]) -> crate::Result<Self> {
        Ok(ProfileDocument::parse_json(bytes)?.summary())
    }

    /// Returns the validated profile schema version.
    #[must_use]
    pub const fn schema_version(self) -> u64 {
        self.schema_version
    }

    /// Returns the controller model declared by the profile.
    #[must_use]
    pub const fn controller_kind(self) -> ControllerKind {
        self.controller_kind
    }

    /// Returns the address-free identity category.
    #[must_use]
    pub const fn identity_kind(self) -> ProfileIdentityKind {
        self.identity_kind
    }

    /// Returns the number of validated local key-store namespaces.
    #[must_use]
    pub const fn namespace_count(self) -> usize {
        self.namespace_count
    }

    /// Returns the total number of validated peer bonds.
    #[must_use]
    pub const fn bond_count(self) -> usize {
        self.bond_count
    }
}

/// Reads and validates a pairing profile without selecting a controller type.
///
/// The returned [`ProfileSummary`] does not contain the path or any profile
/// secret. Error display and debug text also omit the path. The underlying I/O
/// source remains available through [`std::error::Error::source`].
///
/// # Errors
///
/// Returns [`ErrorKind::ProfileNotFound`] when `path` does not exist,
/// [`ErrorKind::InvalidProfile`] when its contents are invalid, and
/// [`ErrorKind::Internal`] for other read failures.
pub fn inspect_profile(path: impl AsRef<Path>) -> crate::Result<ProfileSummary> {
    let bytes = fs::read(path.as_ref()).map_err(|source| {
        let (kind, message) = if source.kind() == io::ErrorKind::NotFound {
            (ErrorKind::ProfileNotFound, "pairing profile was not found")
        } else {
            (ErrorKind::Internal, "pairing profile could not be read")
        };
        Error::with_source(kind, message, source)
    })?;
    ProfileSummary::from_json(&bytes)
}
