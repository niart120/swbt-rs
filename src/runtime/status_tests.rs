use std::{
    collections::VecDeque,
    error::Error as StdError,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    thread,
    time::Duration,
};

use crate::{
    Controller, ErrorKind,
    diagnostics::event::DiagnosticEvent,
    input::{InputState, ProButton},
    model::{ControllerKind, Pro},
    protocol::SwitchHidProtocol,
    reporting::{Direct, Periodic, ReportingKind},
    runtime::{
        command::{CommandEnqueueError, CommandResponseError},
        direct::{DirectTapError, DirectTapInterruption},
        error_map::{
            map_command_error, map_enqueue_error, map_join_error, map_response_error,
            map_worker_failure, unsupported_capability,
        },
        lifecycle::LifecycleCommandError,
        status::{status_projection, status_projection_with_emitter},
        transport::{
            ActivityNotifier, HidChannel, SendAcceptance, TransportCapabilities, TransportError,
            TransportErrorKind, TransportEvent, TransportPort, TransportResult, activity_channel,
            fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
        },
        worker::{
            CommandSource, DirectCommand, MonotonicClock, PriorityShutdown, WorkerBudget,
            WorkerCommandError, WorkerCore, WorkerCoreError, WorkerStep,
        },
        worker_thread::{WorkerFailureCause, WorkerJoinError},
    },
};

use serde_json::Value;

const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

#[test]
fn projection_tracks_committed_fields_and_acceptance_counters() {
    let (mut worker, control, clock, controller) = projected_direct_worker();

    let opened = controller.status();
    assert_eq!(opened.lifecycle, crate::LifecycleState::Open);
    assert!(!opened.connected);
    assert_eq!(opened.controller_kind, ControllerKind::Pro);
    assert_eq!(opened.reporting_kind, ReportingKind::Direct);
    assert_eq!(opened.input_reports_accepted, 0);
    assert_eq!(opened.replies_accepted, 0);
    assert_eq!(controller.snapshot(), InputState::<Pro>::neutral());

    worker
        .begin_connection(clock.now(), CONNECTION_TIMEOUT)
        .expect("begin fake connection");
    control.inject_connected().expect("link event");
    control
        .inject_hid_channel_opened(HidChannel::Control)
        .expect("control channel");
    control
        .inject_hid_channel_opened(HidChannel::Interrupt)
        .expect("interrupt channel");
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));

    let connecting = controller.status();
    assert_eq!(connecting.lifecycle, crate::LifecycleState::Connecting);
    assert!(connecting.connected);
    assert_eq!(connecting.controller_kind, ControllerKind::Pro);
    assert_eq!(connecting.reporting_kind, ReportingKind::Direct);
    assert_eq!(connecting.report_mode, None);
    assert_eq!(connecting.input_reports_accepted, 1);
    assert_eq!(connecting.replies_accepted, 0);
    assert_eq!(connecting.last_subcommand, None);
    assert_eq!(connecting.last_disconnect_reason, None);
    assert_eq!(connecting.worker_failure, None);
    assert_eq!(controller.snapshot(), InputState::<Pro>::neutral());

    clock.set(Duration::from_millis(10));
    control
        .inject_hid_output(HidChannel::Control, &subcommand_report(0x03, &[0x30]))
        .expect("report mode");
    control
        .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x30, &[0x01]))
        .expect("player lights");
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));

    let ready = controller.status();
    assert_eq!(ready.lifecycle, crate::LifecycleState::Ready);
    assert!(ready.connected);
    assert_eq!(ready.report_mode, Some(0x30));
    assert_eq!(ready.input_reports_accepted, 1);
    assert_eq!(ready.replies_accepted, 2);
    assert_eq!(ready.last_subcommand, Some(0x30));

    let pressed = InputState::<Pro>::neutral().with_buttons([ProButton::A]);
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::one(DirectCommand::Send(pressed.clone())),
    ));
    assert_eq!(controller.snapshot(), pressed);
    assert_eq!(controller.status().input_reports_accepted, 2);

    let rejected_state = InputState::<Pro>::neutral().with_buttons([ProButton::X]);
    control.script_sends([ScriptedSendOutcome::Rejected]);
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::one(DirectCommand::Send(rejected_state)),
    ));
    assert_eq!(controller.snapshot(), pressed);
    assert_eq!(controller.status().input_reports_accepted, 2);

    control.script_sends([ScriptedSendOutcome::Rejected]);
    control
        .inject_hid_output(HidChannel::Control, &subcommand_report(0x40, &[0x01]))
        .expect("IMU command whose reply is rejected");
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));
    let rejected = controller.status();
    assert_eq!(rejected.last_subcommand, Some(0x40));
    assert_eq!(rejected.replies_accepted, 2);
    assert_eq!(rejected.report_mode, Some(0x30));

    control
        .inject_hid_output(HidChannel::Control, &subcommand_report(0x03, &[]))
        .expect("syntactically valid subcommand with invalid payload");
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));
    let semantic_failure = controller.status();
    assert_eq!(semantic_failure.last_subcommand, Some(0x03));
    assert_eq!(semantic_failure.replies_accepted, 2);

    control
        .inject_disconnected(Some(0x13))
        .expect("current session disconnect");
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));
    let disconnected = controller.status();
    assert_eq!(disconnected.lifecycle, crate::LifecycleState::Open);
    assert!(!disconnected.connected);
    assert_eq!(disconnected.report_mode, None);
    assert_eq!(disconnected.last_disconnect_reason, Some(0x13));
    assert_eq!(disconnected.last_subcommand, Some(0x03));
    assert_eq!(disconnected.input_reports_accepted, 2);
    assert_eq!(disconnected.replies_accepted, 2);
    assert_eq!(controller.snapshot(), pressed);

    worker
        .begin_connection(clock.now(), CONNECTION_TIMEOUT)
        .expect("begin replacement session");
    let replacement = controller.status();
    assert_eq!(replacement.lifecycle, crate::LifecycleState::Connecting);
    assert!(!replacement.connected);
    assert_eq!(replacement.report_mode, None);
    assert_eq!(replacement.last_subcommand, None);
    assert_eq!(replacement.last_disconnect_reason, None);
    assert_eq!(replacement.input_reports_accepted, 2);
    assert_eq!(replacement.replies_accepted, 2);
    assert_eq!(controller.snapshot(), InputState::<Pro>::neutral());

    control
        .terminate_with(SentinelSource)
        .expect("terminate replacement session");
    let failed = worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    );
    assert!(matches!(failed, WorkerStep::Failed { .. }));
    let terminal = controller.status();
    assert_eq!(terminal.lifecycle, crate::LifecycleState::Failed);
    assert!(!terminal.connected);
    assert_eq!(terminal.report_mode, None);
    assert_eq!(
        terminal.worker_failure.as_deref(),
        Some("worker transport failed")
    );
    assert!(
        !terminal
            .worker_failure
            .as_deref()
            .unwrap_or_default()
            .contains("T26_SECRET")
    );
}

