use std::{
    error::Error as StdError,
    io,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(feature = "bumble")]
use bumble_transport::Error as BumbleError;

use crate::{
    CreateProfileOptions, DirectJoyConL, DirectJoyConR, DirectProController, Error, ErrorKind,
    JoyConL, JoyConLButton, JoyConR, JoyConRButton, ProButton, ProController, ProfileIdentity,
    controller::Controller,
    diagnostics::LifecycleState,
    input::{Button, InputState},
    model::ControllerModel,
    profile::{
        ControllerKind, ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState,
        ProfileDocument, ProfileReadPort,
    },
    reporting::{Direct, Periodic, ReportingMode},
    runtime::{
        cleanup::{CleanupFailure, CleanupPhase},
        readiness::ReadinessError,
        test_support::{TestTransport, TestTransportControl},
        transport::{
            ActivityNotifier, ClassicAclBufferInfo, HidChannel, SendAcceptance,
            TransportCapabilities, TransportError, TransportErrorKind, TransportEvent,
            TransportPort, TransportResult, UsbTransportMetadata,
        },
        worker::{
            ChannelWorkerWaiter, MonotonicClock, WorkerReporting, WorkerWaitError,
            WorkerWaitRequest, WorkerWaiter,
        },
        worker_thread::WorkerSpawnError,
    },
};

use super::{
    create::CreateProfileRuntimeAttempt,
    runtime::{
        ConcreteRuntimeAttempt, ConcreteRuntimeBackend, PairDriver, RuntimeComponents,
        map_worker_spawn_error, open_controller_runtime,
    },
};

const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
const PAIR_TIMEOUT: Duration = Duration::from_secs(2);
const PERIODIC_READY_AT: Duration = Duration::from_millis(300);
const DEADLOCK_WATCHDOG: Duration = Duration::from_secs(2);
const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

#[cfg(not(feature = "bumble"))]
#[test]
fn unavailable_public_lifecycle_keeps_the_runtime_owner_uninstalled() {
    let mut controller = ProController::builder("unavailable:unit")
        .build()
        .expect("build configured controller");

    assert!(controller._runtime.is_none());
    assert_eq!(
        controller
            .open()
            .expect_err("open must be unavailable")
            .kind(),
        ErrorKind::UnsupportedCapability
    );
    assert!(controller._runtime.is_none());
    assert_eq!(
        controller
            .pair(PAIR_TIMEOUT)
            .expect_err("pair must be unavailable")
            .kind(),
        ErrorKind::UnsupportedCapability
    );
    assert!(controller._runtime.is_none());
    assert_eq!(controller.status().lifecycle, LifecycleState::Configured);
}

#[test]
fn controller_open_is_idempotent_preserves_open_on_unsupported_pair_and_reopens_after_join() {
    let mut controller = ProController::builder("usb:0")
        .build()
        .expect("build configured controller");
    let (first_transport, first_control) = TestTransport::with_limits(8, 3);
    let first_dropped = Arc::new(AtomicBool::new(false));
    let first_drop_observer = Arc::clone(&first_dropped);
    let first_clock = ManualClock::at(Duration::ZERO);
    let first_factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(DropTrackingTransport {
                inner: first_transport,
                dropped: first_drop_observer,
            }),
            first_clock,
            ChannelWorkerWaiter::new(activity_receiver),
            UnusedPairDriver,
        ))
    };

    controller
        .open_with(|config, status| open_controller_runtime(config, status, first_factory))
        .expect("first open starts the worker");
    assert_eq!(controller.status().lifecycle, LifecycleState::Open);
    assert!(!controller.status().connected);
    assert_eq!(first_control.counters(), (1, 0, 0));

    let repeated_open_called = Arc::new(AtomicBool::new(false));
    let repeated_open_observer = Arc::clone(&repeated_open_called);
    controller
        .open_with(move |_, _| {
            repeated_open_observer.store(true, Ordering::SeqCst);
            panic!("repeated open must not construct another runtime")
        })
        .expect("repeated open is idempotent");
    assert!(!repeated_open_called.load(Ordering::SeqCst));
    assert_eq!(first_control.counters(), (1, 0, 0));

    assert_eq!(
        controller
            .pair(PAIR_TIMEOUT)
            .expect_err("M3 pairing remains unsupported")
            .kind(),
        ErrorKind::UnsupportedCapability
    );
    assert_eq!(controller.status().lifecycle, LifecycleState::Open);
    assert_eq!(first_control.counters(), (1, 0, 0));

    controller.close().expect("first close joins the worker");
    assert!(first_dropped.load(Ordering::SeqCst));
    assert_eq!(first_control.counters(), (1, 1, 1));
    assert_eq!(controller.status().lifecycle, LifecycleState::Closed);
    controller.close().expect("repeated close is idempotent");
    assert_eq!(first_control.counters(), (1, 1, 1));

    let (second_transport, second_control) = TestTransport::with_limits(8, 3);
    let second_dropped = Arc::new(AtomicBool::new(false));
    let second_drop_observer = Arc::clone(&second_dropped);
    let second_clock = ManualClock::at(Duration::ZERO);
    let second_factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(DropTrackingTransport {
                inner: second_transport,
                dropped: second_drop_observer,
            }),
            second_clock,
            ChannelWorkerWaiter::new(activity_receiver),
            UnusedPairDriver,
        ))
    };

    controller
        .open_with(|config, status| open_controller_runtime(config, status, second_factory))
        .expect("closed controller can reopen");
    assert_eq!(controller.status().lifecycle, LifecycleState::Open);
    assert!(!controller.status().connected);
    assert_eq!(second_control.counters(), (1, 0, 0));

    controller
        .close_without_neutral()
        .expect("reopened worker joins without neutral");
    assert!(second_dropped.load(Ordering::SeqCst));
    assert_eq!(second_control.counters(), (1, 1, 1));
    assert_eq!(controller.status().lifecycle, LifecycleState::Closed);
}

