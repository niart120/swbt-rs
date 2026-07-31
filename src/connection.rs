use std::time::Duration;

use crate::ProfileIdentity;

/// Settings for reconnect-first connection establishment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectOptions {
    /// Time allowed for the selected connection path to reach readiness.
    pub timeout: Duration,
    /// Whether a missing usable bond may fall back to explicit pairing.
    pub allow_pairing: bool,
}

/// Connection path that reached readiness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionPath {
    /// A stored Classic bond was used.
    Reconnected,
    /// No usable bond existed and pairing was explicitly allowed.
    Paired,
}

/// Recoverable outcome returned by the `try_*` connection methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionStatus {
    /// The controller reached protocol readiness.
    Connected,
    /// The profile had no usable Classic bond.
    NoBond,
    /// The connection did not reach readiness before its deadline.
    TimedOut,
    /// The connection ended before reaching readiness.
    Failed,
}

/// Structured result from a recoverable connection attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionResult {
    /// Recoverable outcome category.
    pub status: ConnectionStatus,
    /// Successful path, or `None` when readiness was not reached.
    pub path: Option<ConnectionPath>,
    /// Human-readable failure context whose wording is not a stable contract.
    pub message: Option<String>,
}

/// Settings used when creating and pairing a new controller profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateProfileOptions {
    /// Identity to persist in the new profile.
    pub identity: ProfileIdentity,
    /// Time allowed for pairing to reach normal-input readiness.
    pub pair_timeout: Duration,
}