#[test]
fn runtime_projects_ordered_session_events_from_committed_status_updates() {
    let events = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&events);
    let emitter = Arc::new(move |event: DiagnosticEvent| {
        captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event.to_value());
    });
    let (publisher, reader) = status_projection_with_emitter::<Pro, Direct>(emitter);
    let (mut transport, control) = FakeTransport::with_limits(16, 3);
    let (activity, _wake_receiver) = activity_channel();
    transport.open(activity).expect("open fake transport");
    let mut worker = WorkerCore::new_direct_with_status(
        protocol(),
        Box::new(transport),
        WorkerBudget::new(2, 1),
        Box::new(|_| {}),
        publisher,
    );
    let clock = FakeClock::at(Duration::ZERO);

    prime_ready(&mut worker, &control, &clock);
    let pressed = InputState::<Pro>::neutral().with_buttons([ProButton::A]);
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::one(DirectCommand::Send(pressed)),
    ));
    control
        .inject_disconnected(Some(0x13))
        .expect("current session disconnect");
    assert_continue(worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));

    let records = events
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let names = records
        .iter()
        .map(|record| record["event"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "session_started",
            "lifecycle_changed",
            "report_tx_accepted",
            "subcommand_observed",
            "reply_tx_accepted",
            "subcommand_observed",
            "reply_tx_accepted",
            "lifecycle_changed",
            "report_tx_accepted",
            "session_ended",
        ]
    );
    assert!(records.iter().all(|record| record["session_id"] == 1));
    assert!(
        records
            .iter()
            .all(|record| record["controller_kind"] == "pro")
    );
    assert!(
        records
            .iter()
            .all(|record| record["reporting_kind"] == "direct")
    );
    assert_eq!(records[1]["lifecycle"], "connecting");
    assert_eq!(records[2]["input_reports_accepted"], 1);
    assert_eq!(records[3]["subcommand_id"], 0x03);
    assert_eq!(records[4]["replies_accepted"], 1);
    assert_eq!(records[5]["subcommand_id"], 0x30);
    assert_eq!(records[6]["replies_accepted"], 2);
    assert_eq!(records[7]["lifecycle"], "ready");
    assert_eq!(records[8]["input_reports_accepted"], 2);
    assert_eq!(records[9]["lifecycle"], "open");
    assert_eq!(records[9]["disconnect_reason"], 0x13);
    assert_eq!(reader.status().last_disconnect_reason, Some(0x13));
}