#[test]
fn worker_spawn_error_maps_cleanup_as_related_without_leaking_details() {
    let spawn_error = WorkerSpawnError::new(
        io::Error::other("secret spawn backend detail"),
        Some(CleanupFailure::new(
            CleanupPhase::DrainInterrupt,
            TransportError::with_source(
                TransportErrorKind::SourceTerminated,
                Arc::new(io::Error::other("secret spawn cleanup detail")),
            ),
        )),
    );
    assert_eq!(
        spawn_error.to_string(),
        "controller worker thread could not be spawned"
    );
    assert!(!format!("{spawn_error:?}").contains("secret"));

    let error = map_worker_spawn_error(spawn_error);

    assert_eq!(error.kind(), ErrorKind::WorkerFailed);
    assert_eq!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .expect("typed worker spawn source")
            .to_string(),
        "secret spawn backend detail"
    );
    let cleanup = error.related_error().expect("related spawn cleanup error");
    assert_eq!(cleanup.kind(), ErrorKind::WorkerFailed);
    let cleanup_failure = cleanup
        .source()
        .and_then(|source| source.downcast_ref::<CleanupFailure>())
        .expect("typed spawn cleanup failure");
    assert_eq!(cleanup_failure.phase(), CleanupPhase::DrainInterrupt);
    let transport_error = cleanup_failure
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("typed spawn cleanup transport source");
    assert_eq!(transport_error.kind(), TransportErrorKind::SourceTerminated);
    assert_eq!(
        transport_error
            .source()
            .expect("spawn cleanup backend source")
            .to_string(),
        "secret spawn cleanup detail"
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));
    assert!(!cleanup.to_string().contains("secret"));
    assert!(!format!("{cleanup:?}").contains("secret"));
}

#[test]
fn pro_periodic_build_ready_input_and_close_smoke() {
    periodic_smoke(
        ProController::builder("fake-adapter")
            .build()
            .expect("build configured Pro controller"),
        ProButton::A,
    );
}

#[test]
fn pro_direct_build_ready_input_and_close_smoke() {
    direct_smoke(
        DirectProController::builder("fake-adapter")
            .build()
            .expect("build configured direct Pro controller"),
        ProButton::A,
    );
}

#[test]
fn joycon_l_periodic_build_ready_input_and_close_smoke() {
    periodic_smoke(
        JoyConL::builder("fake-adapter")
            .build()
            .expect("build configured left Joy-Con"),
        JoyConLButton::L,
    );
}

#[test]
fn joycon_l_direct_build_ready_input_and_close_smoke() {
    direct_smoke(
        DirectJoyConL::builder("fake-adapter")
            .build()
            .expect("build configured direct left Joy-Con"),
        JoyConLButton::L,
    );
}

#[test]
fn joycon_r_periodic_build_ready_input_and_close_smoke() {
    periodic_smoke(
        JoyConR::builder("fake-adapter")
            .build()
            .expect("build configured right Joy-Con"),
        JoyConRButton::R,
    );
}

#[test]
fn joycon_r_direct_build_ready_input_and_close_smoke() {
    direct_smoke(
        DirectJoyConR::builder("fake-adapter")
            .build()
            .expect("build configured direct right Joy-Con"),
        JoyConRButton::R,
    );
}

#[test]
fn pair_primary_and_concrete_cleanup_failure_remain_separately_traversable() {
    let (transport, control) = TestTransport::with_limits(8, 3);
    let observed_control = control.clone();
    let clock = ManualClock::at(Duration::ZERO);
    let factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(DrainFailingTransport { inner: transport }),
            clock,
            ChannelWorkerWaiter::new(activity_receiver),
            DisconnectingPairDriver { control },
        ))
    };
    let mut backend = ConcreteRuntimeBackend::new(factory);
    let mut store = MemoryProfileStore::default();

    let error = match ProController::builder("fake-adapter")
        .profile_path("profiles/concrete-cleanup.json")
        .create_profile_with(
            CreateProfileOptions {
                identity: ProfileIdentity::AdapterDefault,
                pair_timeout: PAIR_TIMEOUT,
            },
            &mut store,
            &mut backend,
        ) {
        Ok(_) => panic!("disconnect before Ready and cleanup failure must fail"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::ConnectionFailed);
    assert!(matches!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<ReadinessError>()),
        Some(ReadinessError::Disconnected { reason: Some(0x13) })
    ));
    let cleanup = error.related_error().expect("related cleanup error");
    assert_eq!(cleanup.kind(), ErrorKind::WorkerFailed);
    let cleanup_failure = cleanup
        .source()
        .and_then(|source| source.downcast_ref::<CleanupFailure>())
        .expect("typed cleanup failure");
    assert_eq!(cleanup_failure.phase(), CleanupPhase::DrainInterrupt);
    let transport_error = cleanup_failure
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("cleanup transport source");
    assert_eq!(transport_error.kind(), TransportErrorKind::SourceTerminated);
    assert_eq!(
        transport_error
            .source()
            .expect("backend cleanup source")
            .to_string(),
        "secret cleanup backend detail"
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));
    assert!(!cleanup.to_string().contains("secret"));
    assert!(!format!("{cleanup:?}").contains("secret"));

    let persisted = store.bytes.expect("empty profile remains after failure");
    let document = ProfileDocument::parse_json(&persisted).expect("remaining profile is valid");
    assert_eq!(document.controller_kind(), ControllerKind::Pro);
    assert_eq!(observed_control.counters(), (1, 1, 1));
}

