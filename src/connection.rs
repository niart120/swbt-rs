use std::time::Duration;

use crate::ProfileIdentity;

/// Settings used when creating and pairing a new controller profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateProfileOptions {
    /// Identity to persist in the new profile.
    pub identity: ProfileIdentity,
    /// Time allowed for pairing to reach normal-input readiness.
    pub pair_timeout: Duration,
}
