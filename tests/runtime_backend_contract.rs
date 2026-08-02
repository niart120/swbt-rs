use std::{
    error::Error as _,
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use swbt::{CreateProfileOptions, ErrorKind, LifecycleState, ProController, ProfileIdentity};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn public_open_reaches_the_required_backend_without_a_feature_gate() {
    let mut controller = ProController::builder("usb:0a12:0001/secret-serial[metadata]")
        .build()
        .expect("build side-effect-free controller");

    let error = controller
        .open()
        .expect_err("invalid selector must reach backend validation");

    assert_eq!(error.kind(), ErrorKind::TransportOpen);
    assert!(error.source().is_some());
    assert!(!error.to_string().contains("secret-serial"));
    assert!(!format!("{error:?}").contains("secret-serial"));
    assert_eq!(controller.status().lifecycle, LifecycleState::Configured);
}

#[test]
fn public_create_profile_reaches_the_required_backend_after_persisting() {
    let temp = TempDir::new("production-create-profile");
    let target = temp.path().join("new-profile.json");
    assert_path_absent(&target);

    let result = ProController::builder("usb:0a12:0001/secret-serial[metadata]")
        .profile_path(&target)
        .create_profile(adapter_default_options());
    let error = match result {
        Ok(_) => panic!("invalid selector must not return a controller"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::TransportOpen);
    let source = error
        .source()
        .expect("transport-open error must retain its typed source");
    assert!(!error.to_string().contains("secret-serial"));
    assert!(!format!("{error:?}").contains("secret-serial"));
    assert!(!source.to_string().contains("secret-serial"));
    assert!(!format!("{source:?}").contains("secret-serial"));
    assert!(error.related_error().is_none());

    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(&target).expect("production create-profile must persist before transport open"),
    )
    .expect("persisted profile must be valid JSON");
    assert_eq!(value["format"], "swbt.profile");
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["controller_kind"], "pro");
    assert_eq!(value["identity"]["kind"], "adapter-default");
    assert_eq!(value["key_store"]["namespaces"], serde_json::json!({}));
    assert_eq!(
        directory_paths(temp.path()),
        [target],
        "failed open must not leave a profile temporary file"
    );
}

#[test]
fn public_create_profile_checks_builder_and_target_before_runtime_open() {
    assert_controller_error_kind(
        ProController::builder("invalid-selector").create_profile(adapter_default_options()),
        ErrorKind::ProfilePathRequired,
    );

    let temp = TempDir::new("existing-profile");
    let target = temp.path().join("existing-profile.json");
    fs::write(&target, b"existing profile sentinel").expect("create existing profile fixture");
    assert_controller_error_kind(
        ProController::builder("invalid-selector")
            .profile_path(&target)
            .create_profile(adapter_default_options()),
        ErrorKind::ProfileAlreadyExists,
    );
    assert_eq!(
        fs::read(&target).expect("read existing profile fixture"),
        b"existing profile sentinel"
    );

    let directory_target = temp.path().join("existing-directory");
    fs::create_dir(&directory_target).expect("create existing directory fixture");
    assert_controller_error_kind(
        ProController::builder("invalid-selector")
            .profile_path(&directory_target)
            .create_profile(adapter_default_options()),
        ErrorKind::ProfileAlreadyExists,
    );
    assert!(directory_target.is_dir());
}

#[cfg(any(unix, windows))]
#[test]
fn public_create_profile_never_replaces_a_dangling_symlink() {
    let temp = TempDir::new("dangling-symlink");
    let missing = temp.path().join("missing-target.json");
    let target = temp.path().join("dangling-profile.json");
    if let Err(source) = create_file_symlink(&missing, &target) {
        #[cfg(windows)]
        if source.kind() == io::ErrorKind::PermissionDenied || source.raw_os_error() == Some(1314) {
            eprintln!("dangling-symlink case skipped because Windows denied symlink creation");
            return;
        }
        panic!("create dangling profile symlink fixture: {source}");
    }

    assert_controller_error_kind(
        ProController::builder("invalid-selector")
            .profile_path(&target)
            .create_profile(adapter_default_options()),
        ErrorKind::ProfileAlreadyExists,
    );
    assert!(
        fs::symlink_metadata(&target)
            .expect("dangling profile symlink fixture must remain")
            .file_type()
            .is_symlink()
    );
    assert_path_absent(&missing);
}

fn adapter_default_options() -> CreateProfileOptions {
    CreateProfileOptions {
        identity: ProfileIdentity::AdapterDefault,
        pair_timeout: Duration::from_secs(60),
    }
}

fn assert_controller_error_kind<T>(result: swbt::Result<T>, expected: ErrorKind) {
    let error = match result {
        Ok(_) => panic!("controller construction must fail"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), expected);
}

fn directory_paths(path: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(path)
        .expect("inspect profile directory")
        .map(|entry| entry.expect("read profile directory entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        loop {
            let path = unique_temp_dir_path(label);
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => panic!("create isolated profile fixture directory: {source}"),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        if let Err(source) = fs::remove_dir_all(&self.path) {
            if source.kind() != io::ErrorKind::NotFound && !thread::panicking() {
                panic!("remove isolated profile fixture directory: {source}");
            }
        }
    }
}

fn unique_temp_dir_path(label: &str) -> PathBuf {
    let sequence = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "swbt-rs-{label}-{}-{timestamp}-{sequence}",
        process::id()
    ))
}

fn assert_path_absent(path: &Path) {
    assert!(!path.exists(), "path must not exist: {}", path.display());
    assert!(
        fs::symlink_metadata(path).is_err(),
        "path must not be a dangling symlink: {}",
        path.display()
    );
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}