#[test]
fn terminal_pair_worker_failure_cleans_and_joins_with_typed_primary() {
    let (transport, control) = TestTransport::with_limits(8, 3);
    let fail_poll = Arc::new(AtomicBool::new(false));
    let transport_fail_poll = Arc::clone(&fail_poll);
    let cleanup_trace = Arc::new(Mutex::new(Vec::new()));
    let transport_cleanup_trace = Arc::clone(&cleanup_trace);
    let clock = ManualClock::at(Duration::ZERO);
    let factory = move |activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(TerminalPairTransport {
                inner: transport,
                fail_poll: transport_fail_poll,
                cleanup_trace: transport_cleanup_trace,
            }),
            clock,
            ChannelWorkerWaiter::new(activity_receiver),
            TerminalPairDriver {
                fail_poll,
                activity,
            },
        ))
    };
    let mut backend = ConcreteRuntimeBackend::new(factory);
    let mut store = MemoryProfileStore::default();

    let error = match ProController::builder("fake-adapter")
        .profile_path("profiles/terminal-pair.json")
        .create_profile_with(
            CreateProfileOptions {
                identity: ProfileIdentity::AdapterDefault,
                pair_timeout: PAIR_TIMEOUT,
            },
            &mut store,
            &mut backend,
        ) {
        Ok(_) => panic!("terminal pair worker failure must not return a controller"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::WorkerFailed);
    let primary = error
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("terminal worker transport error is the primary source");
    assert_eq!(primary.kind(), TransportErrorKind::SourceTerminated);
    assert_eq!(
        primary
            .source()
            .expect("terminal pair backend source")
            .to_string(),
        "secret terminal pair backend detail"
    );
    let cleanup = error
        .related_error()
        .expect("terminal cleanup failure is related");
    let cleanup_failure = cleanup
        .source()
        .and_then(|source| source.downcast_ref::<CleanupFailure>())
        .expect("typed terminal cleanup failure");
    assert_eq!(cleanup_failure.phase(), CleanupPhase::DrainInterrupt);
    let cleanup_transport = cleanup_failure
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("typed terminal cleanup transport source");
    assert_eq!(
        cleanup_transport
            .source()
            .expect("terminal cleanup backend source")
            .to_string(),
        "secret terminal cleanup detail"
    );
    assert!(
        cleanup.related_error().is_none(),
        "joined terminal cause must not be repeated after cleanup"
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));
    assert!(!cleanup.to_string().contains("secret"));
    assert!(!format!("{cleanup:?}").contains("secret"));
    assert_eq!(
        *lock(&cleanup_trace),
        [
            TerminalPairTrace::PollFailure,
            TerminalPairTrace::Drain(Duration::from_secs(1)),
            TerminalPairTrace::Disconnect,
            TerminalPairTrace::Close,
            TerminalPairTrace::TransportDrop,
        ],
        "terminal cleanup precedes worker completion and joined teardown"
    );
    assert_eq!(
        control.counters(),
        (1, 1, 1),
        "terminal cleanup disconnects and closes exactly once"
    );
    let persisted = store
        .bytes
        .expect("terminal pairing failure keeps the valid empty profile");
    let document =
        ProfileDocument::parse_json(&persisted).expect("remaining terminal profile is valid");
    assert_eq!(document.controller_kind(), ControllerKind::Pro);
}

#[test]
fn pair_after_worker_finishes_before_enqueue_uses_actual_terminal_outcome() {
    let controller = ProController::builder("fake-adapter")
        .build()
        .expect("build configured Pro controller");
    let (transport, control) = TestTransport::with_limits(8, 3);
    let cleanup_trace = Arc::new(Mutex::new(Vec::new()));
    let transport_cleanup_trace = Arc::clone(&cleanup_trace);
    let clock = ManualClock::at(Duration::ZERO);
    let factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(TerminalPairTransport {
                inner: transport,
                fail_poll: Arc::new(AtomicBool::new(true)),
                cleanup_trace: transport_cleanup_trace,
            }),
            clock,
            ChannelWorkerWaiter::new(activity_receiver),
            UnusedPairDriver,
        ))
    };
    let mut attempt = ConcreteRuntimeAttempt::new(factory, controller.status_publisher.clone());
    attempt
        .open(&controller.config)
        .expect("open concrete runtime attempt");

    let watchdog = Instant::now() + DEADLOCK_WATCHDOG;
    while !attempt.worker_is_finished() {
        assert!(
            Instant::now() < watchdog,
            "worker must terminate before the enqueue-race assertion"
        );
        thread::yield_now();
    }

    let error = attempt
        .pair_to_ready(PAIR_TIMEOUT)
        .expect_err("pairing after worker termination must return its actual outcome");

    assert_eq!(error.kind(), ErrorKind::WorkerFailed);
    let primary = error
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("terminal transport failure must remain the primary source");
    assert_eq!(primary.kind(), TransportErrorKind::SourceTerminated);
    assert_eq!(
        primary
            .source()
            .expect("terminal backend source")
            .to_string(),
        "secret terminal pair backend detail"
    );
    let cleanup = error
        .related_error()
        .expect("terminal cleanup failure remains related");
    let cleanup_failure = cleanup
        .source()
        .and_then(|source| source.downcast_ref::<CleanupFailure>())
        .expect("typed terminal cleanup failure");
    assert_eq!(cleanup_failure.phase(), CleanupPhase::DrainInterrupt);
    assert!(
        cleanup.related_error().is_none(),
        "terminal cause must not be duplicated after cleanup"
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));
    assert_eq!(
        *lock(&cleanup_trace),
        [
            TerminalPairTrace::PollFailure,
            TerminalPairTrace::Drain(Duration::from_secs(1)),
            TerminalPairTrace::Disconnect,
            TerminalPairTrace::Close,
            TerminalPairTrace::TransportDrop,
        ]
    );
    assert_eq!(control.counters(), (1, 1, 1));

    attempt
        .cleanup_without_neutral()
        .expect("terminal pair handling consumes the worker owner");
    assert_eq!(
        control.counters(),
        (1, 1, 1),
        "fallback cleanup must not repeat terminal cleanup"
    );
}

