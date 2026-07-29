use std::{
    collections::VecDeque,
    error::Error as _,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    AdapterSelector, CreateProfileOptions, LocalAddress, ProfileIdentity,
    diagnostics::LifecycleState,
    error::{Error, ErrorKind},
    input::InputState,
    model::Pro,
    profile::{
        ControllerColors, ControllerKind, PairingProfile, ProfileCreatePort,
        ProfileCreateTargetPort, ProfileCreateTargetState, ProfileDocument, ProfileReadPort, Rgb24,
    },
    reporting::{Periodic, ReportingKind},
    runtime::status::StatusPublisher,
};

use super::{
    ProController,
    config::{BuilderConfig, ControllerConfig, ProfileConfig},
    create::{ControllerRuntime, CreateProfileRuntimeAttempt, CreateProfileRuntimeBackend},
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

#[derive(Clone, Debug, PartialEq, Eq)]
enum CreateEvent {
    InspectTarget(PathBuf),
    CheckBackendCapability,
    CreateNew(PathBuf),
    ReadBack(PathBuf),
    Open {
        adapter: AdapterSelector,
        controller_kind: ControllerKind,
        reporting_kind: ReportingKind,
        colors: ControllerColors,
        report_period: Duration,
    },
    PairStarted(Duration),
    ProtocolReady,
    CleanupWithoutNeutral,
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

impl ProfileReadPort for FakeProfileStore {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        lock(&self.events).push(CreateEvent::ReadBack(path.to_owned()));
        self.bytes
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "profile is absent"))
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

#[derive(Clone, Copy)]
enum RuntimeFailureStage {
    Capability,
    Open,
    Pair,
}

struct FakeRuntimeBackend {
    events: Arc<Mutex<Vec<CreateEvent>>>,
    probe: RuntimeProbe,
    failure: Option<RuntimeFailureStage>,
}

impl FakeRuntimeBackend {
    fn succeeding(events: Arc<Mutex<Vec<CreateEvent>>>, probe: RuntimeProbe) -> Self {
        Self {
            events,
            probe,
            failure: None,
        }
    }

    fn failing(
        events: Arc<Mutex<Vec<CreateEvent>>>,
        probe: RuntimeProbe,
        failure: RuntimeFailureStage,
    ) -> Self {
        Self {
            events,
            probe,
            failure: Some(failure),
        }
    }
}

struct FakeRuntimeAttempt {
    events: Arc<Mutex<Vec<CreateEvent>>>,
    status: StatusPublisher<Pro>,
    probe: RuntimeProbe,
    failure: Option<RuntimeFailureStage>,
    lease: Option<RuntimeLease>,
}

impl CreateProfileRuntimeBackend<Pro, Periodic> for FakeRuntimeBackend {
    type Attempt = FakeRuntimeAttempt;

    fn ensure_supported(&mut self, _config: &BuilderConfig<Pro, Periodic>) -> crate::Result<()> {
        lock(&self.events).push(CreateEvent::CheckBackendCapability);
        if matches!(self.failure, Some(RuntimeFailureStage::Capability)) {
            return Err(crate::runtime::error_map::unsupported_capability(
                "Bluetooth transport",
            ));
        }
        Ok(())
    }

    fn begin_attempt(&mut self, status: StatusPublisher<Pro>) -> Self::Attempt {
        FakeRuntimeAttempt {
            events: Arc::clone(&self.events),
            status,
            probe: self.probe.clone(),
            failure: self.failure,
            lease: None,
        }
    }
}

impl CreateProfileRuntimeAttempt<Pro, Periodic> for FakeRuntimeAttempt {
    fn open(&mut self, config: &ControllerConfig<Pro, Periodic>) -> crate::Result<()> {
        let ProfileConfig::Persistent { profile, .. } = &config.profile else {
            panic!("create-profile runtime must receive the reopened profile");
        };
        assert_eq!(profile.controller_kind(), ControllerKind::Pro);
        lock(&self.events).push(CreateEvent::Open {
            adapter: config.adapter.clone(),
            controller_kind: ControllerKind::Pro,
            reporting_kind: ReportingKind::Periodic,
            colors: config.colors,
            report_period: config.report_period(),
        });
        self.lease = Some(RuntimeLease::acquire(self.probe.clone()));
        if matches!(self.failure, Some(RuntimeFailureStage::Open)) {
            return Err(Error::new(
                ErrorKind::ConnectionFailed,
                "fake runtime open failed",
            ));
        }
        self.status.set_lifecycle(LifecycleState::Open);
        Ok(())
    }

    fn pair_to_ready(&mut self, pair_timeout: Duration) -> crate::Result<()> {
        lock(&self.events).push(CreateEvent::PairStarted(pair_timeout));
        if matches!(self.failure, Some(RuntimeFailureStage::Pair)) {
            return Err(Error::new(
                ErrorKind::ConnectionTimeout,
                "fake runtime pairing timed out",
            ));
        }
        self.status
            .begin_session(LifecycleState::Connecting, &InputState::neutral());
        self.status.set_connected(true);
        self.status.set_sender_state(Some(0x30), 1, 2);
        self.status.record_subcommand(0x30);
        self.status.set_lifecycle(LifecycleState::Ready);
        lock(&self.events).push(CreateEvent::ProtocolReady);
        Ok(())
    }

    fn cleanup_without_neutral(mut self) -> crate::Result<()> {
        lock(&self.events).push(CreateEvent::CleanupWithoutNeutral);
        self.probe
            .explicit_cleanup_count
            .fetch_add(1, Ordering::SeqCst);
        drop(self.lease.take());
        Ok(())
    }

    fn into_ready(mut self) -> ControllerRuntime<Pro, Periodic> {
        ControllerRuntime::new(
            self.lease
                .take()
                .expect("successful runtime attempt must own its lease"),
        )
    }
}

impl Drop for FakeRuntimeAttempt {
    fn drop(&mut self) {
        if let Some(lease) = self.lease.take() {
            self.probe
                .fallback_cleanup_count
                .fetch_add(1, Ordering::SeqCst);
            drop(lease);
        }
    }
}

#[derive(Clone)]
struct RuntimeProbe {
    transport_open: Arc<AtomicBool>,
    worker_alive: Arc<AtomicBool>,
    explicit_cleanup_count: Arc<AtomicUsize>,
    fallback_cleanup_count: Arc<AtomicUsize>,
    resource_drop_count: Arc<AtomicUsize>,
}

impl RuntimeProbe {
    fn new() -> Self {
        Self {
            transport_open: Arc::new(AtomicBool::new(false)),
            worker_alive: Arc::new(AtomicBool::new(false)),
            explicit_cleanup_count: Arc::new(AtomicUsize::new(0)),
            fallback_cleanup_count: Arc::new(AtomicUsize::new(0)),
            resource_drop_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn is_active(&self) -> bool {
        self.transport_open.load(Ordering::SeqCst) || self.worker_alive.load(Ordering::SeqCst)
    }
}

struct RuntimeLease {
    probe: RuntimeProbe,
}

impl RuntimeLease {
    fn acquire(probe: RuntimeProbe) -> Self {
        probe.transport_open.store(true, Ordering::SeqCst);
        probe.worker_alive.store(true, Ordering::SeqCst);
        Self { probe }
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        self.probe.transport_open.store(false, Ordering::SeqCst);
        self.probe.worker_alive.store(false, Ordering::SeqCst);
        self.probe
            .resource_drop_count
            .fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn create_profile_persists_and_reopens_before_open_then_returns_ready_controller() {
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
    let probe = RuntimeProbe::new();
    let mut store = FakeProfileStore::empty(Arc::clone(&events));
    let mut backend = FakeRuntimeBackend::succeeding(Arc::clone(&events), probe.clone());

    let controller = ProController::builder("usb:fake")
        .profile_path(path.clone())
        .controller_colors(colors)
        .report_period(report_period)
        .create_profile_with(
            adapter_default_options(pair_timeout),
            &mut store,
            &mut backend,
        )
        .expect("fake create-profile orchestration must reach Ready");

    assert_eq!(
        lock(&events).as_slice(),
        &[
            CreateEvent::InspectTarget(path.clone()),
            CreateEvent::CheckBackendCapability,
            CreateEvent::CreateNew(path.clone()),
            CreateEvent::ReadBack(path.clone()),
            CreateEvent::Open {
                adapter: AdapterSelector::from("usb:fake"),
                controller_kind: ControllerKind::Pro,
                reporting_kind: ReportingKind::Periodic,
                colors,
                report_period,
            },
            CreateEvent::PairStarted(pair_timeout),
            CreateEvent::ProtocolReady,
        ]
    );
    let persisted = store
        .bytes
        .as_deref()
        .expect("create-new must persist an envelope");
    assert_valid_empty_profile(persisted);
    let status = controller.status();
    assert_eq!(status.lifecycle, LifecycleState::Ready);
    assert!(status.connected);
    assert_eq!(status.report_mode, Some(0x30));
    assert_eq!(status.input_reports_accepted, 1);
    assert_eq!(status.replies_accepted, 2);
    assert_eq!(status.last_subcommand, Some(0x30));
    assert_eq!(status.worker_failure, None);
    assert_eq!(controller.snapshot(), InputState::neutral());
    assert!(probe.is_active());
    assert_eq!(probe.explicit_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.fallback_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.resource_drop_count.load(Ordering::SeqCst), 0);

    drop(controller);
    assert!(!probe.is_active());
    assert_eq!(probe.explicit_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.fallback_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.resource_drop_count.load(Ordering::SeqCst), 1);
}

#[test]
fn backend_capability_failure_stops_before_profile_creation_or_runtime_attempt() {
    let path = PathBuf::from("profiles/backend-unavailable.json");
    let events = Arc::new(Mutex::new(Vec::new()));
    let probe = RuntimeProbe::new();
    let mut store = FakeProfileStore::empty(Arc::clone(&events));
    let mut backend = FakeRuntimeBackend::failing(
        Arc::clone(&events),
        probe.clone(),
        RuntimeFailureStage::Capability,
    );

    let result = ProController::builder("unavailable:fake")
        .profile_path(path.clone())
        .create_profile_with(
            adapter_default_options(Duration::from_secs(60)),
            &mut store,
            &mut backend,
        );
    let error = match result {
        Ok(_) => panic!("unsupported backend must not return a controller"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::UnsupportedCapability);
    assert_eq!(
        lock(&events).as_slice(),
        &[
            CreateEvent::InspectTarget(path),
            CreateEvent::CheckBackendCapability,
        ]
    );
    assert!(store.bytes.is_none());
    assert!(!probe.is_active());
    assert_eq!(probe.explicit_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.fallback_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.resource_drop_count.load(Ordering::SeqCst), 0);
}

#[test]
fn create_new_race_preserves_competitor_and_stops_before_runtime_open() {
    let path = PathBuf::from("profiles/raced-pro.json");
    let competitor = b"competitor-owned profile bytes".to_vec();
    let events = Arc::new(Mutex::new(Vec::new()));
    let probe = RuntimeProbe::new();
    let mut store = FakeProfileStore::racing(Arc::clone(&events), competitor.clone());
    let mut backend = FakeRuntimeBackend::succeeding(Arc::clone(&events), probe.clone());

    let result = ProController::builder("usb:race")
        .profile_path(path.clone())
        .create_profile_with(
            adapter_default_options(Duration::from_secs(60)),
            &mut store,
            &mut backend,
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
            CreateEvent::CheckBackendCapability,
            CreateEvent::CreateNew(path),
        ]
    );
    assert!(!probe.is_active());
    assert_eq!(probe.explicit_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.fallback_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.resource_drop_count.load(Ordering::SeqCst), 0);
}

#[test]
fn open_failure_keeps_valid_empty_profile_and_cleans_partial_runtime() {
    assert_runtime_failure_cleanup(
        RuntimeFailureStage::Open,
        ErrorKind::ConnectionFailed,
        false,
    );
}

#[test]
fn pair_failure_keeps_valid_empty_profile_and_cleans_opened_runtime() {
    assert_runtime_failure_cleanup(
        RuntimeFailureStage::Pair,
        ErrorKind::ConnectionTimeout,
        true,
    );
}

#[test]
fn dropping_an_acquired_attempt_uses_fallback_once_without_explicit_cleanup() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let probe = RuntimeProbe::new();
    let mut backend = FakeRuntimeBackend::succeeding(Arc::clone(&events), probe.clone());
    let (status, _) = crate::runtime::status::status_projection::<Pro>();
    let mut attempt = backend.begin_attempt(status);

    assert!(!probe.is_active());
    attempt.lease = Some(RuntimeLease::acquire(probe.clone()));
    assert!(probe.is_active());
    drop(attempt);

    assert!(!probe.is_active());
    assert!(lock(&events).is_empty());
    assert_eq!(probe.explicit_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.fallback_cleanup_count.load(Ordering::SeqCst), 1);
    assert_eq!(probe.resource_drop_count.load(Ordering::SeqCst), 1);
}

fn assert_runtime_failure_cleanup(
    failure: RuntimeFailureStage,
    expected_kind: ErrorKind,
    expects_pair: bool,
) {
    let path = PathBuf::from("profiles/failing-pro.json");
    let colors = ControllerColors::new(
        Rgb24::new(0x21, 0x22, 0x23),
        Rgb24::new(0x24, 0x25, 0x26),
        Rgb24::new(0x27, 0x28, 0x29),
        Rgb24::new(0x2A, 0x2B, 0x2C),
    );
    let report_period = Duration::from_millis(23);
    let pair_timeout = Duration::from_secs(45);
    let events = Arc::new(Mutex::new(Vec::new()));
    let probe = RuntimeProbe::new();
    let mut store = FakeProfileStore::empty(Arc::clone(&events));
    let mut backend = FakeRuntimeBackend::failing(Arc::clone(&events), probe.clone(), failure);

    let result = ProController::builder("usb:failure")
        .profile_path(path.clone())
        .controller_colors(colors)
        .report_period(report_period)
        .create_profile_with(
            adapter_default_options(pair_timeout),
            &mut store,
            &mut backend,
        );
    let error = match result {
        Ok(_) => panic!("runtime failure must not return a controller"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), expected_kind);
    assert_valid_empty_profile(
        store
            .bytes
            .as_deref()
            .expect("runtime failure must leave the created profile"),
    );
    let mut expected = vec![
        CreateEvent::InspectTarget(path.clone()),
        CreateEvent::CheckBackendCapability,
        CreateEvent::CreateNew(path.clone()),
        CreateEvent::ReadBack(path),
        CreateEvent::Open {
            adapter: AdapterSelector::from("usb:failure"),
            controller_kind: ControllerKind::Pro,
            reporting_kind: ReportingKind::Periodic,
            colors,
            report_period,
        },
    ];
    if expects_pair {
        expected.push(CreateEvent::PairStarted(pair_timeout));
    }
    expected.push(CreateEvent::CleanupWithoutNeutral);
    assert_eq!(lock(&events).as_slice(), expected);
    assert!(!probe.is_active());
    assert_eq!(probe.explicit_cleanup_count.load(Ordering::SeqCst), 1);
    assert_eq!(probe.fallback_cleanup_count.load(Ordering::SeqCst), 0);
    assert_eq!(probe.resource_drop_count.load(Ordering::SeqCst), 1);
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
