use std::{fs, io, path::Path};

pub(crate) struct FileProfileCreateTarget;

impl ProfileCreateTargetPort for FileProfileCreateTarget {
    fn inspect(&mut self, path: &Path) -> io::Result<ProfileCreateTargetState> {
        match fs::symlink_metadata(path) {
            Ok(_) => Ok(ProfileCreateTargetState::Existing),
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                Ok(ProfileCreateTargetState::Absent)
            }
            Err(source) => Err(source),
        }
    }
}

pub(crate) trait ProfileReadPort {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;
}

/// Snapshot of a profile creation target before an atomic create-new attempt.
///
/// `Absent` does not reserve the path. The create-new operation must still use
/// no-replace semantics and map a concurrent conflict to `ProfileAlreadyExists`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProfileCreateTargetState {
    Absent,
    Existing,
}

pub(crate) trait ProfileCreateTargetPort {
    /// Inspects the target without creating, replacing, or reserving it.
    fn inspect(&mut self, path: &Path) -> io::Result<ProfileCreateTargetState>;
}

pub(crate) trait ProfileCreatePort: ProfileCreateTargetPort + ProfileReadPort {
    /// Creates a new profile without replacing an existing target.
    ///
    /// An existing target must be reported as [`io::ErrorKind::AlreadyExists`].
    fn create_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;
}