#[test]
fn non_classic_capabilities_fail_before_worker_spawn_and_clean_up_transport() {
    let controller = ProController::builder("fake-adapter")
        .build()
        .expect("build configured Pro controller");
    let capabilities = TransportCapabilities::from_initialized_controller(
        DEVICE_INFO_ADDRESS,
        None,
        Some([0, 0, 0, 0, 0x20, 0, 0, 0]),
        Some(ClassicAclBufferInfo::new(1021, 8)),
        UsbTransportMetadata::new(0x0a12, 0x0001, 1, 7),
    )
    .expect("non-Classic controller still has valid identity metadata");
    let (transport, control) = TestTransport::with_capabilities(8, 3, capabilities);
    let clock = ManualClock::at(Duration::ZERO);
    let factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(transport),
            clock,
            ChannelWorkerWaiter::new(activity_receiver),
            UnusedPairDriver,
        ))
    };
    let mut attempt = ConcreteRuntimeAttempt::new(factory, controller.status_publisher.clone());

    let error = attempt
        .open(&controller.config)
        .expect_err("non-Classic capability must fail during transport open");
    assert_eq!(error.kind(), ErrorKind::TransportOpen);
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("typed unsupported-controller source");
    assert_eq!(source.kind(), TransportErrorKind::UnsupportedController);
    assert!(
        !attempt.owns_worker(),
        "worker must not start before validation"
    );
    assert_eq!(control.counters(), (1, 0, 0));

    attempt
        .cleanup_without_neutral()
        .expect("opened unsupported transport is cleaned up");
    assert_eq!(control.counters(), (1, 1, 1));
}

#[cfg(feature = "bumble")]
#[test]
fn bumble_usb_open_failures_map_to_sanitized_public_transport_errors() {
    let cases = [
        (
            "not-found",
            BumbleError::InvalidSpec("secret USB selector was not found".into()),
        ),
        ("permission", BumbleError::Usb(rusb::Error::Access)),
        ("driver", BumbleError::Usb(rusb::Error::NotSupported)),
        ("claim", BumbleError::Usb(rusb::Error::Busy)),
    ];

    for (stage, backend_error) in cases {
        let controller = ProController::builder("usb:0")
            .build()
            .expect("build configured Pro controller");
        let clock = ManualClock::at(Duration::ZERO);
        let factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
            Ok::<_, Error>(RuntimeComponents::new(
                Box::new(BumbleOpenErrorTransport {
                    source: Some(backend_error),
                }),
                clock,
                ChannelWorkerWaiter::new(activity_receiver),
                UnusedPairDriver,
            ))
        };

        let error = match open_controller_runtime(
            &controller.config,
            controller.status_publisher.clone(),
            factory,
        ) {
            Ok(_) => panic!("{stage} failure must stop controller open"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), ErrorKind::TransportOpen, "{stage}");
        let transport = error
            .source()
            .and_then(|source| source.downcast_ref::<TransportError>())
            .expect("public error retains the typed transport source");
        assert_eq!(transport.kind(), TransportErrorKind::OpenFailed, "{stage}");
        let bumble = transport
            .source()
            .and_then(|source| source.downcast_ref::<BumbleError>())
            .expect("transport error retains the typed Bumble source");
        assert!(
            matches!(
                (stage, bumble),
                ("not-found", BumbleError::InvalidSpec(_))
                    | ("permission", BumbleError::Usb(rusb::Error::Access))
                    | ("driver", BumbleError::Usb(rusb::Error::NotSupported))
                    | ("claim", BumbleError::Usb(rusb::Error::Busy))
            ),
            "unexpected source for {stage}: {bumble}"
        );
        assert!(!error.to_string().contains("secret"), "{stage}");
        assert!(!format!("{error:?}").contains("secret"), "{stage}");
        assert!(!transport.to_string().contains("secret"), "{stage}");
        assert!(!format!("{transport:?}").contains("secret"), "{stage}");
    }
}

#[cfg(feature = "bumble")]
#[test]
fn invalid_usb_selector_maps_through_public_open_without_disclosure() {
    let mut controller = ProController::builder("usb:0a12:0001/secret-serial[metadata]")
        .build()
        .expect("build configured Pro controller");

    let error = controller
        .open()
        .expect_err("invalid selector must stop before USB access");

    assert_eq!(error.kind(), ErrorKind::TransportOpen);
    let transport = error
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("public error retains the typed transport source");
    assert_eq!(transport.kind(), TransportErrorKind::OpenFailed);
    assert!(!error.to_string().contains("secret-serial"));
    assert!(!format!("{error:?}").contains("secret-serial"));
    assert!(!transport.to_string().contains("secret-serial"));
    assert!(!format!("{transport:?}").contains("secret-serial"));
    assert!(controller._runtime.is_none());
}