#[test]
fn runtime_failure_emits_a_typed_category_without_its_error_source() {
    let events = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let captured = Arc::clone(&events);
    let emitter = Arc::new(move |event: DiagnosticEvent| {
        captured
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event.to_value());
    });
    let (publisher, reader) = status_projection_with_emitter::<Pro, Direct>(emitter);
    let (mut transport, control) = FakeTransport::with_limits(16, 3);
    let (activity, _wake_receiver) = activity_channel();
    transport.open(activity).expect("open fake transport");
    let mut worker = WorkerCore::new_direct_with_status(
        protocol(),
        Box::new(transport),
        WorkerBudget::new(2, 1),
        Box::new(|_| {}),
        publisher,
    );
    let clock = FakeClock::at(Duration::ZERO);

    worker
        .begin_connection(clock.now(), CONNECTION_TIMEOUT)
        .expect("begin fake connection");
    control
        .terminate_with(SentinelSource)
        .expect("terminate current session");
    let failed = worker.step(
        &clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    );
    assert!(matches!(failed, WorkerStep::Failed { .. }));

    let records = events
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let names = records
        .iter()
        .map(|record| record["event"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "session_started",
            "lifecycle_changed",
            "worker_failed",
            "lifecycle_changed",
            "session_ended",
        ]
    );
    assert_eq!(records[2]["failure_category"], "transport");
    assert_eq!(records[3]["lifecycle"], "failed");
    assert_eq!(records[4]["lifecycle"], "failed");
    assert!(
        !serde_json::to_string(&*records)
            .unwrap()
            .contains("T26_SECRET")
    );
    assert_eq!(reader.status().lifecycle, crate::LifecycleState::Failed);
    assert_eq!(
        reader.status().worker_failure.as_deref(),
        Some("worker transport failed")
    );
}

#[test]
fn status_and_snapshot_return_while_transport_poll_is_blocked() {
    let (mut inner, control) = FakeTransport::with_limits(16, 3);
    let (activity, _wake_receiver) = activity_channel();
    inner.open(activity).expect("open fake transport");
    let (transport, block) = BlockingPollTransport::new(inner);
    let controller = Controller::<Pro, Direct>::builder("test:status")
        .build()
        .expect("ephemeral test controller");
    let publisher = controller.status_publisher();
    let mut worker = WorkerCore::new_direct_with_status(
        protocol(),
        Box::new(transport),
        WorkerBudget::new(2, 1),
        Box::new(|_| {}),
        publisher,
    );
    let clock = FakeClock::at(Duration::ZERO);
    prime_ready(&mut worker, &control, &clock);
    let pressed = InputState::<Pro>::neutral().with_buttons([ProButton::A]);

    block.arm();
    let expected = pressed.clone();
    let worker_thread = thread::spawn(move || {
        let step = worker.step(
            &clock,
            &mut NoShutdown,
            &mut Commands::one(DirectCommand::Send(pressed)),
        );
        (worker, step)
    });
    block.wait_until_blocked();

    let (query_sender, query_receiver) = sync_channel(1);
    let query_thread = thread::spawn(move || {
        let status = controller.status();
        let snapshot = controller.snapshot();
        query_sender
            .send((controller, status, snapshot))
            .expect("query result receiver");
    });
    let (controller, status, snapshot) = query_receiver
        .recv_timeout(Duration::from_secs(1))
        .expect("status query must not wait for blocked transport");

    assert_eq!(status.lifecycle, crate::LifecycleState::Ready);
    assert_eq!(status.input_reports_accepted, 2);
    assert_eq!(snapshot, expected);

    block.release();
    query_thread.join().expect("query thread");
    let (_worker, step) = worker_thread.join().expect("worker step");
    assert_continue(step);
    drop(controller);
}

