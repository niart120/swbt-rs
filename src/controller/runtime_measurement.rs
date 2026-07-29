use std::{
    env, fs,
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{
            Receiver, RecvTimeoutError, Sender, SyncSender, TryRecvError, channel, sync_channel,
        },
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::{
    input::InputState,
    model::Pro,
    protocol::SwitchHidProtocol,
    reporting::{Direct, Periodic, ReportingMode},
    runtime::{
        cleanup::CloseMode,
        command::{CommandClient, CommandEnqueueError, CommandResponse, command_channel},
        test_support::{TestTransport, TestTransportControl},
        transport::{
            ActivityNotifier, HidChannel, SendAcceptance, TransportEvent, TransportPort,
            TransportResult, activity_channel,
        },
        worker::{
            ChannelWorkerWaiter, DirectCommand, MonotonicClock, PeriodicCommand, RuntimeCommand,
            ShutdownRequest, WorkerBudget, WorkerCommandError, WorkerWaitError, WorkerWaitRequest,
            WorkerWaiter,
        },
        worker_thread::{
            PriorityShutdownClient, WorkerThread, WorkerThreadOutcome, priority_shutdown_channel,
            spawn_worker_thread,
        },
    },
};

use super::runtime::default_runtime_tuning;

const PERIOD: Duration = Duration::from_millis(8);
const FAKE_EVENT_CAPACITY: usize = 64;
const FAKE_POLL_BATCH: usize = 16;
const DEADLOCK_WATCHDOG: Duration = Duration::from_secs(5);
const FULL_IDLE_MS: u64 = 10_000;
const FULL_JITTER_SAMPLES: usize = 10_000;
const FULL_COMMAND_SAMPLES: usize = 10_000;
const FULL_TRANSPORT_SAMPLES: usize = 10_000;
const FULL_SHUTDOWN_SAMPLES: usize = 1_000;
const FULL_FAIRNESS_TICKS: usize = 10_000;
const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

#[test]
#[ignore = "manual release-profile decision gate; writes retained timing evidence"]
fn activity_wait_decision() {
    if cfg!(debug_assertions) {
        panic!("M2 retained measurements require a release test build");
    }
    let output = env::var_os("SWBT_MEASUREMENT_OUTPUT")
        .map(PathBuf::from)
        .expect("SWBT_MEASUREMENT_OUTPUT must name the raw NDJSON output");
    let config = ProbeConfig::from_env();
    let tuning = default_runtime_tuning();
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).expect("create measurement output directory");
    }
    let meta = measurement_meta(&config, tuning);
    let mut raw = NdjsonWriter::create(&output);
    raw.record(json!({
        "schema": "swbt.m2.activity-wait.raw.v2",
        "record": "meta",
        "meta": meta.clone(),
    }));

    let idle = measure_idle(&config, &output, tuning);
    emit_idle_record(&mut raw, &idle["raw"]);
    let jitter = measure_jitter(&config, tuning);
    emit_jitter_records(&mut raw, &jitter["raw"]);
    let command = measure_command_latency(&config, tuning);
    emit_command_records(&mut raw, &command["raw"]);
    let transport = measure_transport_latency(&config, tuning);
    emit_transport_records(&mut raw, &transport["raw"]);
    let shutdown = measure_shutdown_latency(&config, tuning);
    emit_shutdown_records(&mut raw, &shutdown["raw"]);
    let fairness = measure_fairness(&config, tuning);
    emit_fairness_records(&mut raw, &fairness["raw"], tuning.0);
    raw.finish();
    let summary = json!({
        "schema": "swbt.m2.activity-wait.summary.v2",
        "meta": meta,
        "idle": idle["summary"].clone(),
        "jitter": jitter["summary"].clone(),
        "command": command["summary"].clone(),
        "transport": transport["summary"].clone(),
        "shutdown": shutdown["summary"].clone(),
        "fairness": fairness["summary"].clone(),
    });

    let summary_path = output.with_file_name("activity-wait-summary.json");
    fs::write(
        summary_path,
        serde_json::to_vec_pretty(&summary).expect("serialize measurement summary"),
    )
    .expect("write measurement summary");

    println!("measurement_complete");
}

struct NdjsonWriter {
    writer: BufWriter<File>,
}

impl NdjsonWriter {
    fn create(path: &Path) -> Self {
        Self {
            writer: BufWriter::new(File::create(path).expect("create raw NDJSON output")),
        }
    }

    fn record(&mut self, value: Value) {
        serde_json::to_writer(&mut self.writer, &value).expect("serialize raw NDJSON record");
        self.writer
            .write_all(b"\n")
            .expect("terminate raw NDJSON record");
    }

    fn finish(mut self) {
        self.writer.flush().expect("flush raw NDJSON output");
    }
}

fn emit_idle_record(writer: &mut NdjsonWriter, raw: &Value) {
    writer.record(json!({
        "schema": "swbt.m2.activity-wait.raw.v2",
        "metric": "idle",
        "sample_index": 0,
        "sample": raw,
    }));
}

fn emit_jitter_records(writer: &mut NdjsonWriter, raw: &Value) {
    let targets = value_array(raw, "target_ns");
    let wait_returns = value_array(raw, "wait_return_ns");
    let actuals = value_array(raw, "actual_ns");
    let lateness = value_array(raw, "lateness_ns");
    let intervals = value_array(raw, "interval_ns");
    let interval_errors = value_array(raw, "interval_error_ns");
    let interval_error_ppm = value_array(raw, "interval_error_ppm");
    for index in 0..targets.len() {
        writer.record(json!({
            "schema": "swbt.m2.activity-wait.raw.v2",
            "metric": "jitter",
            "sample_index": index,
            "period_ns": duration_ns(PERIOD),
            "target_ns": targets[index],
            "wait_return_ns": wait_returns[index],
            "transport_accept_ns": actuals[index],
            "lateness_ns": lateness[index],
            "interval_ns": previous_value(intervals, index),
            "interval_error_ns": previous_value(interval_errors, index),
            "interval_error_ppm": previous_value(interval_error_ppm, index),
            "interval_error_ppm_basis": "period_ns",
            "interval_error_ppm_ideal": 0,
        }));
    }
}

fn emit_command_records(writer: &mut NdjsonWriter, raw: &Value) {
    let enqueue = value_array(raw, "enqueue_ns");
    let accepted = value_array(raw, "transport_accept_ns");
    let responses = value_array(raw, "response_ns");
    for index in 0..enqueue.len() {
        writer.record(json!({
            "schema": "swbt.m2.activity-wait.raw.v2",
            "metric": "command",
            "sample_index": index,
            "enqueue_ns": enqueue[index],
            "transport_accept_ns": accepted[index],
            "response_ns": responses[index],
        }));
    }
}

fn emit_transport_records(writer: &mut NdjsonWriter, raw: &Value) {
    let injected = value_array(raw, "inject_return_ns");
    let drained = value_array(raw, "drained_ns");
    for index in 0..injected.len() {
        writer.record(json!({
            "schema": "swbt.m2.activity-wait.raw.v2",
            "metric": "transport",
            "sample_index": index,
            "inject_return_ns": injected[index],
            "poll_observed_ns": drained[index],
        }));
    }
}

fn emit_shutdown_records(writer: &mut NdjsonWriter, raw: &Value) {
    let idle = value_array(raw, "idle_ns");
    let saturated = value_array(raw, "saturated_ns");
    for index in 0..idle.len() {
        writer.record(json!({
            "schema": "swbt.m2.activity-wait.raw.v2",
            "metric": "shutdown",
            "condition": "idle",
            "sample_index": index,
            "completion_and_join_ns": idle[index],
        }));
        writer.record(json!({
            "schema": "swbt.m2.activity-wait.raw.v2",
            "metric": "shutdown",
            "condition": "saturated",
            "sample_index": index,
            "completion_and_join_ns": saturated[index],
        }));
    }
}

