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
    related: Option<Box<Error>>,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
            related: None,
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
            related: None,
        }
    }

    /// Returns the stable category for this error.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns the next independent failure observed during the same operation.
    ///
    /// Related failures are separate from the primary causal chain returned by
    /// [`std::error::Error::source`]. Repeated failures are linked in occurrence
    /// order.
    #[must_use]
    pub fn related_error(&self) -> Option<&Error> {
        self.related.as_deref()
    }

    pub(crate) fn with_related(mut self, related: Error) -> Error {
        let mut tail = &mut self.related;
        while let Some(error) = tail {
            tail = &mut error.related;
        }
        *tail = Some(Box::new(related));
        self
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

#[cfg(test)]
mod tests {
    use std::{error::Error as StdError, fmt};

    use super::{Error, ErrorKind};

    #[test]
    fn related_error_keeps_parallel_failure_sources_out_of_display_and_debug() {
        let primary = Error::with_source(
            ErrorKind::ConnectionTimeout,
            "pairing timed out",
            Sentinel("secret primary source"),
        );
        let cleanup = Error::with_source(
            ErrorKind::WorkerFailed,
            "controller runtime cleanup failed",
            Sentinel("secret cleanup source"),
        );

        let error = primary.with_related(cleanup);

        assert_eq!(error.kind(), ErrorKind::ConnectionTimeout);
        assert_eq!(
            error.source().expect("primary source").to_string(),
            "secret primary source"
        );
        let related = error.related_error().expect("cleanup error");
        assert_eq!(related.kind(), ErrorKind::WorkerFailed);
        assert_eq!(
            related.source().expect("cleanup source").to_string(),
            "secret cleanup source"
        );
        assert!(!error.to_string().contains("secret"));
        assert!(!format!("{error:?}").contains("secret"));
    }

    #[test]
    fn repeated_related_errors_are_appended_in_occurrence_order() {
        let error = Error::new(ErrorKind::ConnectionFailed, "primary")
            .with_related(Error::new(ErrorKind::WorkerFailed, "cleanup"))
            .with_related(Error::new(ErrorKind::WorkerFailed, "join"));

        let cleanup = error.related_error().expect("cleanup error");
        assert_eq!(cleanup.to_string(), "cleanup");
        let join = cleanup.related_error().expect("join error");
        assert_eq!(join.to_string(), "join");
        assert!(join.related_error().is_none());
    }

    #[derive(Debug)]
    struct Sentinel(&'static str);

    impl fmt::Display for Sentinel {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(self.0)
        }
    }

    impl StdError for Sentinel {}
}