#[test]
fn public_error_mapping_preserves_categories_sources_and_redaction() {
    assert_eq!(
        map_enqueue_error(CommandEnqueueError::InvariantViolation).kind(),
        ErrorKind::Internal
    );
    assert_eq!(
        map_enqueue_error(CommandEnqueueError::Disconnected).kind(),
        ErrorKind::WorkerFailed
    );
    assert_eq!(
        map_response_error(CommandResponseError::WorkerFailed).kind(),
        ErrorKind::WorkerFailed
    );
    assert_eq!(
        map_command_error(WorkerCommandError::Lifecycle(
            LifecycleCommandError::Shutdown
        ))
        .kind(),
        ErrorKind::Shutdown
    );
    assert_eq!(
        map_command_error(WorkerCommandError::Lifecycle(LifecycleCommandError::Failed)).kind(),
        ErrorKind::WorkerFailed
    );
    assert_eq!(
        map_command_error(WorkerCommandError::Direct(DirectTapError::Interrupted(
            DirectTapInterruption::Shutdown
        )))
        .kind(),
        ErrorKind::Shutdown
    );
    assert_eq!(
        map_command_error(WorkerCommandError::Direct(DirectTapError::NotReady)).kind(),
        ErrorKind::TransportClosed
    );
    assert_eq!(
        map_worker_failure(WorkerFailureCause::Panicked).kind(),
        ErrorKind::WorkerFailed
    );
    assert_eq!(
        map_join_error(WorkerJoinError::Panicked).kind(),
        ErrorKind::WorkerFailed
    );
    assert_eq!(
        unsupported_capability("Bluetooth transport").kind(),
        ErrorKind::UnsupportedCapability
    );

    let transport = TransportError::with_source(
        TransportErrorKind::SourceTerminated,
        Arc::new(SentinelSource),
    );
    let public = map_worker_failure(WorkerFailureCause::Core(WorkerCoreError::Transport(
        transport,
    )));
    assert_eq!(public.kind(), ErrorKind::WorkerFailed);
    let transport_source = public.source().expect("typed transport source");
    assert_eq!(
        transport_source
            .source()
            .expect("backend source")
            .to_string(),
        "T26_SECRET"
    );
    assert!(!public.to_string().contains("T26_SECRET"));
    assert!(!format!("{public:?}").contains("T26_SECRET"));
}

#[test]
fn controller_and_reporting_kinds_are_derived_from_the_requested_types() {
    let (_publisher, direct_reader) = status_projection::<Pro, Direct>();
    let (_publisher, periodic_reader) = status_projection::<Pro, Periodic>();

    let direct = direct_reader.status();
    let periodic = periodic_reader.status();

    assert_eq!(direct.controller_kind, ControllerKind::Pro);
    assert_eq!(periodic.controller_kind, ControllerKind::Pro);
    assert_eq!(direct.reporting_kind, ReportingKind::Direct);
    assert_eq!(periodic.reporting_kind, ReportingKind::Periodic);
}

fn projected_direct_worker() -> (
    WorkerCore<Pro, Direct>,
    FakeTransportControl,
    FakeClock,
    Controller<Pro, Direct>,
) {
    let (mut transport, control) = FakeTransport::with_limits(16, 3);
    let (activity, _wake_receiver) = activity_channel();
    transport.open(activity).expect("open fake transport");
    let controller = Controller::<Pro, Direct>::builder("test:status")
        .build()
        .expect("ephemeral test controller");
    let publisher = controller.status_publisher();
    let worker = WorkerCore::new_direct_with_status(
        protocol(),
        Box::new(transport),
        WorkerBudget::new(2, 1),
        Box::new(|_| {}),
        publisher,
    );
    (worker, control, FakeClock::at(Duration::ZERO), controller)
}