fn emit_fairness_records(writer: &mut NdjsonWriter, raw: &Value, command_capacity: usize) {
    let targets = value_array(raw, "target_ns");
    let releases = value_array(raw, "gate_release_requested_ns");
    let staging = value_array(raw, "staging_ns");
    let actuals = value_array(raw, "actual_ns");
    let post_release_accept = value_array(raw, "post_release_accept_ns");
    let lateness = value_array(raw, "lateness_ns");
    let intervals = value_array(raw, "interval_ns");
    let interval_errors = value_array(raw, "interval_error_ns");
    let interval_error_ppm = value_array(raw, "interval_error_ppm");
    let command_responses = value_array(raw, "command_response_ns");
    let post_release_command = value_array(raw, "post_release_command_completion_observed_ns");
    let reply_attempts = value_array(raw, "reply_attempt_ns");
    let post_release_replies = value_array(raw, "post_release_reply_attempt_ns");
    let commands = value_array(raw, "commands_completed_per_tick");
    let transport = value_array(raw, "transport_events_per_tick");
    for index in 0..targets.len() {
        let reply_start = index.saturating_mul(command_capacity);
        let reply_end = reply_start
            .saturating_add(command_capacity)
            .min(command_responses.len());
        let attempt_start = index.saturating_mul(FAKE_EVENT_CAPACITY);
        let attempt_end = attempt_start
            .saturating_add(FAKE_EVENT_CAPACITY)
            .min(reply_attempts.len());
        writer.record(json!({
            "schema": "swbt.m2.activity-wait.raw.v2",
            "metric": "fairness",
            "sample_index": index,
            "period_ns": duration_ns(PERIOD),
            "target_ns": targets[index],
            "gate_release_requested_ns": releases[index],
            "staging_ns": staging[index],
            "transport_accept_ns": actuals[index],
            "post_release_accept_ns": post_release_accept[index],
            "lateness_ns": lateness[index],
            "interval_ns": previous_value(intervals, index),
            "interval_error_ns": previous_value(interval_errors, index),
            "interval_error_ppm": previous_value(interval_error_ppm, index),
            "interval_error_ppm_basis": "period_ns",
            "interval_error_ppm_ideal": 0,
            "command_response_ns": &command_responses[reply_start..reply_end],
            "post_release_command_completion_observed_ns":
                &post_release_command[reply_start..reply_end],
            "reply_attempt_ns": &reply_attempts[attempt_start..attempt_end],
            "post_release_reply_attempt_ns":
                &post_release_replies[attempt_start..attempt_end],
            "reply_boundary": "send_attempt",
            "reply_outcome": "rejected_for_fairness_probe",
            "commands_completed": commands[index],
            "transport_events_drained": transport[index],
        }));
    }
}

fn value_array<'a>(object: &'a Value, key: &str) -> &'a [Value] {
    object[key]
        .as_array()
        .unwrap_or_else(|| panic!("{key} must be an array"))
}

fn previous_value(values: &[Value], index: usize) -> Value {
    index
        .checked_sub(1)
        .and_then(|previous| values.get(previous))
        .cloned()
        .unwrap_or(Value::Null)
}

#[derive(Clone, Copy)]
struct ProbeConfig {
    idle: Duration,
    jitter_samples: usize,
    command_samples: usize,
    transport_samples: usize,
    shutdown_samples: usize,
    fairness_ticks: usize,
}

impl ProbeConfig {
    fn from_env() -> Self {
        let config = Self {
            idle: Duration::from_millis(env_u64("SWBT_MEASUREMENT_IDLE_MS", FULL_IDLE_MS)),
            jitter_samples: env_usize("SWBT_MEASUREMENT_JITTER_SAMPLES", FULL_JITTER_SAMPLES),
            command_samples: env_usize("SWBT_MEASUREMENT_COMMAND_SAMPLES", FULL_COMMAND_SAMPLES),
            transport_samples: env_usize(
                "SWBT_MEASUREMENT_TRANSPORT_SAMPLES",
                FULL_TRANSPORT_SAMPLES,
            ),
            shutdown_samples: env_usize("SWBT_MEASUREMENT_SHUTDOWN_SAMPLES", FULL_SHUTDOWN_SAMPLES),
            fairness_ticks: env_usize("SWBT_MEASUREMENT_FAIRNESS_TICKS", FULL_FAIRNESS_TICKS),
        };
        if env::var("SWBT_MEASUREMENT_MODE").as_deref() == Ok("full") {
            assert_eq!(config.idle, Duration::from_millis(FULL_IDLE_MS));
            assert_eq!(config.jitter_samples, FULL_JITTER_SAMPLES);
            assert_eq!(config.command_samples, FULL_COMMAND_SAMPLES);
            assert_eq!(config.transport_samples, FULL_TRANSPORT_SAMPLES);
            assert_eq!(config.shutdown_samples, FULL_SHUTDOWN_SAMPLES);
            assert_eq!(config.fairness_ticks, FULL_FAIRNESS_TICKS);
        }
        config
    }
}

#[derive(Clone)]
struct InstantClock {
    origin: Instant,
}

impl InstantClock {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    fn elapsed_at(&self, instant: Instant) -> Duration {
        instant.saturating_duration_since(self.origin)
    }
}

impl MonotonicClock for InstantClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

#[derive(Clone, Copy)]
struct SendObservation {
    completed_at: Instant,
    report_id: Option<u8>,
    accepted: bool,
}

#[derive(Clone, Copy)]
struct PollObservation {
    observed_at: Instant,
    events: usize,
}

struct MeasuredTransport {
    inner: TestTransport,
    sends: Sender<SendObservation>,
    polls: Sender<PollObservation>,
    poll_calls: Arc<AtomicU64>,
    poll_events: Arc<AtomicU64>,
    poll_errors: Arc<AtomicU64>,
}

impl TransportPort for MeasuredTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
        self.inner.open(activity)
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.poll_calls.fetch_add(1, Ordering::Release);
        let result = self.inner.poll(timeout);
        match &result {
            Ok(events) => {
                self.poll_events.fetch_add(
                    u64::try_from(events.len()).unwrap_or(u64::MAX),
                    Ordering::Release,
                );
                if !events.is_empty() {
                    let _ = self.polls.send(PollObservation {
                        observed_at: Instant::now(),
                        events: events.len(),
                    });
                }
            }
            Err(_) => {
                self.poll_errors.fetch_add(1, Ordering::Release);
            }
        }
        result
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        let result = self.inner.send_interrupt(payload);
        let _ = self.sends.send(SendObservation {
            completed_at: Instant::now(),
            report_id: payload.first().copied(),
            accepted: result.is_ok(),
        });
        result
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

struct TransportTelemetry {
    sends: Receiver<SendObservation>,
    polls: Receiver<PollObservation>,
    poll_calls: Arc<AtomicU64>,
    poll_events: Arc<AtomicU64>,
    poll_errors: Arc<AtomicU64>,
}

struct WorkerHarness<R: ReportingMode> {
    activity: ActivityNotifier,
    clock: InstantClock,
    commands: CommandClient<RuntimeCommand<Pro, R>>,
    shutdown: PriorityShutdownClient,
    thread: Option<WorkerThread>,
    control: TestTransportControl,
    telemetry: TransportTelemetry,
}

impl<R> WorkerHarness<R>
where
    R: ReportingMode,
{
    fn request_shutdown(&self) {
        assert!(
            self.shutdown
                .request_for_test(ShutdownRequest::explicit(CloseMode::WithoutNeutral)),
            "measurement shutdown request must win once"
        );
    }

    fn finish(&mut self) -> WorkerThreadOutcome {
        finish_worker(
            self.thread
                .take()
                .expect("measurement harness retains its worker thread"),
        )
    }
}

struct FirstWaiter {
    inner: ChannelWorkerWaiter,
    first_wait: Option<SyncSender<WorkerWaitRequest>>,
    counters: Arc<WaitCounters>,
}

