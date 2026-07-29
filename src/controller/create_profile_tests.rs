use std::{
    collections::VecDeque,
    error::Error as _,
    io,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::{
    CreateProfileOptions, LocalAddress, ProfileIdentity,
    error::ErrorKind,
    profile::{ProfileCreateTargetPort, ProfileCreateTargetState},
};

use super::ProController;

struct FakeCreateTarget {
    results: VecDeque<io::Result<ProfileCreateTargetState>>,
    inspected: Vec<PathBuf>,
}

impl FakeCreateTarget {
    fn returning(result: io::Result<ProfileCreateTargetState>) -> Self {
        Self {
            results: VecDeque::from([result]),
            inspected: Vec::new(),
        }
    }

    fn rejecting_inspection() -> Self {
        Self {
            results: VecDeque::new(),
            inspected: Vec::new(),
        }
    }
}

impl ProfileCreateTargetPort for FakeCreateTarget {
    fn inspect(&mut self, path: &Path) -> io::Result<ProfileCreateTargetState> {
        self.inspected.push(path.to_owned());
        self.results
            .pop_front()
            .expect("unexpected extra target inspection")
    }
}

#[test]
fn builder_path_and_identity_failures_precede_target_inspection() {
    let local_address =
        LocalAddress::parse("02:12:34:56:78:9A").expect("valid local address fixture");

    let cases = [
        (
            ProController::builder("usb:0")
                .profile_path("profiles/new.json")
                .report_period(Duration::ZERO),
            adapter_default_options(Duration::from_secs(60)),
            ErrorKind::InvalidInput,
        ),
        (
            ProController::builder("usb:0"),
            adapter_default_options(Duration::from_secs(60)),
            ErrorKind::ProfilePathRequired,
        ),
        (
            ProController::builder("usb:0").profile_path("profiles/new.json"),
            CreateProfileOptions {
                identity: ProfileIdentity::LocalAddress(local_address),
                pair_timeout: Duration::from_secs(60),
            },
            ErrorKind::UnsupportedCapability,
        ),
    ];

    for (builder, options, expected_kind) in cases {
        let mut target = FakeCreateTarget::rejecting_inspection();
        let result = builder.validate_create_profile_target(options, &mut target);
        let error = match result {
            Ok(_) => panic!("invalid create-profile request must fail"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), expected_kind);
        assert!(target.inspected.is_empty());
    }
}

#[test]
fn existing_target_is_rejected_after_read_only_inspection() {
    let path = PathBuf::from("profiles/existing.json");
    let mut target = FakeCreateTarget::returning(Ok(ProfileCreateTargetState::Existing));

    let result = ProController::builder("usb:0")
        .profile_path(path.clone())
        .validate_create_profile_target(
            adapter_default_options(Duration::from_secs(60)),
            &mut target,
        );
    let error = match result {
        Ok(_) => panic!("existing target must fail create-profile validation"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::ProfileAlreadyExists);
    assert_eq!(target.inspected, [path]);
}

#[test]
fn absent_target_returns_a_preflight_plan_after_one_inspection() {
    let path = PathBuf::from("profiles/new.json");
    let mut target = FakeCreateTarget::returning(Ok(ProfileCreateTargetState::Absent));

    let plan = ProController::builder("usb:0")
        .profile_path(path.clone())
        .validate_create_profile_target(adapter_default_options(Duration::ZERO), &mut target)
        .expect("absent target must produce a validated plan");

    assert_eq!(target.inspected.as_slice(), std::slice::from_ref(&path));
    assert_eq!(plan.profile_path(), path.as_path());
    assert_eq!(plan.pair_timeout(), Duration::ZERO);
}

#[test]
fn target_inspection_failure_keeps_its_source_without_disclosing_the_path() {
    let path = PathBuf::from("profiles/SECRET_CREATE_PATH.json");
    let mut target = FakeCreateTarget::returning(Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        "denied",
    )));

    let result = ProController::builder("usb:0")
        .profile_path(path.clone())
        .validate_create_profile_target(
            adapter_default_options(Duration::from_secs(60)),
            &mut target,
        );
    let error = match result {
        Ok(_) => panic!("target inspection failure must fail validation"),
        Err(error) => error,
    };

    assert_eq!(target.inspected, [path]);
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert!(error.source().is_some());
    assert!(!error.to_string().contains("SECRET_CREATE_PATH"));
    assert!(!format!("{error:?}").contains("SECRET_CREATE_PATH"));
}

const fn adapter_default_options(pair_timeout: Duration) -> CreateProfileOptions {
    CreateProfileOptions {
        identity: ProfileIdentity::AdapterDefault,
        pair_timeout,
    }
}
