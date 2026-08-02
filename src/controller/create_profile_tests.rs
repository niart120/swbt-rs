use std::{
    collections::VecDeque,
    error::Error as _,
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

#[cfg(feature = "bumble")]
use crate::LocalAddress;
use crate::{
    AdapterSelector, CreateProfileOptions, ProfileIdentity,
    error::{Error, ErrorKind},
    model::Pro,
    profile::{
        ControllerColors, ControllerKind, PairingProfile, ProfileCreatePort,
        ProfileCreateTargetPort, ProfileCreateTargetState, ProfileDocument, Rgb24,
    },
    reporting::{Periodic, ReportingKind},
    runtime::status::StatusPublisher,
};

use super::{
    ProController,
    config::{ControllerConfig, ProfileConfig},
    create::ControllerRuntime,
};

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
fn builder_path_failures_precede_target_inspection() {
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
#[cfg(feature = "bumble")]
fn local_address_is_retained_in_an_absent_target_plan() {
    let path = PathBuf::from("profiles/local-address.json");
    let address = LocalAddress::parse("02:12:34:56:78:9a").expect("valid local address fixture");
    let mut target = FakeCreateTarget::returning(Ok(ProfileCreateTargetState::Absent));

    let plan = ProController::builder("usb:0")
        .profile_path(path.clone())
        .validate_create_profile_target(
            CreateProfileOptions {
                identity: ProfileIdentity::LocalAddress(address),
                pair_timeout: Duration::from_secs(60),
            },
            &mut target,
        )
        .expect("local address must produce a preflight plan for a supported backend");

    assert_eq!(plan.identity(), ProfileIdentity::LocalAddress(address));
    assert_eq!(target.inspected, [path]);
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum CreateEvent {
    InspectTarget(PathBuf),
    CreateNew(PathBuf),
    Open {
        adapter: AdapterSelector,
        controller_kind: ControllerKind,
        identity: ProfileIdentity,
        reporting_kind: ReportingKind,
        colors: ControllerColors,
        report_period: Duration,
        pair_timeout: Duration,
    },
}

struct FakeProfileStore {
    events: Arc<Mutex<Vec<CreateEvent>>>,
    bytes: Option<Vec<u8>>,
    race_on_create: Option<Vec<u8>>,
}

impl FakeProfileStore {
    fn empty(events: Arc<Mutex<Vec<CreateEvent>>>) -> Self {
        Self {
            events,
            bytes: None,
            race_on_create: None,
        }
    }

    fn racing(events: Arc<Mutex<Vec<CreateEvent>>>, competitor: Vec<u8>) -> Self {
        Self {
            events,
            bytes: None,
            race_on_create: Some(competitor),
        }
    }
}

impl ProfileCreateTargetPort for FakeProfileStore {
    fn inspect(&mut self, path: &Path) -> io::Result<ProfileCreateTargetState> {
        lock(&self.events).push(CreateEvent::InspectTarget(path.to_owned()));
        Ok(if self.bytes.is_some() {
            ProfileCreateTargetState::Existing
        } else {
            ProfileCreateTargetState::Absent
        })
    }
}

impl ProfileCreatePort for FakeProfileStore {
    fn create_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        lock(&self.events).push(CreateEvent::CreateNew(path.to_owned()));
        if let Some(competitor) = self.race_on_create.take() {
            self.bytes = Some(competitor);
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile won the create-new race",
            ));
        }
        if self.bytes.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "profile already exists",
            ));
        }
        self.bytes = Some(bytes.to_vec());
        Ok(())
    }
}