#[derive(Default)]
struct WaitCounters {
    entries: AtomicU64,
    returns: AtomicU64,
}

impl FirstWaiter {
    fn new(
        receiver: Receiver<()>,
        first_wait: SyncSender<WorkerWaitRequest>,
        counters: Arc<WaitCounters>,
    ) -> Self {
        Self {
            inner: ChannelWorkerWaiter::new(receiver),
            first_wait: Some(first_wait),
            counters,
        }
    }
}

impl WorkerWaiter for FirstWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError> {
        self.counters.entries.fetch_add(1, Ordering::Release);
        if let Some(first_wait) = self.first_wait.take() {
            first_wait
                .send(request)
                .expect("measurement first-wait observer remains connected");
        }
        let result = self.inner.wait(request, clock);
        self.counters.returns.fetch_add(1, Ordering::Release);
        result
    }
}

#[derive(Clone, Copy)]
struct DeadlineObservation {
    deadline: Duration,
    returned_at: Duration,
}

struct DeadlineWaiter {
    inner: ChannelWorkerWaiter,
    observations: Sender<DeadlineObservation>,
}

impl DeadlineWaiter {
    fn new(receiver: Receiver<()>, observations: Sender<DeadlineObservation>) -> Self {
        Self {
            inner: ChannelWorkerWaiter::new(receiver),
            observations,
        }
    }
}

impl WorkerWaiter for DeadlineWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError> {
        let result = self.inner.wait(request, clock);
        if result.is_ok() {
            if let WorkerWaitRequest::ActivityOrDeadline(deadline) = request {
                let returned_at = clock.now();
                if returned_at >= deadline {
                    let _ = self.observations.send(DeadlineObservation {
                        deadline,
                        returned_at,
                    });
                }
            }
        }
        result
    }
}

struct ActivityEntryWaiter {
    inner: ChannelWorkerWaiter,
    entries: Arc<AtomicU64>,
}

impl ActivityEntryWaiter {
    fn new(receiver: Receiver<()>, entries: Arc<AtomicU64>) -> Self {
        Self {
            inner: ChannelWorkerWaiter::new(receiver),
            entries,
        }
    }
}

impl WorkerWaiter for ActivityEntryWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError> {
        if request == WorkerWaitRequest::Activity {
            self.entries.fetch_add(1, Ordering::Release);
        }
        self.inner.wait(request, clock)
    }
}

struct RequestEntryWaiter {
    inner: ChannelWorkerWaiter,
    entries: SyncSender<WorkerWaitRequest>,
}

impl RequestEntryWaiter {
    fn new(receiver: Receiver<()>, entries: SyncSender<WorkerWaitRequest>) -> Self {
        Self {
            inner: ChannelWorkerWaiter::new(receiver),
            entries,
        }
    }
}

impl WorkerWaiter for RequestEntryWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError> {
        self.entries
            .send(request)
            .map_err(|_| WorkerWaitError::Disconnected)?;
        self.inner.wait(request, clock)
    }
}

struct GateControl {
    enabled: Arc<AtomicBool>,
    entered: Receiver<WorkerWaitRequest>,
    release: Sender<()>,
}

impl GateControl {
    fn enable(&self) {
        self.enabled.store(true, Ordering::Release);
    }

    fn next(&self) -> WorkerWaitRequest {
        self.entered
            .recv_timeout(DEADLOCK_WATCHDOG)
            .expect("worker wait gate exceeds 5 s watchdog")
    }

    fn release(&self) {
        self.release
            .send(())
            .expect("worker wait gate remains connected");
    }
}

struct GateWaiterParts {
    enabled: Arc<AtomicBool>,
    entered: Sender<WorkerWaitRequest>,
    release: Receiver<()>,
}

impl GateWaiterParts {
    fn with_receiver(self, receiver: Receiver<()>) -> GateWaiter {
        GateWaiter {
            inner: ChannelWorkerWaiter::new(receiver),
            enabled: self.enabled,
            entered: self.entered,
            release: self.release,
        }
    }
}

struct GateWaiter {
    inner: ChannelWorkerWaiter,
    enabled: Arc<AtomicBool>,
    entered: Sender<WorkerWaitRequest>,
    release: Receiver<()>,
}

impl WorkerWaiter for GateWaiter {
    fn wait(
        &mut self,
        request: WorkerWaitRequest,
        clock: &dyn MonotonicClock,
    ) -> Result<(), WorkerWaitError> {
        if self.enabled.load(Ordering::Acquire) {
            self.entered
                .send(request)
                .map_err(|_| WorkerWaitError::Disconnected)?;
            self.release
                .recv_timeout(DEADLOCK_WATCHDOG)
                .map_err(|_| WorkerWaitError::Disconnected)?;
        }
        self.inner.wait(request, clock)
    }
}

fn gate_channels(enabled: bool) -> (GateControl, GateWaiterParts) {
    let enabled = Arc::new(AtomicBool::new(enabled));
    let (entered, entered_receiver) = channel();
    let (release, release_receiver) = channel();
    (
        GateControl {
            enabled: Arc::clone(&enabled),
            entered: entered_receiver,
            release,
        },
        GateWaiterParts {
            enabled,
            entered,
            release: release_receiver,
        },
    )
}

fn spawn_direct_worker<W>(
    tuning: (usize, usize, usize),
    waiter: impl FnOnce(Receiver<()>) -> W,
) -> WorkerHarness<Direct>
where
    W: WorkerWaiter + 'static,
{
    let (activity, activity_receiver) = activity_channel();
    let (transport, control, telemetry) = measured_transport(activity.clone());
    let worker = crate::runtime::worker::WorkerCore::new_direct(
        protocol(),
        Box::new(transport),
        WorkerBudget::new(tuning.1, tuning.2),
        Box::new(|_| {}),
    );
    let (commands, command_receiver) =
        command_channel::<RuntimeCommand<Pro, Direct>>(tuning.0, activity.clone());
    let (shutdown, shutdown_receiver) = priority_shutdown_channel(activity.clone());
    let clock = InstantClock::new();
    let thread = spawn_worker_thread(
        worker,
        clock.clone(),
        shutdown_receiver,
        command_receiver,
        waiter(activity_receiver),
    )
    .expect("spawn Direct measurement worker");
    WorkerHarness {
        activity,
        clock,
        commands,
        shutdown,
        thread: Some(thread),
        control,
        telemetry,
    }
}

fn spawn_periodic_worker<W>(
    tuning: (usize, usize, usize),
    waiter: impl FnOnce(Receiver<()>) -> W,
) -> WorkerHarness<Periodic>
where
    W: WorkerWaiter + 'static,
{
    let (activity, activity_receiver) = activity_channel();
    let (transport, control, telemetry) = measured_transport(activity.clone());
    let worker = crate::runtime::worker::WorkerCore::new_periodic(
        protocol(),
        Box::new(transport),
        PERIOD,
        WorkerBudget::new(tuning.1, tuning.2),
        Box::new(|_| {}),
    )
    .expect("8 ms measurement period is valid");
    let (commands, command_receiver) =
        command_channel::<RuntimeCommand<Pro, Periodic>>(tuning.0, activity.clone());
    let (shutdown, shutdown_receiver) = priority_shutdown_channel(activity.clone());
    let clock = InstantClock::new();
    let thread = spawn_worker_thread(
        worker,
        clock.clone(),
        shutdown_receiver,
        command_receiver,
        waiter(activity_receiver),
    )
    .expect("spawn Periodic measurement worker");
    WorkerHarness {
        activity,
        clock,
        commands,
        shutdown,
        thread: Some(thread),
        control,
        telemetry,
    }
}