#[test]
fn partial_transport_open_failure_is_cleaned_before_attempt_drop() {
    let (transport, control) = TestTransport::with_limits(8, 3);
    let observed_control = control.clone();
    let cleanup_trace = Arc::new(Mutex::new(Vec::new()));
    let transport_cleanup_trace = Arc::clone(&cleanup_trace);
    let clock = ManualClock::at(Duration::ZERO);
    let factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(PartiallyOpeningTransport {
                inner: transport,
                cleanup_trace: transport_cleanup_trace,
            }),
            clock,
            ChannelWorkerWaiter::new(activity_receiver),
            UnusedPairDriver,
        ))
    };
    let mut backend = ConcreteRuntimeBackend::new(factory);
    let mut store = MemoryProfileStore::default();

    let error = match ProController::builder("fake-adapter")
        .profile_path("profiles/partial-open.json")
        .create_profile_with(
            CreateProfileOptions {
                identity: ProfileIdentity::AdapterDefault,
                pair_timeout: PAIR_TIMEOUT,
            },
            &mut store,
            &mut backend,
        ) {
        Ok(_) => panic!("partially opened transport must report its open error"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), ErrorKind::TransportOpen);
    let source = error
        .source()
        .and_then(|source| source.downcast_ref::<TransportError>())
        .expect("typed transport open source");
    assert_eq!(source.kind(), TransportErrorKind::OpenFailed);
    assert_eq!(
        source.source().expect("backend open source").to_string(),
        "secret partial open detail"
    );
    assert!(
        error.related_error().is_none(),
        "successful cleanup must not fabricate a related failure"
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));

    let persisted = store
        .bytes
        .expect("empty profile remains after open failure");
    let document = ProfileDocument::parse_json(&persisted).expect("remaining profile is valid");
    assert_eq!(document.controller_kind(), ControllerKind::Pro);
    assert_eq!(
        observed_control.counters(),
        (1, 1, 1),
        "explicit fallback drains, disconnects, and closes the partial open once"
    );
    assert_eq!(
        *lock(&cleanup_trace),
        [
            PartialCleanupTrace::Drain(Duration::from_secs(1)),
            PartialCleanupTrace::Disconnect,
            PartialCleanupTrace::Close,
        ],
        "explicit fallback keeps bounded drain, disconnect, and close order"
    );
}

#[test]
fn partial_transport_open_attempt_drop_skips_drain_and_cleans_once() {
    let controller = ProController::builder("fake-adapter")
        .build()
        .expect("build configured Pro controller");
    let (transport, control) = TestTransport::with_limits(8, 3);
    let cleanup_trace = Arc::new(Mutex::new(Vec::new()));
    let transport_cleanup_trace = Arc::clone(&cleanup_trace);
    let clock = ManualClock::at(Duration::ZERO);
    let factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(PartiallyOpeningTransport {
                inner: transport,
                cleanup_trace: transport_cleanup_trace,
            }),
            clock,
            ChannelWorkerWaiter::new(activity_receiver),
            UnusedPairDriver,
        ))
    };
    let mut attempt = ConcreteRuntimeAttempt::new(factory, controller.status_publisher.clone());

    let error = attempt
        .open(&controller.config)
        .expect_err("partially opened transport reports its open error");
    assert_eq!(error.kind(), ErrorKind::TransportOpen);
    assert_eq!(control.counters(), (1, 0, 0));

    drop(attempt);

    assert_eq!(
        *lock(&cleanup_trace),
        [PartialCleanupTrace::Disconnect, PartialCleanupTrace::Close],
        "Drop cleanup skips drain and preserves disconnect-before-close order"
    );
    assert_eq!(
        control.counters(),
        (1, 1, 1),
        "Drop disconnects and closes the partial open exactly once"
    );
}

#[test]
fn partial_transport_open_panic_is_guarded_for_drop_cleanup() {
    let controller = ProController::builder("fake-adapter")
        .build()
        .expect("build configured Pro controller");
    let (transport, control) = TestTransport::with_limits(8, 3);
    let cleanup_trace = Arc::new(Mutex::new(Vec::new()));
    let transport_cleanup_trace = Arc::clone(&cleanup_trace);
    let clock = ManualClock::at(Duration::ZERO);
    let factory = move |_activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(PanickingOpenTransport {
                inner: transport,
                cleanup_trace: transport_cleanup_trace,
            }),
            clock,
            ChannelWorkerWaiter::new(activity_receiver),
            UnusedPairDriver,
        ))
    };
    let mut attempt = ConcreteRuntimeAttempt::new(factory, controller.status_publisher.clone());

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let _ = attempt.open(&controller.config);
    }));

    assert!(panicked.is_err(), "transport open panic must propagate");
    assert_eq!(control.counters(), (1, 0, 0));
    drop(attempt);
    assert_eq!(
        *lock(&cleanup_trace),
        [PartialCleanupTrace::Disconnect, PartialCleanupTrace::Close],
        "attempt Drop skips drain and cleans the guarded partial open"
    );
    assert_eq!(control.counters(), (1, 1, 1));
}