#[test]
fn create_profile_persists_typed_profile_before_requesting_runtime() {
    let path = PathBuf::from("profiles/new-pro.json");
    let colors = ControllerColors::new(
        Rgb24::new(0x01, 0x02, 0x03),
        Rgb24::new(0x04, 0x05, 0x06),
        Rgb24::new(0x07, 0x08, 0x09),
        Rgb24::new(0x0A, 0x0B, 0x0C),
    );
    let report_period = Duration::from_millis(17);
    let pair_timeout = Duration::from_secs(60);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut store = FakeProfileStore::empty(Arc::clone(&events));
    let runtime_events = Arc::clone(&events);

    let result = ProController::builder("usb:fake")
        .profile_path(path.clone())
        .controller_colors(colors)
        .report_period(report_period)
        .create_profile_with(
            adapter_default_options(pair_timeout),
            &mut store,
            move |config, status, pair_timeout| {
                observe_runtime_config(config, status, pair_timeout, &runtime_events)
            },
        );
    let error = match result {
        Ok(_) => panic!("config observation must stop before runtime open"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::ConnectionFailed);

    assert_eq!(
        lock(&events).as_slice(),
        &[
            CreateEvent::InspectTarget(path.clone()),
            CreateEvent::CreateNew(path.clone()),
            CreateEvent::Open {
                adapter: AdapterSelector::from("usb:fake"),
                controller_kind: ControllerKind::Pro,
                identity: ProfileIdentity::AdapterDefault,
                reporting_kind: ReportingKind::Periodic,
                colors,
                report_period,
                pair_timeout,
            },
        ]
    );
    let persisted = store
        .bytes
        .as_deref()
        .expect("create-new must persist an envelope");
    assert_valid_empty_profile(persisted);
}

#[test]
#[cfg(feature = "bumble")]
fn local_address_profile_is_created_and_typed_before_runtime_open() {
    let path = PathBuf::from("profiles/local-pro.json");
    let address = LocalAddress::parse("02:12:34:56:78:9a").expect("valid local address fixture");
    let pair_timeout = Duration::from_secs(30);
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut store = FakeProfileStore::empty(Arc::clone(&events));
    let runtime_events = Arc::clone(&events);

    let result = ProController::builder("usb:local")
        .profile_path(path.clone())
        .create_profile_with(
            CreateProfileOptions {
                identity: ProfileIdentity::LocalAddress(address),
                pair_timeout,
            },
            &mut store,
            move |config, status, pair_timeout| {
                observe_runtime_config(config, status, pair_timeout, &runtime_events)
            },
        );
    let error = match result {
        Ok(_) => panic!("config observation must stop before runtime open"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::ConnectionFailed);

    assert_eq!(
        lock(&events).as_slice(),
        &[
            CreateEvent::InspectTarget(path.clone()),
            CreateEvent::CreateNew(path.clone()),
            CreateEvent::Open {
                adapter: AdapterSelector::from("usb:local"),
                controller_kind: ControllerKind::Pro,
                identity: ProfileIdentity::LocalAddress(address),
                reporting_kind: ReportingKind::Periodic,
                colors: ControllerColors::default(),
                report_period: Duration::from_millis(8),
                pair_timeout,
            },
        ]
    );
    assert_valid_empty_local_profile(
        store
            .bytes
            .as_deref()
            .expect("create-new must persist the local-address envelope"),
        address,
    );
}

#[test]
fn create_new_race_preserves_competitor_and_stops_before_runtime_open() {
    let path = PathBuf::from("profiles/raced-pro.json");
    let competitor = b"competitor-owned profile bytes".to_vec();
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut store = FakeProfileStore::racing(Arc::clone(&events), competitor.clone());

    let result = ProController::builder("usb:race")
        .profile_path(path.clone())
        .create_profile_with(
            adapter_default_options(Duration::from_secs(60)),
            &mut store,
            runtime_must_not_open,
        );
    let error = match result {
        Ok(_) => panic!("create-new conflict must not return a controller"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::ProfileAlreadyExists);
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<io::Error>())
        .expect("create-new conflict must preserve its I/O source");
    assert_eq!(source.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(store.bytes.as_deref(), Some(competitor.as_slice()));
    assert_eq!(
        lock(&events).as_slice(),
        &[
            CreateEvent::InspectTarget(path.clone()),
            CreateEvent::CreateNew(path),
        ]
    );
}

#[test]
#[cfg(feature = "bumble")]
fn local_address_create_new_race_preserves_competitor_before_runtime_open() {
    let path = PathBuf::from("profiles/raced-local-pro.json");
    let competitor = b"competitor-owned local profile bytes".to_vec();
    let address = LocalAddress::parse("02:12:34:56:78:9a").expect("valid local address fixture");
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut store = FakeProfileStore::racing(Arc::clone(&events), competitor.clone());

    let result = ProController::builder("usb:local-race")
        .profile_path(path.clone())
        .create_profile_with(
            CreateProfileOptions {
                identity: ProfileIdentity::LocalAddress(address),
                pair_timeout: Duration::from_secs(60),
            },
            &mut store,
            runtime_must_not_open,
        );
    let error = match result {
        Ok(_) => panic!("local create-new conflict must not return a controller"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::ProfileAlreadyExists);
    assert_eq!(store.bytes.as_deref(), Some(competitor.as_slice()));
    assert_eq!(
        lock(&events).as_slice(),
        &[
            CreateEvent::InspectTarget(path.clone()),
            CreateEvent::CreateNew(path),
        ]
    );
}

fn observe_runtime_config(
    config: &ControllerConfig<Pro, Periodic>,
    _status: StatusPublisher<Pro>,
    pair_timeout: Duration,
    events: &Arc<Mutex<Vec<CreateEvent>>>,
) -> crate::Result<ControllerRuntime<Pro, Periodic>> {
    let ProfileConfig::Persistent { profile, .. } = &config.profile else {
        panic!("create-profile runtime must receive the generated typed profile");
    };
    assert_eq!(profile.controller_kind(), ControllerKind::Pro);
    lock(events).push(CreateEvent::Open {
        adapter: config.adapter.clone(),
        controller_kind: ControllerKind::Pro,
        identity: profile.identity(),
        reporting_kind: ReportingKind::Periodic,
        colors: config.colors,
        report_period: config.report_period(),
        pair_timeout,
    });
    Err(Error::new(
        ErrorKind::ConnectionFailed,
        "test stopped after observing runtime configuration",
    ))
}

fn runtime_must_not_open(
    _config: &ControllerConfig<Pro, Periodic>,
    _status: StatusPublisher<Pro>,
    _pair_timeout: Duration,
) -> crate::Result<ControllerRuntime<Pro, Periodic>> {
    panic!("runtime must not be requested before profile creation succeeds")
}

fn assert_valid_empty_profile(bytes: &[u8]) {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).expect("created profile must remain valid JSON");
    assert_eq!(value["format"], "swbt.profile");
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["controller_kind"], "pro");
    assert_eq!(value["identity"]["kind"], "adapter-default");
    assert_eq!(value["key_store"]["namespaces"], serde_json::json!({}));
    let document =
        ProfileDocument::parse_json(bytes).expect("created profile must remain valid JSON");
    let profile =
        PairingProfile::<Pro>::try_from(document).expect("created profile must remain Pro-typed");
    assert_eq!(profile.controller_kind(), ControllerKind::Pro);
}

#[cfg(feature = "bumble")]
fn assert_valid_empty_local_profile(bytes: &[u8], address: LocalAddress) {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).expect("created profile must remain valid JSON");
    assert_eq!(value["format"], "swbt.profile");
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["controller_kind"], "pro");
    assert_eq!(value["identity"]["kind"], "exp-local-address");
    assert_eq!(value["identity"]["address"], "02:12:34:56:78:9A");
    assert_eq!(value["key_store"]["namespaces"], serde_json::json!({}));
    let profile = PairingProfile::<Pro>::from_json(bytes)
        .expect("created local-address profile must remain Pro-typed");
    assert_eq!(profile.identity(), ProfileIdentity::LocalAddress(address));
}

const fn adapter_default_options(pair_timeout: Duration) -> CreateProfileOptions {
    CreateProfileOptions {
        identity: ProfileIdentity::AdapterDefault,
        pair_timeout,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