fn measured_transport(
    activity: ActivityNotifier,
) -> (MeasuredTransport, TestTransportControl, TransportTelemetry) {
    let (mut inner, control) = TestTransport::with_limits(FAKE_EVENT_CAPACITY, FAKE_POLL_BATCH);
    inner
        .open(activity)
        .expect("open TestTransport for measurement");
    let (send_observer, sends) = channel();
    let (poll_observer, polls) = channel();
    let poll_calls = Arc::new(AtomicU64::new(0));
    let poll_events = Arc::new(AtomicU64::new(0));
    let poll_errors = Arc::new(AtomicU64::new(0));
    (
        MeasuredTransport {
            inner,
            sends: send_observer,
            polls: poll_observer,
            poll_calls: Arc::clone(&poll_calls),
            poll_events: Arc::clone(&poll_events),
            poll_errors: Arc::clone(&poll_errors),
        },
        control,
        TransportTelemetry {
            sends,
            polls,
            poll_calls,
            poll_events,
            poll_errors,
        },
    )
}

fn pair_direct(harness: &WorkerHarness<Direct>) {
    let response = enqueue_pair(&harness.commands);
    inject_ready_events(&harness.control);
    wait_response(&response, "Direct pair");
}

fn pair_periodic(harness: &WorkerHarness<Periodic>) {
    let response = enqueue_pair(&harness.commands);
    inject_ready_events(&harness.control);
    wait_response(&response, "Periodic pair");
}

fn enqueue_pair<R>(commands: &CommandClient<RuntimeCommand<Pro, R>>) -> CommandResponse
where
    R: ReportingMode,
{
    commands
        .try_enqueue(RuntimeCommand::Pair {
            timeout: DEADLOCK_WATCHDOG,
        })
        .expect("enqueue measurement pair command")
}

fn inject_ready_events(control: &TestTransportControl) {
    control.inject_connected().expect("inject connected event");
    control
        .inject_hid_channel_opened(HidChannel::Control)
        .expect("inject control channel");
    control
        .inject_hid_channel_opened(HidChannel::Interrupt)
        .expect("inject interrupt channel");
    control
        .inject_hid_output(HidChannel::Control, &subcommand_report(0x03, &[0x30]))
        .expect("inject report-mode reply");
    control
        .inject_hid_output(HidChannel::Interrupt, &subcommand_report(0x30, &[0x01]))
        .expect("inject player-lights reply");
}

fn wait_response(response: &CommandResponse, label: &str) -> u64 {
    let started = Instant::now();
    let deadline = started + DEADLOCK_WATCHDOG;
    loop {
        match response.try_recv() {
            Ok(Ok(())) => return duration_ns(started.elapsed()),
            Ok(Err(error)) => panic!("{label} command failed: {error:?}"),
            Err(TryRecvError::Disconnected) => {
                panic!("{label} response disconnected before completion")
            }
            Err(TryRecvError::Empty) => {
                assert!(Instant::now() < deadline, "{label} exceeded 5 s watchdog");
                thread::yield_now();
            }
        }
    }
}

fn finish_worker(worker: WorkerThread) -> WorkerThreadOutcome {
    let deadline = Instant::now() + DEADLOCK_WATCHDOG;
    while !worker.is_finished() {
        assert!(
            Instant::now() < deadline,
            "worker completion exceeded 5 s watchdog"
        );
        thread::yield_now();
    }
    worker.finish()
}

fn assert_clean_close(outcome: WorkerThreadOutcome) {
    assert!(
        matches!(
            outcome,
            WorkerThreadOutcome::Closed {
                result: Ok(()),
                delivery_error: None,
            }
        ),
        "measurement worker must close cleanly: {outcome:?}"
    );
}

fn protocol() -> SwitchHidProtocol<Pro> {
    SwitchHidProtocol::new(None, DEVICE_INFO_ADDRESS)
}

fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut raw = vec![0x01, 0];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.push(subcommand_id);
    raw.extend_from_slice(payload);
    raw
}

fn measure_idle(config: &ProbeConfig, output: &Path, tuning: (usize, usize, usize)) -> Value {
    let (first_wait, first_wait_receiver) = sync_channel(1);
    let wait_counters = Arc::new(WaitCounters::default());
    let worker_wait_counters = Arc::clone(&wait_counters);
    let mut harness = spawn_direct_worker(tuning, |receiver| {
        FirstWaiter::new(receiver, first_wait, worker_wait_counters)
    });
    assert_eq!(
        first_wait_receiver
            .recv_timeout(DEADLOCK_WATCHDOG)
            .expect("Direct/Open worker reaches its idle wait"),
        WorkerWaitRequest::Activity
    );
    let output_dir = output
        .parent()
        .expect("measurement output must have a parent directory");
    let ready_path = output_dir.join("idle-ready");
    let start_path = output_dir.join("idle-start");
    let done_path = output_dir.join("idle-done");
    let cpu_path = output_dir.join("idle-cpu.json");
    write_marker(
        &ready_path,
        json!({
            "schema": "swbt.m2.idle-marker.v1",
            "phase": "ready",
            "pid": std::process::id(),
            "requested_idle_ns": duration_ns(config.idle),
        }),
    );
    wait_for_file(&start_path, "idle-start");

    let wait_entries_before = wait_counters.entries.load(Ordering::Acquire);
    let wait_returns_before = wait_counters.returns.load(Ordering::Acquire);
    let poll_calls_before = harness.telemetry.poll_calls.load(Ordering::Acquire);
    let poll_events_before = harness.telemetry.poll_events.load(Ordering::Acquire);
    let started = Instant::now();
    thread::sleep(config.idle);
    let elapsed = started.elapsed();
    let wait_entries_after = wait_counters.entries.load(Ordering::Acquire);
    let wait_returns_after = wait_counters.returns.load(Ordering::Acquire);
    let poll_calls_after = harness.telemetry.poll_calls.load(Ordering::Acquire);
    let poll_events_after = harness.telemetry.poll_events.load(Ordering::Acquire);
    write_marker(
        &done_path,
        json!({
            "schema": "swbt.m2.idle-marker.v1",
            "phase": "done",
            "pid": std::process::id(),
            "elapsed_ns": duration_ns(elapsed),
        }),
    );
    let cpu = wait_for_json(&cpu_path, "idle-cpu.json");
    let cpu_ticks = cpu.get("process_cpu_ticks_100ns").and_then(Value::as_u64);
    let cpu_wall_ns = cpu.get("wall_ns").and_then(Value::as_u64);
    let cpu_one_core_percent = cpu_ticks.zip(cpu_wall_ns).and_then(|(ticks, wall_ns)| {
        (wall_ns > 0).then(|| ticks as f64 * 100.0 / wall_ns as f64 * 100.0)
    });

    harness.request_shutdown();
    assert_clean_close(harness.finish());

    json!({
        "raw": {
            "requested_idle_ns": duration_ns(config.idle),
            "elapsed_ns": duration_ns(elapsed),
            "process_cpu_ticks_100ns": cpu_ticks,
            "process_cpu_wall_ns": cpu_wall_ns,
            "wait_entries_before": wait_entries_before,
            "wait_entries_delta": wait_entries_after.saturating_sub(wait_entries_before),
            "wait_returns_before": wait_returns_before,
            "wait_returns_delta": wait_returns_after.saturating_sub(wait_returns_before),
            "worker_loop_wake_delta": wait_returns_after.saturating_sub(wait_returns_before),
            "transport_poll_calls_delta": poll_calls_after.saturating_sub(poll_calls_before),
            "transport_poll_events_delta": poll_events_after.saturating_sub(poll_events_before),
        },
        "summary": {
            "elapsed_ns": duration_ns(elapsed),
            "process_cpu_ticks_100ns": cpu_ticks,
            "process_cpu_wall_ns": cpu_wall_ns,
            "cpu_percent_one_core": cpu_one_core_percent,
            "wait_entries_delta": wait_entries_after.saturating_sub(wait_entries_before),
            "wait_returns_delta": wait_returns_after.saturating_sub(wait_returns_before),
            "worker_loop_wake_delta": wait_returns_after.saturating_sub(wait_returns_before),
            "transport_poll_calls_delta": poll_calls_after.saturating_sub(poll_calls_before),
            "transport_poll_events_delta": poll_events_after.saturating_sub(poll_events_before),
        }
    })
}

