use std::{
    collections::VecDeque,
    error::Error as _,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crate::{
    AdapterSelector, CreateProfileOptions, LocalAddress, ProfileIdentity,
    diagnostics::LifecycleState,
    error::ErrorKind,
    input::InputState,
    model::Pro,
    profile::{
        ControllerColors, ControllerKind, ProfileCreatePort, ProfileCreateTargetPort,
        ProfileCreateTargetState, ProfileReadPort, Rgb24,
    },
    reporting::{Periodic, ReportingKind},
    runtime::status::StatusPublisher,
};

use super::{
    ProController,
    config::{BuilderConfig, ControllerConfig, ProfileConfig},
    create::{CreateProfileRuntimeBackend, ReadyRuntime},
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
enum CreateSuccessEvent {
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
}

struct FakeProfileStore {
    events: Arc<Mutex<Vec<CreateSuccessEvent>>>,
    bytes: Option<Vec<u8>>,
}

impl FakeProfileStore {
    fn empty(events: Arc<Mutex<Vec<CreateSuccessEvent>>>) -> Self {
        Self {
            events,
            bytes: None,
        }
    }
}

impl ProfileCreateTargetPort for FakeProfileStore {
    fn inspect(&mut self, path: &Path) -> io::Result<ProfileCreateTargetState> {
        lock(&self.events).push(CreateSuccessEvent::InspectTarget(path.to_owned()));
        Ok(if self.bytes.is_some() {
            ProfileCreateTargetState::Existing
        } else {
            ProfileCreateTargetState::Absent
        })
    }
}

impl ProfileReadPort for FakeProfileStore {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        lock(&self.events).push(CreateSuccessEvent::ReadBack(path.to_owned()));
        self.bytes
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "profile is absent"))
    }
}

impl ProfileCreatePort for FakeProfileStore {
    fn create_new(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        lock(&self.events).push(CreateSuccessEvent::CreateNew(path.to_owned()));
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

struct FakeRuntimeBackend {
    events: Arc<Mutex<Vec<CreateSuccessEvent>>>,
    runtime_alive: Arc<AtomicBool>,
}

struct FakeOpenedRuntime {
    status: StatusPublisher<Pro>,
    runtime_alive: Arc<AtomicBool>,
}

impl CreateProfileRuntimeBackend<Pro, Periodic> for FakeRuntimeBackend {
    type Opened = FakeOpenedRuntime;

    fn ensure_supported(&mut self, _config: &BuilderConfig<Pro, Periodic>) -> crate::Result<()> {
        lock(&self.events).push(CreateSuccessEvent::CheckBackendCapability);
        Ok(())
    }

    fn open(
        &mut self,
        config: &ControllerConfig<Pro, Periodic>,
        status: StatusPublisher<Pro>,
    ) -> crate::Result<Self::Opened> {
        let ProfileConfig::Persistent { profile, .. } = &config.profile else {
            panic!("create-profile runtime must receive the reopened profile");
        };
        assert_eq!(profile.controller_kind(), ControllerKind::Pro);
        lock(&self.events).push(CreateSuccessEvent::Open {
            adapter: config.adapter.clone(),
            controller_kind: ControllerKind::Pro,
            reporting_kind: ReportingKind::Periodic,
            colors: config.colors,
            report_period: config.report_period(),
        });
        status.set_lifecycle(LifecycleState::Open);
        Ok(FakeOpenedRuntime {
            status,
            runtime_alive: Arc::clone(&self.runtime_alive),
        })
    }

    fn pair_to_ready(
        &mut self,
        opened: Self::Opened,
        pair_timeout: Duration,
    ) -> crate::Result<ReadyRuntime<Pro, Periodic>> {
        lock(&self.events).push(CreateSuccessEvent::PairStarted(pair_timeout));
        opened
            .status
            .begin_session(LifecycleState::Connecting, &InputState::neutral());
        opened.status.set_connected(true);
        opened.status.set_sender_state(Some(0x30), 1, 2);
        opened.status.record_subcommand(0x30);
        opened.status.set_lifecycle(LifecycleState::Ready);
        lock(&self.events).push(CreateSuccessEvent::ProtocolReady);
        opened.runtime_alive.store(true, Ordering::SeqCst);
        Ok(ReadyRuntime::new(RuntimeDropProbe {
            alive: opened.runtime_alive,
        }))
    }
}

struct RuntimeDropProbe {
    alive: Arc<AtomicBool>,
}

impl Drop for RuntimeDropProbe {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::SeqCst);
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
    let runtime_alive = Arc::new(AtomicBool::new(false));
    let mut store = FakeProfileStore::empty(Arc::clone(&events));
    let mut backend = FakeRuntimeBackend {
        events: Arc::clone(&events),
        runtime_alive: Arc::clone(&runtime_alive),
    };

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
            CreateSuccessEvent::InspectTarget(path.clone()),
            CreateSuccessEvent::CheckBackendCapability,
            CreateSuccessEvent::CreateNew(path.clone()),
            CreateSuccessEvent::ReadBack(path.clone()),
            CreateSuccessEvent::Open {
                adapter: AdapterSelector::from("usb:fake"),
                controller_kind: ControllerKind::Pro,
                reporting_kind: ReportingKind::Periodic,
                colors,
                report_period,
            },
            CreateSuccessEvent::PairStarted(pair_timeout),
            CreateSuccessEvent::ProtocolReady,
        ]
    );
    let persisted = store
        .bytes
        .as_deref()
        .expect("create-new must persist an envelope");
    let value: serde_json::Value =
        serde_json::from_slice(persisted).expect("persisted envelope must be valid JSON");
    assert_eq!(value["format"], "swbt.profile");
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["controller_kind"], "pro");
    assert_eq!(value["identity"]["kind"], "adapter-default");
    assert_eq!(value["key_store"]["namespaces"], serde_json::json!({}));
    let status = controller.status();
    assert_eq!(status.lifecycle, LifecycleState::Ready);
    assert!(status.connected);
    assert_eq!(status.report_mode, Some(0x30));
    assert_eq!(status.input_reports_accepted, 1);
    assert_eq!(status.replies_accepted, 2);
    assert_eq!(status.last_subcommand, Some(0x30));
    assert_eq!(status.worker_failure, None);
    assert_eq!(controller.snapshot(), InputState::neutral());
    assert!(runtime_alive.load(Ordering::SeqCst));

    drop(controller);
    assert!(!runtime_alive.load(Ordering::SeqCst));
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