fn prime_ready(
    worker: &mut WorkerCore<Pro, Direct>,
    control: &FakeTransportControl,
    clock: &FakeClock,
) {
    worker
        .begin_connection(clock.now(), CONNECTION_TIMEOUT)
        .expect("begin fake connection");
    control.inject_connected().expect("link event");
    control
        .inject_hid_channel_opened(HidChannel::Control)
        .expect("control channel");
    control
        .inject_hid_channel_opened(HidChannel::Interrupt)
        .expect("interrupt channel");
    assert_continue(worker.step(
        clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));
    clock.set(Duration::from_millis(10));
    control
        .inject_hid_output(HidChannel::Control, &subcommand_report(0x03, &[0x30]))
        .expect("report mode");
    control
        .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x30, &[0x01]))
        .expect("player lights");
    assert_continue(worker.step(
        clock,
        &mut NoShutdown,
        &mut Commands::<DirectCommand<Pro>>::none(),
    ));
}

fn assert_continue(step: WorkerStep) {
    assert!(matches!(step, WorkerStep::Continue(_)));
}

fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut raw = vec![0x01, 0];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.push(subcommand_id);
    raw.extend_from_slice(payload);
    raw
}

fn protocol() -> SwitchHidProtocol<Pro> {
    SwitchHidProtocol::new(None, DEVICE_INFO_ADDRESS)
}

struct Commands<C> {
    queued: VecDeque<C>,
}

impl<C> Commands<C> {
    fn none() -> Self {
        Self {
            queued: VecDeque::new(),
        }
    }

    fn one(command: C) -> Self {
        Self {
            queued: VecDeque::from([command]),
        }
    }
}

impl<C> CommandSource<C> for Commands<C> {
    fn try_next(&mut self) -> Option<C> {
        self.queued.pop_front()
    }
}

struct NoShutdown;

impl PriorityShutdown for NoShutdown {
    fn take(&mut self) -> Option<crate::runtime::worker::ShutdownRequest> {
        None
    }
}

#[derive(Clone)]
struct FakeClock {
    now: Arc<std::sync::Mutex<Duration>>,
}

impl FakeClock {
    fn at(now: Duration) -> Self {
        Self {
            now: Arc::new(std::sync::Mutex::new(now)),
        }
    }

    fn set(&self, now: Duration) {
        let mut current = self
            .now
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert!(now >= *current);
        *current = now;
    }
}

impl MonotonicClock for FakeClock {
    fn now(&self) -> Duration {
        *self
            .now
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

struct BlockingPollTransport {
    inner: FakeTransport,
    armed: Arc<AtomicBool>,
    entered: SyncSender<()>,
    release: Receiver<()>,
}

struct PollBlockControl {
    armed: Arc<AtomicBool>,
    entered: Receiver<()>,
    release: SyncSender<()>,
}

impl BlockingPollTransport {
    fn new(inner: FakeTransport) -> (Self, PollBlockControl) {
        let armed = Arc::new(AtomicBool::new(false));
        let (entered_sender, entered) = sync_channel(1);
        let (release_sender, release) = sync_channel(1);
        (
            Self {
                inner,
                armed: Arc::clone(&armed),
                entered: entered_sender,
                release,
            },
            PollBlockControl {
                armed,
                entered,
                release: release_sender,
            },
        )
    }
}

impl PollBlockControl {
    fn arm(&self) {
        assert!(!self.armed.swap(true, Ordering::SeqCst));
    }

    fn wait_until_blocked(&self) {
        self.entered
            .recv_timeout(Duration::from_secs(1))
            .expect("worker must enter the armed transport poll");
    }

    fn release(&self) {
        self.release.send(()).expect("blocked transport receiver");
    }
}

impl TransportPort for BlockingPollTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        self.inner.open(activity)
    }

    fn start_pairing(&mut self) -> TransportResult<()> {
        self.inner.start_pairing()
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.send(()).expect("poll block observer");
            self.release.recv().expect("poll block release");
        }
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

#[derive(Debug)]
struct SentinelSource;

impl fmt::Display for SentinelSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("T26_SECRET")
    }
}

impl StdError for SentinelSource {}
