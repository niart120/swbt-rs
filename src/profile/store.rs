use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const TEMP_CREATE_ATTEMPTS: usize = 128;
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

pub(crate) struct FileProfileStore;

impl ProfileCreateTargetPort for FileProfileStore {
    fn inspect(&mut self, path: &Path) -> io::Result<ProfileCreateTargetState> {
        inspect_target(path)
    }
}

impl ProfileReadPort for FileProfileStore {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}

impl ProfileCreatePort for FileProfileStore {
    fn create_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let parent = usable_parent(path);
        fs::create_dir_all(parent)?;
        let (temporary_path, mut temporary) = create_temporary(parent, path)?;
        let publish = (|| {
            temporary.write_all(bytes)?;
            temporary.flush()?;
            temporary.sync_all()?;
            drop(temporary);
            fs::hard_link(&temporary_path, path).map_err(|source| {
                if fs::symlink_metadata(path).is_ok() {
                    io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "profile creation target already exists",
                    )
                } else {
                    source
                }
            })?;
            fs::remove_file(&temporary_path)?;
            sync_parent_directory(parent)
        })();

        if publish.is_err() {
            let _cleanup = fs::remove_file(&temporary_path);
        }
        publish
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

fn inspect_target(path: &Path) -> io::Result<ProfileCreateTargetState> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(ProfileCreateTargetState::Existing),
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            Ok(ProfileCreateTargetState::Absent)
        }
        Err(source) => Err(source),
    }
}

fn usable_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn create_temporary(parent: &Path, target: &Path) -> io::Result<(PathBuf, File)> {
    let target_name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "profile creation target must have a file name",
        )
    })?;
    for _ in 0..TEMP_CREATE_ATTEMPTS {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(target_name);
        name.push(format!(
            ".swbt-profile-{}-{sequence}.tmp",
            std::process::id()
        ));
        let temporary_path = parent.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(source),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique profile temporary file",
    ))
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };

    use crate::{
        model,
        profile::{PairingProfile, ProfileDocument},
    };

    use super::{
        FileProfileStore, ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState,
        ProfileReadPort,
    };

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn file_profile_store_creates_parents_and_reopens_exact_typed_bytes() {
        let temp = TempDirectory::new("create");
        let target = temp.path().join("nested").join("pro.json");
        let expected = ProfileDocument::empty_adapter_default::<model::Pro>()
            .to_json_bytes()
            .expect("serialize empty Pro profile");
        let mut store = FileProfileStore;

        assert_eq!(
            store.inspect(&target).expect("inspect absent target"),
            ProfileCreateTargetState::Absent
        );
        store
            .create_new(&target, &expected)
            .expect("create profile through production store");
        assert_eq!(store.read(&target).expect("read created profile"), expected);

        let parsed = ProfileDocument::parse_json(&expected).expect("created JSON is valid");
        PairingProfile::<model::Pro>::try_from(parsed).expect("created profile is Pro-typed");
        assert_eq!(
            directory_names(target.parent().expect("target has a parent")),
            ["pro.json"],
            "successful publication must remove its same-directory temporary file"
        );
    }

    #[test]
    fn file_profile_store_never_replaces_a_racing_file_or_directory() {
        let temp = TempDirectory::new("no-replace");
        let mut store = FileProfileStore;
        let file_target = temp.path().join("racing.json");
        let competitor = b"competitor-owned bytes";

        assert_eq!(
            store.inspect(&file_target).expect("inspect absent target"),
            ProfileCreateTargetState::Absent
        );
        fs::write(&file_target, competitor).expect("create racing target");
        let error = store
            .create_new(&file_target, b"replacement")
            .expect_err("create-new must not replace a racing target");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            fs::read(&file_target).expect("read racing target"),
            competitor
        );
        assert_eq!(
            directory_names(temp.path()),
            ["racing.json"],
            "failed publication must clean its temporary file"
        );

        let directory_target = temp.path().join("existing-directory");
        fs::create_dir(&directory_target).expect("create directory target");
        let error = store
            .create_new(&directory_target, b"replacement")
            .expect_err("create-new must not replace a directory");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(directory_target.is_dir());
        assert_eq!(
            directory_names(temp.path()),
            ["existing-directory", "racing.json"],
            "directory conflict must clean its temporary file"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn file_profile_store_never_replaces_a_dangling_symlink() {
        let temp = TempDirectory::new("symlink");
        let missing = temp.path().join("missing.json");
        let target = temp.path().join("profile.json");
        if let Err(source) = create_file_symlink(&missing, &target) {
            #[cfg(windows)]
            if source.kind() == io::ErrorKind::PermissionDenied
                || source.raw_os_error() == Some(1314)
            {
                eprintln!("symlink case skipped because Windows denied symlink creation");
                return;
            }
            panic!("create dangling profile symlink: {source}");
        }
        let mut store = FileProfileStore;

        assert_eq!(
            store.inspect(&target).expect("inspect symlink target"),
            ProfileCreateTargetState::Existing
        );
        let error = store
            .create_new(&target, b"replacement")
            .expect_err("create-new must not replace a dangling symlink");
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            fs::symlink_metadata(&target)
                .expect("symlink remains")
                .file_type()
                .is_symlink()
        );
        assert!(!missing.exists());
        assert_eq!(
            directory_names(temp.path()),
            ["profile.json"],
            "symlink conflict must clean its temporary file"
        );
    }

    fn directory_names(path: &Path) -> Vec<String> {
        let mut names = fs::read_dir(path)
            .expect("read test directory")
            .map(|entry| {
                entry
                    .expect("read directory entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    struct TempDirectory {
        path: PathBuf,
    }

    impl TempDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "swbt-rs-profile-store-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create unique test directory");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).expect("remove test directory");
        }
    }
}
