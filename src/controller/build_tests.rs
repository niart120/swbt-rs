use std::{
    collections::VecDeque,
    error::Error as _,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    AdapterSelector,
    diagnostics::LifecycleState,
    error::ErrorKind,
    input::InputState,
    model,
    profile::{ControllerColors, ControllerKind, Rgb24},
};

use super::{ProController, ProfileConfig, build::ProfileStore};

#[derive(Debug, PartialEq, Eq)]
enum BuildEvent {
    Read(PathBuf),
}

struct FakeProfileReader {
    reads: VecDeque<io::Result<Vec<u8>>>,
    events: Vec<BuildEvent>,
}

impl FakeProfileReader {
    fn returning(read: io::Result<Vec<u8>>) -> Self {
        Self {
            reads: VecDeque::from([read]),
            events: Vec::new(),
        }
    }

    fn rejecting_reads() -> Self {
        Self {
            reads: VecDeque::new(),
            events: Vec::new(),
        }
    }
}

impl ProfileStore for FakeProfileReader {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        self.events.push(BuildEvent::Read(path.to_owned()));
        self.reads
            .pop_front()
            .expect("unexpected extra profile read")
    }

    fn create_new(&mut self, _path: &Path, _bytes: &[u8]) -> io::Result<()> {
        panic!("build test must not create a profile")
    }

    fn update(&mut self, _path: &Path, _replacement: &[u8]) -> io::Result<()> {
        panic!("build test must not update a profile")
    }
}

#[test]
fn ephemeral_build_reads_no_profile_and_returns_configured_controller() {
    let colors = custom_colors();
    let mut reader = FakeProfileReader::rejecting_reads();

    let controller = ProController::builder("usb:ephemeral")
        .controller_colors(colors)
        .build_with_profile_reader(&mut reader)
        .expect("ephemeral build must succeed without profile I/O");

    assert!(reader.events.is_empty());
    assert_eq!(
        controller.config().adapter,
        AdapterSelector::from("usb:ephemeral")
    );
    assert_eq!(controller.config().colors, colors);
    assert!(matches!(
        controller.config().profile,
        ProfileConfig::Ephemeral
    ));
    assert_eq!(
        controller.config().report_period(),
        Duration::from_millis(8)
    );
    assert_configured_and_neutral(&controller);
}

#[test]
fn existing_matching_profile_is_read_once_and_retained_as_typed_config() {
    let path = PathBuf::from("profiles/existing-pro.json");
    let mut reader = FakeProfileReader::returning(Ok(valid_profile("pro")));

    let controller = ProController::builder("usb:persistent")
        .profile_path(path.clone())
        .build_with_profile_reader(&mut reader)
        .expect("matching existing profile must build");

    assert_eq!(reader.events, [BuildEvent::Read(path.clone())]);
    let ProfileConfig::Persistent {
        path: configured_path,
        profile,
    } = &controller.config().profile
    else {
        panic!("existing profile must produce persistent configuration");
    };
    assert_eq!(configured_path, &path);
    assert_eq!(profile.controller_kind(), ControllerKind::Pro);
    assert_configured_and_neutral(&controller);
}

#[test]
fn missing_and_other_read_failures_are_distinguished_without_path_disclosure() {
    let secret_path = PathBuf::from("profiles/SECRET_PROFILE_PATH.json");

    for (source, expected_kind) in [
        (
            io::Error::new(io::ErrorKind::NotFound, "missing"),
            ErrorKind::ProfileNotFound,
        ),
        (
            io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
            ErrorKind::Internal,
        ),
    ] {
        let mut reader = FakeProfileReader::returning(Err(source));
        let result = ProController::builder("usb:0")
            .profile_path(secret_path.clone())
            .build_with_profile_reader(&mut reader);
        let error = match result {
            Ok(_) => panic!("profile read failure must fail construction"),
            Err(error) => error,
        };

        assert_eq!(reader.events, [BuildEvent::Read(secret_path.clone())]);
        assert_eq!(error.kind(), expected_kind);
        assert!(error.source().is_some());
        assert!(!error.to_string().contains("SECRET_PROFILE_PATH"));
        assert!(!format!("{error:?}").contains("SECRET_PROFILE_PATH"));
    }
}

#[test]
fn mismatched_profile_is_rejected_after_one_profile_read() {
    let path = PathBuf::from("profiles/joycon-left.json");
    let mut reader = FakeProfileReader::returning(Ok(valid_profile("joycon_l")));

    let result = ProController::builder("usb:0")
        .profile_path(path.clone())
        .build_with_profile_reader(&mut reader);
    let error = match result {
        Ok(_) => panic!("model-mismatched profile must fail construction"),
        Err(error) => error,
    };

    assert_eq!(reader.events, [BuildEvent::Read(path)]);
    assert_eq!(error.kind(), ErrorKind::ProfileControllerMismatch);
}

#[test]
fn builder_validation_failure_precedes_profile_read() {
    let mut reader = FakeProfileReader::rejecting_reads();

    let result = ProController::builder("usb:0")
        .profile_path("profiles/existing-pro.json")
        .report_period(Duration::ZERO)
        .build_with_profile_reader(&mut reader);
    let error = match result {
        Ok(_) => panic!("invalid builder settings must fail construction"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert!(reader.events.is_empty());
}

fn assert_configured_and_neutral<R: crate::reporting::ReportingMode>(
    controller: &crate::Controller<model::Pro, R>,
) {
    let status = controller.status();

    assert_eq!(status.lifecycle, LifecycleState::Configured);
    assert!(!status.connected);
    assert_eq!(status.input_reports_accepted, 0);
    assert_eq!(status.replies_accepted, 0);
    assert_eq!(controller.snapshot(), InputState::<model::Pro>::neutral());
}

fn custom_colors() -> ControllerColors {
    ControllerColors::new(
        Rgb24::new(0x01, 0x02, 0x03),
        Rgb24::new(0x04, 0x05, 0x06),
        Rgb24::new(0x07, 0x08, 0x09),
        Rgb24::new(0x0A, 0x0B, 0x0C),
    )
}

fn valid_profile(controller_kind: &str) -> Vec<u8> {
    format!(
        r#"{{
            "format": "swbt.profile",
            "schema_version": 2,
            "controller_kind": "{controller_kind}",
            "identity": {{ "kind": "adapter-default" }},
            "key_store": {{ "namespaces": {{}} }}
        }}"#
    )
    .into_bytes()
}