fn write_marker(path: &Path, value: Value) {
    assert!(!path.exists(), "stale marker exists: {}", path.display());
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(&value).expect("serialize idle marker"),
    )
    .expect("write idle marker");
    fs::rename(temporary, path).expect("publish idle marker atomically");
}

fn wait_for_file(path: &Path, label: &str) {
    let deadline = Instant::now() + DEADLOCK_WATCHDOG;
    while !path.is_file() {
        assert!(Instant::now() < deadline, "{label} exceeded 5 s watchdog");
        thread::sleep(Duration::from_millis(5));
    }
}

fn wait_for_json(path: &Path, label: &str) -> Value {
    let deadline = Instant::now() + DEADLOCK_WATCHDOG;
    loop {
        if let Ok(bytes) = fs::read(path) {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                return value;
            }
        }
        assert!(Instant::now() < deadline, "{label} exceeded 5 s watchdog");
        thread::sleep(Duration::from_millis(5));
    }
}

fn measure_jitter(config: &ProbeConfig, tuning: (usize, usize, usize)) -> Value {
    let (deadline_sender, deadline_receiver) = channel();
    let mut harness = spawn_periodic_worker(tuning, |receiver| {
        DeadlineWaiter::new(receiver, deadline_sender)
    });
    pair_periodic(&harness);
    let warmup = next_correlated_jitter_sample(
        &deadline_receiver,
        &harness.telemetry.sends,
        &harness.clock,
        None,
        "Periodic jitter warm-up",
    );

    let mut samples = Vec::with_capacity(config.jitter_samples);
    let mut previous_accepted_at = warmup.1;
    for _ in 0..config.jitter_samples {
        let sample = next_correlated_jitter_sample(
            &deadline_receiver,
            &harness.telemetry.sends,
            &harness.clock,
            Some(previous_accepted_at),
            "Periodic jitter sample",
        );
        previous_accepted_at = sample.1;
        samples.push(sample);
    }
    harness.request_shutdown();
    assert_clean_close(harness.finish());

    let mut targets = Vec::with_capacity(config.jitter_samples);
    let mut wait_returns = Vec::with_capacity(config.jitter_samples);
    let mut actuals = Vec::with_capacity(config.jitter_samples);
    let mut lateness = Vec::with_capacity(config.jitter_samples);
    let mut intervals = Vec::with_capacity(config.jitter_samples.saturating_sub(1));
    let mut interval_errors = Vec::with_capacity(config.jitter_samples.saturating_sub(1));
    let mut interval_error_ppm = Vec::with_capacity(config.jitter_samples.saturating_sub(1));
    let mut skipped = 0_u64;
    let mut bursts = 0_u64;
    let mut previous = None;

    for (observation, actual) in samples {
        targets.push(duration_ns(observation.deadline));
        wait_returns.push(duration_ns(observation.returned_at));
        actuals.push(duration_ns(actual));
        lateness.push(duration_ns(actual.saturating_sub(observation.deadline)));
        if let Some(previous) = previous {
            let interval = actual.saturating_sub(previous);
            intervals.push(duration_ns(interval));
            interval_errors.push(signed_interval_error_ns(interval));
            interval_error_ppm.push(interval_error_ppm_from(interval));
            if interval < PERIOD / 2 {
                bursts = bursts.saturating_add(1);
            }
        }
        previous = Some(actual);
        let missed = actual.saturating_sub(observation.deadline).as_nanos() / PERIOD.as_nanos();
        skipped = skipped.saturating_add(u64::try_from(missed).unwrap_or(u64::MAX));
    }

    json!({
        "raw": {
            "period_ns": duration_ns(PERIOD),
            "target_ns": targets,
            "wait_return_ns": wait_returns,
            "actual_ns": actuals,
            "lateness_ns": lateness,
            "interval_ns": intervals,
            "interval_error_ns": interval_errors,
            "interval_error_ppm": interval_error_ppm,
        },
        "summary": {
            "lateness_ns": distribution(&lateness),
            "interval_ns": distribution(&intervals),
            "interval_error_ns": distribution_i64(&interval_errors),
            "interval_error_ppm": {
                "distribution": distribution_i64(&interval_error_ppm),
                "formula": "(interval_ns - period_ns) * 1000000 / period_ns",
                "basis": "period_ns",
                "ideal": 0,
            },
            "skipped_ticks": skipped,
            "burst_intervals_below_4ms": bursts,
        }
    })
}

fn next_correlated_jitter_sample(
    deadlines: &Receiver<DeadlineObservation>,
    sends: &Receiver<SendObservation>,
    clock: &InstantClock,
    previous_accepted_at: Option<Duration>,
    label: &str,
) -> (DeadlineObservation, Duration) {
    let watchdog = Instant::now() + DEADLOCK_WATCHDOG;
    let observation = loop {
        let remaining = watchdog.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "{label} deadline observation exceeds 5 s watchdog"
        );
        let observation = deadlines
            .recv_timeout(remaining)
            .unwrap_or_else(|_| panic!("{label} deadline observation exceeds 5 s watchdog"));
        assert!(
            observation.returned_at >= observation.deadline,
            "{label} wait return must reach its requested deadline"
        );
        if previous_accepted_at.is_none_or(|accepted_at| observation.deadline > accepted_at) {
            break observation;
        }
    };
    let accepted_at = next_input_send_at_or_after(sends, clock, observation.deadline, label);
    assert!(
        accepted_at >= observation.deadline,
        "{label} transport acceptance must not precede its correlated deadline"
    );
    assert!(
        observation.returned_at <= accepted_at,
        "{label} wait return must precede Periodic transport acceptance"
    );
    (observation, accepted_at)
}

fn next_input_send_at_or_after(
    sends: &Receiver<SendObservation>,
    clock: &InstantClock,
    not_before: Duration,
    label: &str,
) -> Duration {
    let watchdog = Instant::now() + DEADLOCK_WATCHDOG;
    loop {
        let remaining = watchdog.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "{label} send exceeds 5 s watchdog");
        match sends.recv_timeout(remaining) {
            Ok(observation) if observation.report_id == Some(0x30) && observation.accepted => {
                let accepted_at = clock.elapsed_at(observation.completed_at);
                if accepted_at >= not_before {
                    return accepted_at;
                }
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                panic!("{label} send exceeds 5 s watchdog")
            }
        }
    }
}

fn next_input_send(sends: &Receiver<SendObservation>, label: &str) -> Instant {
    let deadline = Instant::now() + DEADLOCK_WATCHDOG;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(!remaining.is_zero(), "{label} exceeded 5 s watchdog");
        match sends.recv_timeout(remaining) {
            Ok(observation) if observation.report_id == Some(0x30) && observation.accepted => {
                return observation.completed_at;
            }
            Ok(_) => {}
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                panic!("{label} exceeded 5 s watchdog")
            }
        }
    }
}

