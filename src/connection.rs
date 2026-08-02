use std::time::Duration;

use crate::ProfileIdentity;

/// Settings for reconnect-first connection establishment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectOptions {
    /// Time allowed for the selected connection path to reach readiness.
    ///
    /// Values above `u64::MAX` nanoseconds are rejected as invalid input.
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

/// Settings used when creating and pairing a new controller profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateProfileOptions {
    /// Identity to persist in the new profile.
    pub identity: ProfileIdentity,
    /// Time allowed for pairing to reach normal-input readiness.
    ///
    /// Values above `u64::MAX` nanoseconds are rejected as invalid input.
    pub pair_timeout: Duration,
}