#[test]
fn common_input_bridge_dispatches_for_periodic_and_direct_workers() {
    let periodic = ProController::builder("fake-adapter")
        .build()
        .expect("build configured Pro controller");
    let (mut periodic, periodic_control) = install_fake_runtime(periodic, true);
    periodic
        .press([ProButton::A])
        .expect("Periodic common press reaches worker");
    assert_eq!(
        periodic.snapshot(),
        InputState::neutral().with_buttons([ProButton::A])
    );
    periodic
        .release([ProButton::A])
        .expect("Periodic common release reaches worker");
    assert_eq!(periodic.snapshot(), InputState::neutral());
    periodic
        .tap([ProButton::A], Duration::ZERO)
        .expect("Periodic zero-duration tap reaches worker");
    assert_eq!(periodic.snapshot(), InputState::neutral());
    periodic
        .neutral()
        .expect("Periodic common neutral reaches worker");
    assert_eq!(periodic.snapshot(), InputState::neutral());
    periodic.close().expect("close Periodic worker");
    assert_closed(&periodic, &periodic_control);

    let direct = DirectProController::builder("fake-adapter")
        .build()
        .expect("build configured direct Pro controller");
    let (mut direct, direct_control) = install_fake_runtime(direct, false);
    direct
        .press([ProButton::A])
        .expect("Direct common press reaches worker");
    assert_eq!(
        direct.snapshot(),
        InputState::neutral().with_buttons([ProButton::A])
    );
    direct
        .release([ProButton::A])
        .expect("Direct common release reaches worker");
    assert_eq!(direct.snapshot(), InputState::neutral());
    direct
        .tap([ProButton::A], Duration::ZERO)
        .expect("Direct zero-duration tap reaches worker");
    assert_eq!(direct.snapshot(), InputState::neutral());
    direct
        .neutral()
        .expect("Direct common neutral reaches worker");
    assert_eq!(direct.snapshot(), InputState::neutral());
    direct
        .close_without_neutral()
        .expect("close Direct worker without neutral");
    assert_closed(&direct, &direct_control);
}

#[test]
fn initialized_address_feeds_device_info_reply_without_reversal() {
    let local_address = [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc];
    let capabilities = TransportCapabilities::from_initialized_controller(
        local_address,
        None,
        Some([0; 8]),
        Some(ClassicAclBufferInfo::new(1021, 8)),
        UsbTransportMetadata::new(0x0a12, 0x0001, 1, 7),
    )
    .expect("valid custom controller capabilities");
    let controller = ProController::builder("fake-adapter")
        .build()
        .expect("build configured Pro controller");
    let (mut controller, control) =
        install_fake_runtime_with_capabilities(controller, true, capabilities, true);

    let device_info = control
        .accepted_interrupts()
        .into_iter()
        .find(|report| report.get(14) == Some(&0x02))
        .expect("worker emits the device-info reply");
    assert_eq!(&device_info[19..25], local_address);

    controller.close().expect("close custom-address worker");
    assert_closed(&controller, &control);
}

#[test]
fn configured_input_is_transport_closed_and_close_is_idempotent() {
    let mut periodic = ProController::builder("fake-adapter")
        .build()
        .expect("build configured Pro controller");
    assert_eq!(periodic.report_period(), Duration::from_millis(8));
    let input_error = periodic
        .apply(InputState::neutral().with_buttons([ProButton::A]))
        .expect_err("Configured controller has no open runtime");
    assert_eq!(input_error.kind(), ErrorKind::TransportClosed);
    periodic.close().expect("Configured close is successful");
    periodic.close().expect("repeated close is successful");
    assert_eq!(periodic.status().lifecycle, LifecycleState::Closed);

    let mut direct = DirectProController::builder("fake-adapter")
        .build()
        .expect("build configured direct Pro controller");
    let input_error = direct
        .send(InputState::neutral().with_buttons([ProButton::A]))
        .expect_err("Configured direct controller has no open runtime");
    assert_eq!(input_error.kind(), ErrorKind::TransportClosed);
    direct
        .close_without_neutral()
        .expect("Configured close without neutral is successful");
    direct
        .close_without_neutral()
        .expect("repeated close without neutral is successful");
    assert_eq!(direct.status().lifecycle, LifecycleState::Closed);
}

fn periodic_smoke<M>(controller: Controller<M, Periodic>, button: Button<M>)
where
    M: ControllerModel,
    Periodic: WorkerReporting<M>,
{
    let (mut controller, control) = install_fake_runtime(controller, true);
    let state = InputState::neutral().with_buttons([button]);

    controller
        .apply(state.clone())
        .expect("Periodic worker accepts a model-valid state");

    assert_eq!(controller.snapshot(), state);
    assert_eq!(controller.status().lifecycle, LifecycleState::Ready);
    controller.close().expect("explicit close joins worker");
    assert_closed(&controller, &control);
}

fn direct_smoke<M>(controller: Controller<M, Direct>, button: Button<M>)
where
    M: ControllerModel,
    Direct: WorkerReporting<M>,
{
    let (mut controller, control) = install_fake_runtime(controller, false);
    let state = InputState::neutral().with_buttons([button]);

    controller
        .send(state.clone())
        .expect("Direct worker accepts a model-valid state");

    assert_eq!(controller.snapshot(), state);
    assert_eq!(controller.status().lifecycle, LifecycleState::Ready);
    controller.close().expect("explicit close joins worker");
    assert_closed(&controller, &control);
}

fn install_fake_runtime<M, R>(
    controller: Controller<M, R>,
    periodic: bool,
) -> (Controller<M, R>, TestTransportControl)
where
    M: ControllerModel,
    R: ReportingMode + WorkerReporting<M>,
{
    install_fake_runtime_with_capabilities(
        controller,
        periodic,
        TransportCapabilities::test_default(),
        false,
    )
}