fn measure_command_latency(config: &ProbeConfig, tuning: (usize, usize, usize)) -> Value {
    let wait_entries = Arc::new(AtomicU64::new(0));
    let worker_wait_entries = Arc::clone(&wait_entries);
    let mut harness = spawn_direct_worker(tuning, |receiver| {
        ActivityEntryWaiter::new(receiver, worker_wait_entries)
    });
    pair_direct(&harness);
    while harness.telemetry.sends.try_recv().is_ok() {}
    let pair_wait_entries = wait_entries.load(Ordering::Acquire);
    let warmup = harness
        .commands
        .try_enqueue(RuntimeCommand::Input(DirectCommand::Send(
            InputState::neutral(),
        )))
        .expect("enqueue Direct command wait-entry warmup");
    wait_response(&warmup, "Direct command wait-entry warmup");
    wait_atomic_at_least(
        &wait_entries,
        pair_wait_entries.saturating_add(1),
        "Direct command post-warmup activity wait",
    );
    while harness.telemetry.sends.try_recv().is_ok() {}
    let mut observed_wait_entries = wait_entries.load(Ordering::Acquire);
    let mut enqueue = Vec::with_capacity(config.command_samples);
    let mut accepted = Vec::with_capacity(config.command_samples);
    let mut response = Vec::with_capacity(config.command_samples);

    for _ in 0..config.command_samples {
        wait_atomic_at_least(
            &wait_entries,
            observed_wait_entries,
            "Direct command pre-sample activity wait",
        );
        let submitted = Instant::now();
        let command_response = harness
            .commands
            .try_enqueue(RuntimeCommand::Input(DirectCommand::Send(
                InputState::neutral(),
            )))
            .expect("sequential Direct command enqueue fits");
        enqueue.push(duration_ns(submitted.elapsed()));
        wait_response(&command_response, "Direct ready command");
        let response_elapsed = duration_ns(submitted.elapsed());
        let accepted_at = next_input_send(
            &harness.telemetry.sends,
            "Direct command transport acceptance",
        );
        accepted.push(duration_ns(
            accepted_at.saturating_duration_since(submitted),
        ));
        response.push(response_elapsed);
        wait_atomic_at_least(
            &wait_entries,
            observed_wait_entries.saturating_add(1),
            "Direct command post-sample activity wait",
        );
        observed_wait_entries = wait_entries.load(Ordering::Acquire);
    }
    harness.request_shutdown();
    assert_clean_close(harness.finish());

    json!({
        "raw": {
            "enqueue_ns": enqueue,
            "transport_accept_ns": accepted,
            "response_ns": response,
        },
        "summary": {
            "enqueue_ns": distribution(&enqueue),
            "transport_accept_ns": distribution(&accepted),
            "response_ns": distribution(&response),
        }
    })
}

fn measure_transport_latency(config: &ProbeConfig, tuning: (usize, usize, usize)) -> Value {
    let wait_entries = Arc::new(AtomicU64::new(0));
    let worker_wait_entries = Arc::clone(&wait_entries);
    let mut harness = spawn_direct_worker(tuning, |receiver| {
        ActivityEntryWaiter::new(receiver, worker_wait_entries)
    });
    wait_atomic_at_least(
        &wait_entries,
        1,
        "Direct/Open transport initial activity wait",
    );
    let mut observed_wait_entries = wait_entries.load(Ordering::Acquire);
    let mut injected = Vec::with_capacity(config.transport_samples);
    let mut drained_latency = Vec::with_capacity(config.transport_samples);

    for _ in 0..config.transport_samples {
        let started = Instant::now();
        harness
            .control
            .inject_connected()
            .expect("inject one transport event");
        injected.push(duration_ns(started.elapsed()));
        let observation = harness
            .telemetry
            .polls
            .recv_timeout(DEADLOCK_WATCHDOG)
            .expect("Direct/Open transport poll exceeds 5 s watchdog");
        assert_eq!(
            observation.events, 1,
            "sequential transport probe drains one event"
        );
        drained_latency.push(duration_ns(
            observation.observed_at.saturating_duration_since(started),
        ));
        wait_atomic_at_least(
            &wait_entries,
            observed_wait_entries.saturating_add(1),
            "Direct/Open transport post-sample activity wait",
        );
        observed_wait_entries = wait_entries.load(Ordering::Acquire);
    }
    let overflow_count = harness.telemetry.poll_errors.load(Ordering::Acquire);
    harness.request_shutdown();
    assert_clean_close(harness.finish());

    json!({
        "raw": {
            "inject_return_ns": injected,
            "drained_ns": drained_latency,
        },
        "summary": {
            "inject_return_ns": distribution(&injected),
            "drained_ns": distribution(&drained_latency),
            "overflow_count": overflow_count,
        }
    })
}

fn measure_shutdown_latency(config: &ProbeConfig, tuning: (usize, usize, usize)) -> Value {
    let mut idle = Vec::with_capacity(config.shutdown_samples);
    let mut saturated = Vec::with_capacity(config.shutdown_samples);
    for _ in 0..config.shutdown_samples {
        idle.push(one_idle_shutdown_sample(tuning));
        saturated.push(one_saturated_shutdown_sample(tuning));
    }
    json!({
        "raw": {
            "idle_ns": idle,
            "saturated_ns": saturated,
        },
        "summary": {
            "idle_ns": distribution(&idle),
            "saturated_ns": distribution(&saturated),
        }
    })
}

fn one_idle_shutdown_sample(tuning: (usize, usize, usize)) -> u64 {
    let (first_wait, first_wait_receiver) = sync_channel(1);
    let counters = Arc::new(WaitCounters::default());
    let mut harness = spawn_direct_worker(tuning, |receiver| {
        FirstWaiter::new(receiver, first_wait, counters)
    });
    first_wait_receiver
        .recv_timeout(DEADLOCK_WATCHDOG)
        .expect("idle shutdown worker reaches activity wait");
    let started = Instant::now();
    harness.request_shutdown();
    assert_clean_close(harness.finish());
    duration_ns(started.elapsed())
}

fn one_saturated_shutdown_sample(tuning: (usize, usize, usize)) -> u64 {
    let (wait_entries, wait_entry_receiver) = sync_channel(0);
    let mut harness = spawn_direct_worker(tuning, move |receiver| {
        RequestEntryWaiter::new(receiver, wait_entries)
    });
    assert_eq!(
        next_wait_entry(&wait_entry_receiver, "saturated shutdown initial wait"),
        WorkerWaitRequest::Activity,
        "Direct/Open worker reaches its initial idle wait"
    );
    let pair = enqueue_pair(&harness.commands);
    assert!(matches!(
        next_wait_entry(&wait_entry_receiver, "pending pair wait"),
        WorkerWaitRequest::ActivityOrDeadline(_)
    ));

    let mut queued = Vec::with_capacity(tuning.0);
    for _ in 0..tuning.0 {
        queued.push(
            harness
                .commands
                .try_enqueue(RuntimeCommand::Input(DirectCommand::Send(
                    InputState::neutral(),
                )))
                .expect("fill production command queue while worker is gated"),
        );
    }
    assert!(matches!(
        harness
            .commands
            .try_enqueue(RuntimeCommand::Input(DirectCommand::Send(
                InputState::neutral(),
            ))),
        Err(CommandEnqueueError::Busy)
    ));
    assert!(matches!(
        next_wait_entry(&wait_entry_receiver, "full queue wait"),
        WorkerWaitRequest::ActivityOrDeadline(_)
    ));
    harness.activity.notify();
    assert!(matches!(
        next_wait_entry(&wait_entry_receiver, "quiescent full queue wait"),
        WorkerWaitRequest::ActivityOrDeadline(_)
    ));

    let started = Instant::now();
    harness.request_shutdown();
    assert_clean_close(harness.finish());
    let elapsed = duration_ns(started.elapsed());
    assert!(
        matches!(pair.try_recv(), Ok(Err(WorkerCommandError::Shutdown))),
        "in-flight Pair must receive the typed shutdown result"
    );
    assert!(
        queued
            .iter()
            .all(|response| matches!(response.try_recv(), Err(TryRecvError::Disconnected))),
        "priority shutdown must win before saturated commands are processed"
    );
    elapsed
}

fn next_wait_entry(entries: &Receiver<WorkerWaitRequest>, label: &str) -> WorkerWaitRequest {
    entries
        .recv_timeout(DEADLOCK_WATCHDOG)
        .unwrap_or_else(|_| panic!("{label} exceeds 5 s watchdog"))
}

