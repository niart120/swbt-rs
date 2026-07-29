//! Public error types.

use std::{error::Error as StdError, fmt};

/// Error categories that callers can use for recovery decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Creating a profile requires a destination path.
    ProfilePathRequired,
    /// The requested profile does not exist.
    ProfileNotFound,
    /// A profile already exists at the requested destination.
    ProfileAlreadyExists,
    /// The profile contents are invalid.
    InvalidProfile,
    /// The profile belongs to a different controller model.
    ProfileControllerMismatch,
    /// The transport is not ready to accept the requested operation.
    TransportClosed,
    /// Establishing the controller connection timed out.
    ConnectionTimeout,
    /// Establishing the controller connection failed.
    ConnectionFailed,
    /// A controller protocol operation failed.
    Protocol,
    /// A value is outside its accepted domain.
    InvalidInput,
    /// A dynamic input is not supported by the selected controller model.
    UnsupportedInput,
    /// The requested operation requires a capability unavailable in this build.
    UnsupportedCapability,
    /// The bounded worker command queue has no remaining capacity.
    Busy,
    /// The controller worker terminated or could not complete the operation.
    WorkerFailed,
    /// The operation was rejected or interrupted because shutdown has begun.
    Shutdown,
    /// An internal invariant or unclassified operation failed.
    Internal,
}

/// Error returned by `swbt` operations.
pub struct Error {
    kind: ErrorKind,
    message: String,
    source: Option<Box<dyn StdError + Send + Sync>>,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn with_source(
        kind: ErrorKind,
        message: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    /// Returns the stable category for this error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Error")
            .field("kind", &self.kind)
            .field("message", &self.message)
            .finish_non_exhaustive()
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &(dyn StdError + 'static))
    }
}

/// Result type returned by `swbt` operations.
pub type Result<T> = std::result::Result<T, Error>;