fn install_fake_runtime_with_capabilities<M, R>(
    mut controller: Controller<M, R>,
    periodic: bool,
    capabilities: TransportCapabilities,
    request_device_info: bool,
) -> (Controller<M, R>, TestTransportControl)
where
    M: ControllerModel,
    R: ReportingMode + WorkerReporting<M>,
{
    assert_eq!(controller.status().lifecycle, LifecycleState::Configured);
    assert_eq!(controller.snapshot(), InputState::neutral());

    let (transport, control) = TestTransport::with_capabilities(16, 3, capabilities);
    let observed_control = control.clone();
    let clock = ManualClock::at(Duration::ZERO);
    let worker_clock = clock.clone();
    let factory = move |activity: ActivityNotifier, activity_receiver: Receiver<()>| {
        let (requests, observed_requests) = sync_channel(16);
        let waiter = ObservedWaiter {
            inner: ChannelWorkerWaiter::new(activity_receiver),
            requests,
        };
        let driver = FakePairDriver {
            control,
            clock,
            activity,
            observed_requests,
            periodic,
            request_device_info,
        };
        Ok::<_, Error>(RuntimeComponents::new(
            Box::new(transport),
            worker_clock,
            waiter,
            driver,
        ))
    };
    let mut attempt = ConcreteRuntimeAttempt::new(factory, controller.status_publisher.clone());

    attempt
        .open(&controller.config)
        .expect("open fake concrete runtime");
    attempt
        .pair_to_ready(PAIR_TIMEOUT)
        .expect("pair through worker one-shot");
    controller._runtime = Some(attempt.into_ready());

    let status = controller.status();
    assert_eq!(status.lifecycle, LifecycleState::Ready);
    assert!(status.connected);
    assert_eq!(status.report_mode, Some(0x30));
    assert_eq!(status.last_subcommand, Some(0x30));
    assert_eq!(status.input_reports_accepted, 1);
    assert_eq!(
        status.replies_accepted,
        if request_device_info { 3 } else { 2 }
    );
    assert_eq!(observed_control.counters(), (1, 0, 0));
    (controller, observed_control)
}

fn assert_closed<M, R>(controller: &Controller<M, R>, control: &TestTransportControl)
where
    M: ControllerModel,
    R: ReportingMode,
{
    let status = controller.status();
    assert_eq!(status.lifecycle, LifecycleState::Closed);
    assert!(!status.connected);
    assert_eq!(control.counters(), (1, 1, 1));
}

struct FakePairDriver {
    control: TestTransportControl,
    clock: ManualClock,
    activity: ActivityNotifier,
    observed_requests: Receiver<WorkerWaitRequest>,
    periodic: bool,
    request_device_info: bool,
}

struct DisconnectingPairDriver {
    control: TestTransportControl,
}

struct UnusedPairDriver;

impl PairDriver for UnusedPairDriver {
    fn after_pair_enqueued(&mut self) -> crate::Result<()> {
        panic!("pairing must not start after transport open failure")
    }
}

impl PairDriver for DisconnectingPairDriver {
    fn after_pair_enqueued(&mut self) -> crate::Result<()> {
        self.control
            .inject_disconnected(Some(0x13))
            .map_err(pair_driver_error)
    }
}

impl PairDriver for FakePairDriver {
    fn after_pair_enqueued(&mut self) -> crate::Result<()> {
        self.control.inject_connected().map_err(pair_driver_error)?;
        self.control
            .inject_hid_channel_opened(HidChannel::Control)
            .map_err(pair_driver_error)?;
        self.control
            .inject_hid_channel_opened(HidChannel::Interrupt)
            .map_err(pair_driver_error)?;
        if self.request_device_info {
            self.control
                .inject_hid_output(HidChannel::Control, &subcommand_report(0x02, &[]))
                .map_err(pair_driver_error)?;
        }
        self.control
            .inject_hid_output(HidChannel::Control, &subcommand_report(0x03, &[0x30]))
            .map_err(pair_driver_error)?;
        self.control
            .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x30, &[0x01]))
            .map_err(pair_driver_error)?;

        if self.periodic {
            loop {
                let request = self
                    .observed_requests
                    .recv_timeout(DEADLOCK_WATCHDOG)
                    .map_err(|source| {
                        Error::with_source(
                            crate::ErrorKind::WorkerFailed,
                            "worker did not publish the Periodic readiness deadline",
                            source,
                        )
                    })?;
                if request == WorkerWaitRequest::ActivityOrDeadline(PERIODIC_READY_AT) {
                    self.clock.set(PERIODIC_READY_AT);
                    self.activity.notify();
                    break;
                }
            }
        }
        Ok(())
    }
}

fn pair_driver_error(source: crate::runtime::transport::TransportError) -> Error {
    Error::with_source(
        crate::ErrorKind::ConnectionFailed,
        "fake pairing event could not be injected",
        source,
    )
}

#[derive(Clone)]
struct ManualClock {
    now: Arc<Mutex<Duration>>,
}

impl ManualClock {
    fn at(now: Duration) -> Self {
        Self {
            now: Arc::new(Mutex::new(now)),
        }
    }

    fn set(&self, now: Duration) {
        let mut current = lock(&self.now);
        assert!(now >= *current, "manual clock cannot move backwards");
        *current = now;
    }
}

impl MonotonicClock for ManualClock {
    fn now(&self) -> Duration {
        *lock(&self.now)
    }
}

struct ObservedWaiter {
    inner: ChannelWorkerWaiter,
    requests: SyncSender<WorkerWaitRequest>,
}

struct DrainFailingTransport {
    inner: TestTransport,
}

struct TerminalPairTransport {
    inner: TestTransport,
    fail_poll: Arc<AtomicBool>,
    cleanup_trace: Arc<Mutex<Vec<TerminalPairTrace>>>,
}

struct TerminalPairDriver {
    fail_poll: Arc<AtomicBool>,
    activity: ActivityNotifier,
}

struct PartiallyOpeningTransport {
    inner: TestTransport,
    cleanup_trace: Arc<Mutex<Vec<PartialCleanupTrace>>>,
}

struct PanickingOpenTransport {
    inner: TestTransport,
    cleanup_trace: Arc<Mutex<Vec<PartialCleanupTrace>>>,
}