fn measure_fairness(config: &ProbeConfig, tuning: (usize, usize, usize)) -> Value {
    let (gate, waiter) = gate_channels(false);
    let mut harness = spawn_periodic_worker(tuning, move |receiver| waiter.with_receiver(receiver));
    pair_periodic(&harness);
    while harness.telemetry.sends.try_recv().is_ok() {}
    while harness.telemetry.polls.try_recv().is_ok() {}
    gate.enable();
    harness.activity.notify();

    let mut targets = Vec::with_capacity(config.fairness_ticks);
    let mut releases = Vec::with_capacity(config.fairness_ticks);
    let mut staging = Vec::with_capacity(config.fairness_ticks);
    let mut actuals = Vec::with_capacity(config.fairness_ticks);
    let mut post_release_accept = Vec::with_capacity(config.fairness_ticks);
    let mut lateness = Vec::with_capacity(config.fairness_ticks);
    let mut intervals = Vec::with_capacity(config.fairness_ticks.saturating_sub(1));
    let mut interval_errors = Vec::with_capacity(config.fairness_ticks.saturating_sub(1));
    let mut interval_error_ppm = Vec::with_capacity(config.fairness_ticks.saturating_sub(1));
    let mut command_response = Vec::with_capacity(config.fairness_ticks.saturating_mul(tuning.0));
    let mut post_release_command =
        Vec::with_capacity(config.fairness_ticks.saturating_mul(tuning.0));
    let mut reply_attempt =
        Vec::with_capacity(config.fairness_ticks.saturating_mul(FAKE_EVENT_CAPACITY));
    let mut post_release_reply =
        Vec::with_capacity(config.fairness_ticks.saturating_mul(FAKE_EVENT_CAPACITY));
    let mut commands_per_tick = Vec::with_capacity(config.fairness_ticks);
    let mut transport_per_tick = Vec::with_capacity(config.fairness_ticks);
    let mut skipped = 0_u64;
    let mut bursts = 0_u64;
    let mut previous = None;
    let mut busy = 0_u64;

    for _ in 0..config.fairness_ticks {
        let target = loop {
            match gate.next() {
                WorkerWaitRequest::ActivityOrDeadline(deadline) => break deadline,
                WorkerWaitRequest::Activity => gate.release(),
            }
        };
        while harness.telemetry.sends.try_recv().is_ok() {}
        sleep_until(&harness.clock, target);

        let mut commands = Vec::with_capacity(tuning.0);
        for _ in 0..tuning.0 {
            let submitted = Instant::now();
            let response = harness
                .commands
                .try_enqueue(RuntimeCommand::Input(PeriodicCommand::Apply(
                    InputState::neutral(),
                )))
                .expect("fill fairness command capacity while worker is gated");
            commands.push((submitted, response));
        }
        assert!(matches!(
            harness
                .commands
                .try_enqueue(RuntimeCommand::Input(PeriodicCommand::Apply(
                    InputState::neutral(),
                ))),
            Err(CommandEnqueueError::Busy)
        ));
        busy = busy.saturating_add(1);

        let poll_events_before = harness.telemetry.poll_events.load(Ordering::Acquire);
        harness.control.reject_next_sends(FAKE_EVENT_CAPACITY);
        let mut injected_at = Vec::with_capacity(FAKE_EVENT_CAPACITY);
        for _ in 0..FAKE_EVENT_CAPACITY {
            injected_at.push(Instant::now());
            harness
                .control
                .inject_hid_output(HidChannel::Control, &subcommand_report(0x30, &[0x01]))
                .expect("fill fairness TestTransport with valid HID output");
        }

        let release_requested = Instant::now();
        let release_at = harness.clock.elapsed_at(release_requested);
        gate.release();
        let (accepted, reply_attempts) =
            collect_fairness_sends(&harness.telemetry.sends, FAKE_EVENT_CAPACITY);
        let actual = harness.clock.elapsed_at(accepted);
        for (injected, attempted) in injected_at.into_iter().zip(reply_attempts) {
            reply_attempt.push(duration_ns(attempted.saturating_duration_since(injected)));
            post_release_reply.push(duration_ns(
                attempted.saturating_duration_since(release_requested),
            ));
        }
        wait_atomic_at_least(
            &harness.telemetry.poll_events,
            poll_events_before.saturating_add(FAKE_EVENT_CAPACITY as u64),
            "fairness transport drain",
        );
        let mut completed = 0_u64;
        for (submitted, response) in &commands {
            wait_response(response, "fairness command reply");
            command_response.push(duration_ns(submitted.elapsed()));
            post_release_command.push(duration_ns(release_requested.elapsed()));
            completed = completed.saturating_add(1);
        }
        commands_per_tick.push(completed);
        transport_per_tick.push(
            harness
                .telemetry
                .poll_events
                .load(Ordering::Acquire)
                .saturating_sub(poll_events_before),
        );

        targets.push(duration_ns(target));
        releases.push(duration_ns(release_at));
        staging.push(duration_ns(release_at.saturating_sub(target)));
        actuals.push(duration_ns(actual));
        post_release_accept.push(duration_ns(actual.saturating_sub(release_at)));
        lateness.push(duration_ns(actual.saturating_sub(target)));
        if let Some(previous) = previous {
            let interval = actual.saturating_sub(previous);
            intervals.push(duration_ns(interval));
            interval_errors.push(signed_interval_error_ns(interval));
            interval_error_ppm.push(interval_error_ppm_from(interval));
            if interval < PERIOD / 2 {
                bursts = bursts.saturating_add(1);
            }
        }
        previous = Some(actual);
        let missed = actual.saturating_sub(target).as_nanos() / PERIOD.as_nanos();
        skipped = skipped.saturating_add(u64::try_from(missed).unwrap_or(u64::MAX));
    }

    let transport_errors = harness.telemetry.poll_errors.load(Ordering::Acquire);
    let _shutdown_gate = gate.next();
    harness.request_shutdown();
    gate.release();
    assert_clean_close(harness.finish());
    let commands_completed = commands_per_tick.iter().copied().sum::<u64>();
    let transport_events_drained = transport_per_tick.iter().copied().sum::<u64>();

    json!({
        "raw": {
            "period_ns": duration_ns(PERIOD),
            "target_ns": targets,
            "gate_release_requested_ns": releases,
            "staging_ns": staging,
            "actual_ns": actuals,
            "post_release_accept_ns": post_release_accept,
            "lateness_ns": lateness,
            "interval_ns": intervals,
            "interval_error_ns": interval_errors,
            "interval_error_ppm": interval_error_ppm,
            "command_response_ns": command_response,
            "post_release_command_completion_observed_ns": post_release_command,
            "reply_attempt_ns": reply_attempt,
            "post_release_reply_attempt_ns": post_release_reply,
            "commands_completed_per_tick": commands_per_tick,
            "transport_events_per_tick": transport_per_tick,
        },
        "summary": {
            "latency_scope": {
                "staging": "target deadline through 16 command and 64 event placement while the worker gate is held",
                "post_release": "gate release request through worker observation",
                "command_completion": "caller observation after all periodic and reply send observations; conservative upper bound",
            },
            "staging_ns": distribution(&staging),
            "post_release_accept_ns": distribution(&post_release_accept),
            "lateness_ns": distribution(&lateness),
            "interval_ns": distribution(&intervals),
            "interval_error_ns": distribution_i64(&interval_errors),
            "interval_error_ppm": {
                "distribution": distribution_i64(&interval_error_ppm),
                "formula": "(interval_ns - period_ns) * 1000000 / period_ns",
                "basis": "period_ns",
                "ideal": 0,
            },
            "command_response_ns": distribution(&command_response),
            "post_release_command_completion_observed_ns":
                distribution(&post_release_command),
            "reply_attempt_ns": distribution(&reply_attempt),
            "post_release_reply_attempt_ns": distribution(&post_release_reply),
            "reply_boundary": "send_attempt",
            "reply_outcome": "rejected_for_fairness_probe",
            "skipped_ticks": skipped,
            "burst_intervals_below_4ms": bursts,
            "commands_completed": commands_completed,
            "command_busy": busy,
            "transport_events_drained": transport_events_drained,
            "transport_errors": transport_errors,
        }
    })
}

