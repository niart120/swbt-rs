//! Public error types.

use std::fmt;

/// Error categories that callers can use for recovery decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorKind {
    /// A value is outside its accepted domain.
    InvalidInput,
    /// A dynamic input is not supported by the selected controller model.
    UnsupportedInput,
}

/// Error returned by `swbt` operations.
#[derive(Debug)]
pub struct Error {
    kind: ErrorKind,
    message: String,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
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

impl std::error::Error for Error {}

/// Result type returned by `swbt` operations.
pub type Result<T> = std::result::Result<T, Error>;