struct DropTrackingTransport {
    inner: TestTransport,
    dropped: Arc<AtomicBool>,
}

#[cfg(feature = "bumble")]
struct BumbleOpenErrorTransport {
    source: Option<BumbleError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TerminalPairTrace {
    PollFailure,
    Drain(Duration),
    Disconnect,
    Close,
    TransportDrop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PartialCleanupTrace {
    Drain(Duration),
    Disconnect,
    Close,
}

impl PairDriver for TerminalPairDriver {
    fn after_pair_enqueued(&mut self) -> crate::Result<()> {
        self.fail_poll.store(true, Ordering::SeqCst);
        self.activity.notify();
        Ok(())
    }
}

impl TransportPort for TerminalPairTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        self.inner.open(activity)
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        if self.fail_poll.load(Ordering::SeqCst) {
            lock(&self.cleanup_trace).push(TerminalPairTrace::PollFailure);
            return Err(TransportError::with_source(
                TransportErrorKind::SourceTerminated,
                Arc::new(io::Error::other("secret terminal pair backend detail")),
            ));
        }
        self.inner.poll(timeout)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.inner.send_interrupt(payload)
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(TerminalPairTrace::Drain(timeout));
        Err(TransportError::with_source(
            TransportErrorKind::SourceTerminated,
            Arc::new(io::Error::other("secret terminal cleanup detail")),
        ))
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(TerminalPairTrace::Disconnect);
        self.inner.disconnect()
    }

    fn close(&mut self) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(TerminalPairTrace::Close);
        self.inner.close()
    }
}

impl Drop for TerminalPairTransport {
    fn drop(&mut self) {
        lock(&self.cleanup_trace).push(TerminalPairTrace::TransportDrop);
    }
}

impl TransportPort for PartiallyOpeningTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        self.inner.open(activity)?;
        Err(TransportError::with_source(
            TransportErrorKind::OpenFailed,
            Arc::new(io::Error::other("secret partial open detail")),
        ))
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.inner.poll(timeout)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.inner.send_interrupt(payload)
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(PartialCleanupTrace::Drain(timeout));
        self.inner.drain_interrupt(timeout)
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(PartialCleanupTrace::Disconnect);
        self.inner.disconnect()
    }

    fn close(&mut self) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(PartialCleanupTrace::Close);
        self.inner.close()
    }
}

#[cfg(feature = "bumble")]
impl TransportPort for BumbleOpenErrorTransport {
    fn open(&mut self, _activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        let source = self
            .source
            .take()
            .expect("Bumble open error transport is opened once");
        Err(TransportError::with_source(
            TransportErrorKind::OpenFailed,
            Arc::new(source),
        ))
    }

    fn poll(&mut self, _timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        Err(TransportError::new(TransportErrorKind::Closed))
    }

    fn send_interrupt(&mut self, _payload: &[u8]) -> TransportResult<SendAcceptance> {
        Err(TransportError::new(TransportErrorKind::Closed))
    }

    fn drain_interrupt(&mut self, _timeout: Duration) -> TransportResult<()> {
        Ok(())
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        Ok(())
    }

    fn close(&mut self) -> TransportResult<()> {
        Ok(())
    }
}

impl TransportPort for PanickingOpenTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        self.inner.open(activity).expect("open test transport");
        panic!("secret partial open panic");
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.inner.poll(timeout)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.inner.send_interrupt(payload)
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(PartialCleanupTrace::Drain(timeout));
        self.inner.drain_interrupt(timeout)
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(PartialCleanupTrace::Disconnect);
        self.inner.disconnect()
    }

    fn close(&mut self) -> TransportResult<()> {
        lock(&self.cleanup_trace).push(PartialCleanupTrace::Close);
        self.inner.close()
    }
}

impl TransportPort for DropTrackingTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        self.inner.open(activity)
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.inner.poll(timeout)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.inner.send_interrupt(payload)
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        self.inner.drain_interrupt(timeout)
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        self.inner.disconnect()
    }

    fn close(&mut self) -> TransportResult<()> {
        self.inner.close()
    }
}

impl Drop for DropTrackingTransport {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl TransportPort for DrainFailingTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        self.inner.open(activity)
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.inner.poll(timeout)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.inner.send_interrupt(payload)
    }

    fn drain_interrupt(&mut self, _timeout: Duration) -> TransportResult<()> {
        Err(TransportError::with_source(
            TransportErrorKind::SourceTerminated,
            Arc::new(io::Error::other("secret cleanup backend detail")),
        ))
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        self.inner.disconnect()
    }

    fn close(&mut self) -> TransportResult<()> {
        self.inner.close()
    }
}

#[derive(Default)]
struct MemoryProfileStore {
    bytes: Option<Vec<u8>>,
}

impl ProfileCreateTargetPort for MemoryProfileStore {
    fn inspect(&mut self, _path: &Path) -> io::Result<ProfileCreateTargetState> {
        Ok(if self.bytes.is_some() {
            ProfileCreateTargetState::Existing
        } else {
            ProfileCreateTargetState::Absent
        })
    }
}

impl ProfileReadPort for MemoryProfileStore {
    fn read(&mut self, _path: &Path) -> io::Result<Vec<u8>> {
        self.bytes
            .clone()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "profile is absent"))
    }
}

impl ProfileCreatePort for MemoryProfileStore {
    fn create_new(&mut self, _path: &Path, bytes: &[u8]) -> io::Result<()> {
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

impl WorkerWaiter for ObservedWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError> {
        let _ = self.requests.try_send(request);
        self.inner.wait(request, clock)
    }
}

fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut raw = vec![0x01, 0];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.push(subcommand_id);
    raw.extend_from_slice(payload);
    raw
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
