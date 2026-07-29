use std::{
    fs, io,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use swbt::{CreateProfileOptions, ErrorKind, ProController, ProfileIdentity};
#[cfg(not(feature = "bumble"))]
use swbt::{DirectJoyConL, DirectJoyConR, DirectProController, JoyConL, JoyConR, LifecycleState};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(not(feature = "bumble"))]
#[test]
fn public_open_reports_missing_backend_and_pair_requires_an_open_runtime() {
    macro_rules! assert_unavailable_lifecycle {
        ($controller:expr) => {{
            let mut controller = $controller;
            let configured_status = controller.status();
            let configured_snapshot = controller.snapshot();

            assert_eq!(configured_status.lifecycle, LifecycleState::Configured);
            assert!(!configured_status.connected);

            for _ in 0..2 {
                assert_error_kind(controller.open(), ErrorKind::UnsupportedCapability);
                assert_eq!(controller.status(), configured_status);
                assert_eq!(controller.snapshot(), configured_snapshot);
            }
            for _ in 0..2 {
                assert_error_kind(
                    controller.pair(Duration::from_secs(2)),
                    ErrorKind::TransportClosed,
                );
                assert_eq!(controller.status(), configured_status);
                assert_eq!(controller.snapshot(), configured_snapshot);
            }

            assert_error_kind(controller.neutral(), ErrorKind::TransportClosed);
        }};
    }

    assert_unavailable_lifecycle!(
        ProController::builder("unavailable:pro-periodic")
            .build()
            .expect("build configured periodic Pro controller")
    );
    assert_unavailable_lifecycle!(
        DirectProController::builder("unavailable:pro-direct")
            .build()
            .expect("build configured direct Pro controller")
    );
    assert_unavailable_lifecycle!(
        JoyConL::builder("unavailable:joycon-l-periodic")
            .build()
            .expect("build configured periodic left Joy-Con")
    );
    assert_unavailable_lifecycle!(
        DirectJoyConL::builder("unavailable:joycon-l-direct")
            .build()
            .expect("build configured direct left Joy-Con")
    );
    assert_unavailable_lifecycle!(
        JoyConR::builder("unavailable:joycon-r-periodic")
            .build()
            .expect("build configured periodic right Joy-Con")
    );
    assert_unavailable_lifecycle!(
        DirectJoyConR::builder("unavailable:joycon-r-direct")
            .build()
            .expect("build configured direct right Joy-Con")
    );
}

#[cfg(not(feature = "bumble"))]
#[test]
fn public_create_profile_reports_missing_backend_without_creating_the_target() {
    let temp = TempDir::new("create-profile");
    let target = temp.path().join("new-profile.json");
    assert_path_absent(&target);

    let result = ProController::builder("unavailable:create-profile")
        .profile_path(&target)
        .create_profile(adapter_default_options());

    assert_controller_error_kind(result, ErrorKind::UnsupportedCapability);
    assert_path_absent(&target);
}

#[cfg(feature = "bumble")]
#[test]
fn public_create_profile_reaches_the_production_backend_after_persisting() {
    let temp = TempDir::new("production-create-profile");
    let target = temp.path().join("new-profile.json");
    assert_path_absent(&target);

    let result = ProController::builder("not-a-usb-selector")
        .profile_path(&target)
        .create_profile(adapter_default_options());
    let error = match result {
        Ok(_) => panic!("invalid selector must not return a controller"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::TransportOpen);
    let value: serde_json::Value = serde_json::from_slice(
        &fs::read(&target).expect("production create-profile must persist before transport open"),
    )
    .expect("persisted profile must be valid JSON");
    assert_eq!(value["format"], "swbt.profile");
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["controller_kind"], "pro");
    assert_eq!(value["identity"]["kind"], "adapter-default");
    assert_eq!(value["key_store"]["namespaces"], serde_json::json!({}));
}

#[test]
fn public_create_profile_keeps_preflight_errors_ahead_of_backend_availability() {
    assert_controller_error_kind(
        ProController::builder("unavailable:no-profile-path")
            .create_profile(adapter_default_options()),
        ErrorKind::ProfilePathRequired,
    );

    let temp = TempDir::new("existing-profile");
    let target = temp.path().join("existing-profile.json");
    fs::write(&target, b"existing profile sentinel").expect("create existing profile fixture");
    assert_controller_error_kind(
        ProController::builder("unavailable:existing-profile")
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
        ProController::builder("unavailable:existing-directory")
            .profile_path(&directory_target)
            .create_profile(adapter_default_options()),
        ErrorKind::ProfileAlreadyExists,
    );
    assert!(
        fs::symlink_metadata(&directory_target)
            .expect("existing directory fixture must remain")
            .is_dir()
    );
}

#[cfg(any(unix, windows))]
#[test]
fn public_create_profile_treats_a_dangling_symlink_as_an_existing_target() {
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
    assert_path_absent(&missing);

    assert_controller_error_kind(
        ProController::builder("unavailable:dangling-symlink")
            .profile_path(&target)
            .create_profile(adapter_default_options()),
        ErrorKind::ProfileAlreadyExists,
    );
    let metadata =
        fs::symlink_metadata(&target).expect("dangling profile symlink fixture must remain");
    assert!(metadata.file_type().is_symlink());
    assert_path_absent(&missing);
}

fn adapter_default_options() -> CreateProfileOptions {
    CreateProfileOptions {
        identity: ProfileIdentity::AdapterDefault,
        pair_timeout: Duration::from_secs(60),
    }
}

#[cfg(not(feature = "bumble"))]
fn assert_error_kind(result: swbt::Result<()>, expected: ErrorKind) {
    let error = result.expect_err("operation must fail while the backend is unavailable");
    assert_eq!(error.kind(), expected);
}

fn assert_controller_error_kind<T>(result: swbt::Result<T>, expected: ErrorKind) {
    let error = match result {
        Ok(_) => panic!("controller construction must fail while the backend is unavailable"),
        Err(error) => error,
    };
    assert_eq!(error.kind(), expected);
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
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let id = NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed);

    std::env::temp_dir().join(format!(
        "swbt-rs-t34-{label}-{}-{timestamp}-{id}",
        process::id()
    ))
}

fn assert_path_absent(path: &Path) {
    match fs::symlink_metadata(path) {
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Ok(_) => panic!("profile target must not have a filesystem entry"),
        Err(source) => panic!("inspect absent profile target: {source}"),
    }
}

#[cfg(unix)]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_file_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}