fn collect_fairness_sends(
    sends: &Receiver<SendObservation>,
    expected_replies: usize,
) -> (Instant, Vec<Instant>) {
    let deadline = Instant::now() + DEADLOCK_WATCHDOG;
    let mut periodic = None;
    let mut replies = Vec::with_capacity(expected_replies);
    while periodic.is_none() || replies.len() < expected_replies {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "fairness send observations exceeded 5 s watchdog"
        );
        let observation = sends
            .recv_timeout(remaining)
            .expect("fairness send observations exceeded 5 s watchdog");
        match observation.report_id {
            Some(0x21) => {
                assert!(
                    !observation.accepted,
                    "fairness replies must be rejected to avoid the 300 ms holdoff"
                );
                replies.push(observation.completed_at);
            }
            Some(0x30) if observation.accepted => {
                periodic.get_or_insert(observation.completed_at);
            }
            _ => {}
        }
    }
    (
        periodic.expect("fairness Periodic report is accepted"),
        replies,
    )
}

fn sleep_until(clock: &InstantClock, deadline: Duration) {
    let watchdog = Instant::now() + DEADLOCK_WATCHDOG;
    loop {
        let now = clock.now();
        if now >= deadline {
            return;
        }
        assert!(
            Instant::now() < watchdog,
            "fairness deadline exceeds 5 s watchdog"
        );
        thread::sleep((deadline - now).min(Duration::from_millis(1)));
    }
}

fn wait_atomic_at_least(counter: &AtomicU64, expected: u64, label: &str) {
    let deadline = Instant::now() + DEADLOCK_WATCHDOG;
    while counter.load(Ordering::Acquire) < expected {
        assert!(Instant::now() < deadline, "{label} exceeded 5 s watchdog");
        thread::yield_now();
    }
}

fn measurement_meta(config: &ProbeConfig, tuning: (usize, usize, usize)) -> Value {
    json!({
        "git_sha": env::var("SWBT_MEASUREMENT_GIT_SHA").unwrap_or_else(|_| "unrecorded".to_owned()),
        "profile": env::var("SWBT_MEASUREMENT_PROFILE")
            .unwrap_or_else(|_| "unrecorded".to_owned()),
        "feature_set": env::var("SWBT_MEASUREMENT_FEATURE_SET")
            .unwrap_or_else(|_| "unrecorded".to_owned()),
        "mode": env::var("SWBT_MEASUREMENT_MODE")
            .unwrap_or_else(|_| "unrecorded".to_owned()),
        "worktree_dirty": env_bool("SWBT_MEASUREMENT_WORKTREE_DIRTY"),
        "cfg_test": true,
        "raw_format": "ndjson-one-sample-per-line",
        "interval_error_ppm": {
            "formula": "(interval_ns - period_ns) * 1000000 / period_ns",
            "basis": "period_ns",
            "ideal": 0,
        },
        "host": {
            "os": env::var("SWBT_MEASUREMENT_OS").unwrap_or_else(|_| env::consts::OS.to_owned()),
            "arch": env::consts::ARCH,
            "cpu": env::var("SWBT_MEASUREMENT_CPU").unwrap_or_else(|_| "unrecorded".to_owned()),
            "logical_cpus": env::var("SWBT_MEASUREMENT_LOGICAL_CPUS")
                .ok()
                .and_then(|value| value.parse::<u64>().ok()),
        },
        "toolchain": {
            "rustc": command_version("rustc"),
            "cargo": command_version("cargo"),
        },
        "tuning": {
            "command_capacity": tuning.0,
            "command_batch": tuning.1,
            "poll_batches": tuning.2,
            "fake_event_capacity": FAKE_EVENT_CAPACITY,
            "fake_poll_batch": FAKE_POLL_BATCH,
            "fairness_reply_boundary": "send_attempt",
            "fairness_reply_outcome": "rejected_for_fairness_probe",
            "fairness_staging":
                "after target deadline, place one configured command and event batch while the worker gate is held",
            "fairness_release_boundary": "gate release request",
            "fairness_command_completion_boundary":
                "caller observation after periodic and reply send observations; conservative upper bound",
        },
        "samples": {
            "idle_ms": config.idle.as_millis(),
            "jitter": config.jitter_samples,
            "command": config.command_samples,
            "transport": config.transport_samples,
            "shutdown_each": config.shutdown_samples,
            "fairness_ticks": config.fairness_ticks,
        },
        "measured_layers": [
            "ActivityNotifier coalescing channel",
            "ChannelWorkerWaiter deadline and activity waits",
            "WorkerCore step and spawned production worker loop",
            "protocol input and subcommand-reply preparation",
            "typed bounded command queue and one-shot response",
            "priority explicit shutdown, cleanup, completion, and join",
            "TestTransport activity notification and non-blocking drain",
            "Periodic real-ready accepted 8 ms sends",
            "absolute Periodic deadlines under production 16/16/4 work budgets",
        ],
        "unmeasured_layers": [
            "Controller, ReadyRuntime, and WorkerOwner wrapper overhead",
            "Bumble backend and USB/HCI driver latency",
            "Bluetooth air delivery and Switch acknowledgement",
            "hardware long-run jitter and power management",
        ],
    })
}

fn distribution(samples: &[u64]) -> Value {
    if samples.is_empty() {
        return json!({
            "count": 0,
            "p50": null,
            "p95": null,
            "p99": null,
            "max": null,
        });
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    json!({
        "count": sorted.len(),
        "p50": nearest_rank(&sorted, 50),
        "p95": nearest_rank(&sorted, 95),
        "p99": nearest_rank(&sorted, 99),
        "max": sorted.last().copied(),
    })
}

fn distribution_i64(samples: &[i64]) -> Value {
    if samples.is_empty() {
        return json!({
            "count": 0,
            "p50": null,
            "p95": null,
            "p99": null,
            "max": null,
        });
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    json!({
        "count": sorted.len(),
        "p50": nearest_rank_i64(&sorted, 50),
        "p95": nearest_rank_i64(&sorted, 95),
        "p99": nearest_rank_i64(&sorted, 99),
        "max": sorted.last().copied(),
    })
}

fn nearest_rank(sorted: &[u64], percentile: usize) -> u64 {
    let rank = sorted.len().saturating_mul(percentile).saturating_add(99) / 100;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn nearest_rank_i64(sorted: &[i64], percentile: usize) -> i64 {
    let rank = sorted.len().saturating_mul(percentile).saturating_add(99) / 100;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

fn signed_interval_error_ns(interval: Duration) -> i64 {
    let difference = interval.as_nanos() as i128 - PERIOD.as_nanos() as i128;
    i64::try_from(difference).unwrap_or_else(|_| {
        if difference.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

fn interval_error_ppm_from(interval: Duration) -> i64 {
    let error = i128::from(signed_interval_error_ns(interval));
    let ratio = error.saturating_mul(1_000_000) / PERIOD.as_nanos() as i128;
    i64::try_from(ratio).unwrap_or_else(|_| {
        if ratio.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    })
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn env_usize(name: &str, default: usize) -> usize {
    let value = env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("{name} must be usize"))
        })
        .unwrap_or(default);
    assert!(value > 0, "{name} must be positive");
    value
}

fn env_u64(name: &str, default: u64) -> u64 {
    let value = env::var(name)
        .ok()
        .map(|value| {
            value
                .parse::<u64>()
                .unwrap_or_else(|_| panic!("{name} must be u64"))
        })
        .unwrap_or(default);
    assert!(value > 0, "{name} must be positive");
    value
}

fn env_bool(name: &str) -> Option<bool> {
    env::var(name).ok().map(|value| match value.as_str() {
        "true" => true,
        "false" => false,
        _ => panic!("{name} must be true or false"),
    })
}

fn command_version(program: &str) -> String {
    Command::new(program)
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unavailable".to_owned())
}
