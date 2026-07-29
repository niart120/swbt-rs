use std::{io, path::Path};

/// Snapshot of a profile creation target before an atomic create-new attempt.
///
/// `Absent` does not reserve the path. The create-new operation must still use
/// no-replace semantics and map a concurrent conflict to `ProfileAlreadyExists`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileCreateTargetState {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 constructs the absent state in create-profile orchestration"
        )
    )]
    Absent,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 maps create-profile target conflicts before persistence"
        )
    )]
    Existing,
}

pub(crate) trait ProfileCreateTargetPort {
    /// Inspects the target without creating, replacing, or reserving it.
    fn inspect(&mut self, path: &Path) -> io::Result<ProfileCreateTargetState>;
}
