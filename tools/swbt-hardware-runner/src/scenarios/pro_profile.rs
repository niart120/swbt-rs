use std::{
    fmt, fs,
    fs::OpenOptions,
    io::{self, Write},
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use swbt::{
    DirectProController, Error, ErrorKind, GamepadStatus, LifecycleState, ProButton, ProController,
    ProInputState, ReportingKind, Stick,
};

const EVIDENCE_SCHEMA: &str = "swbt.m6.pro-profile";
const EVIDENCE_SCHEMA_VERSION: u64 = 1;
const BUTTON_HOLD: Duration = Duration::from_millis(500);
const STICK_HOLD: Duration = Duration::from_millis(500);
const DIRECT_IDLE: Duration = Duration::from_millis(500);
const MIN_CONNECT_TIMEOUT_SECS: u64 = 1;
const MAX_CONNECT_TIMEOUT_SECS: u64 = 600;
const MIN_RUN_INDEX: u8 = 1;
const MAX_RUN_INDEX: u8 = 99;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunnerMode {
    Periodic,
    Direct,
}

impl RunnerMode {
    const fn name(self) -> &'static str {
        match self {
            Self::Periodic => "periodic",
            Self::Direct => "direct",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperatorSetup {
    Normal,
    PostPowerCycle,
    StaleBond,
}

impl OperatorSetup {
    const fn name(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::PostPowerCycle => "post_power_cycle",
            Self::StaleBond => "stale_bond",
        }
    }

    const fn expects_connection(self) -> bool {
        !matches!(self, Self::StaleBond)
    }
}

struct RunnerArgs {
    adapter: String,
    profile: PathBuf,
    stale_source_profile: Option<PathBuf>,
    mode: RunnerMode,
    setup: OperatorSetup,
    connect_timeout: Duration,
    run_index: u8,
}

impl fmt::Debug for RunnerArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunnerArgs")
            .field("adapter", &"<redacted>")
            .field("profile", &"<redacted>")
            .field(
                "stale_source_profile",
                &self.stale_source_profile.as_ref().map(|_| "<redacted>"),
            )
            .field("mode", &self.mode)
            .field("setup", &self.setup)
            .field("connect_timeout", &self.connect_timeout)
            .field("run_index", &self.run_index)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UsageError(&'static str);

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for UsageError {}

pub(crate) fn run(arguments: Vec<String>) -> u8 {
    let args = match parse_args(arguments) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: swbt-hardware-runner pro-profile --adapter <selector> --profile <path> \
                 --mode <periodic|direct> --setup <normal|post-power-cycle|stale-bond> \
                 --connect-timeout-secs <1..600> --run <1..99> \
                 [--stale-source-profile <existing-path>]"
            );
            return 2;
        }
    };

    let success = run_hardware(&args);
    u8::from(!success)
}

fn parse_args<I, S>(args: I) -> Result<RunnerArgs, UsageError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut adapter = None;
    let mut profile = None;
    let mut stale_source_profile = None;
    let mut mode = None;
    let mut setup = None;
    let mut connect_timeout_secs = None;
    let mut run_index = None;
    let mut args = args.into_iter().map(Into::into);

    while let Some(flag) = args.next() {
        let value = args.next().ok_or(UsageError("argument value is missing"))?;
        match flag.as_str() {
            "--adapter" if adapter.is_none() => adapter = Some(value),
            "--profile" if profile.is_none() => profile = Some(PathBuf::from(value)),
            "--stale-source-profile" if stale_source_profile.is_none() => {
                stale_source_profile = Some(PathBuf::from(value));
            }
            "--mode" if mode.is_none() => {
                mode = Some(match value.as_str() {
                    "periodic" => RunnerMode::Periodic,
                    "direct" => RunnerMode::Direct,
                    _ => return Err(UsageError("invalid --mode")),
                });
            }
            "--setup" if setup.is_none() => {
                setup = Some(match value.as_str() {
                    "normal" => OperatorSetup::Normal,
                    "post-power-cycle" => OperatorSetup::PostPowerCycle,
                    "stale-bond" => OperatorSetup::StaleBond,
                    _ => return Err(UsageError("invalid --setup")),
                });
            }
            "--connect-timeout-secs" if connect_timeout_secs.is_none() => {
                connect_timeout_secs = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| UsageError("invalid --connect-timeout-secs"))?,
                );
            }
            "--run" if run_index.is_none() => {
                run_index = Some(
                    value
                        .parse::<u8>()
                        .map_err(|_| UsageError("invalid --run"))?,
                );
            }
            "--adapter"
            | "--profile"
            | "--stale-source-profile"
            | "--mode"
            | "--setup"
            | "--connect-timeout-secs"
            | "--run" => return Err(UsageError("duplicate argument")),
            _ => return Err(UsageError("unknown argument")),
        }
    }

    let adapter = adapter.ok_or(UsageError("missing --adapter"))?;
    if adapter.is_empty() {
        return Err(UsageError("invalid --adapter"));
    }
    let profile = profile.ok_or(UsageError("missing --profile"))?;
    if profile.as_os_str().is_empty() {
        return Err(UsageError("invalid --profile"));
    }
    let mode = mode.ok_or(UsageError("missing --mode"))?;
    let setup = setup.ok_or(UsageError("missing --setup"))?;
    let connect_timeout_secs =
        connect_timeout_secs.ok_or(UsageError("missing --connect-timeout-secs"))?;
    if !(MIN_CONNECT_TIMEOUT_SECS..=MAX_CONNECT_TIMEOUT_SECS).contains(&connect_timeout_secs) {
        return Err(UsageError("invalid --connect-timeout-secs"));
    }
    let run_index = run_index.ok_or(UsageError("missing --run"))?;
    if !(MIN_RUN_INDEX..=MAX_RUN_INDEX).contains(&run_index) {
        return Err(UsageError("invalid --run"));
    }
    match (setup, stale_source_profile.as_ref()) {
        (OperatorSetup::StaleBond, None) => {
            return Err(UsageError(
                "stale-bond setup requires --stale-source-profile",
            ));
        }
        (OperatorSetup::StaleBond, Some(source)) if source == &profile => {
            return Err(UsageError(
                "stale-bond source and target profiles must differ",
            ));
        }
        (OperatorSetup::Normal | OperatorSetup::PostPowerCycle, Some(_)) => {
            return Err(UsageError(
                "--stale-source-profile requires stale-bond setup",
            ));
        }
        _ => {}
    }

    Ok(RunnerArgs {
        adapter,
        profile,
        stale_source_profile,
        mode,
        setup,
        connect_timeout: Duration::from_secs(connect_timeout_secs),
        run_index,
    })
}

struct ProfileBaseline {
    bytes: Vec<u8>,
    stale_source: Option<(PathBuf, Vec<u8>)>,
}

fn prepare_profile(args: &RunnerArgs, started: Instant) -> Result<ProfileBaseline, &'static str> {
    let baseline = if args.setup == OperatorSetup::StaleBond {
        let source_path = args
            .stale_source_profile
            .as_ref()
            .expect("validated stale-bond source");
        let source = fs::read(source_path).map_err(|_| "stale_source_read")?;
        let stale = stale_profile_bytes(&source)?;
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&args.profile)
            .map_err(|_| "stale_target_create")?;
        target.write_all(&stale).map_err(|_| "stale_target_write")?;
        target.sync_all().map_err(|_| "stale_target_sync")?;
        ProfileBaseline {
            bytes: stale,
            stale_source: Some((source_path.clone(), source)),
        }
    } else {
        ProfileBaseline {
            bytes: fs::read(&args.profile).map_err(|_| "profile_read")?,
            stale_source: None,
        }
    };
    emit(
        args,
        started,
        "profile_preflight",
        [
            ("profile_exists", json!(true)),
            ("profile_size_bytes", json!(baseline.bytes.len())),
            ("raw_profile_emitted", json!(false)),
            ("key_material_emitted", json!(false)),
            (
                "stale_copy_created",
                json!(args.setup == OperatorSetup::StaleBond),
            ),
        ],
    );
    Ok(baseline)
}

fn stale_profile_bytes(source: &[u8]) -> Result<Vec<u8>, &'static str> {
    let mut profile: Value =
        serde_json::from_slice(source).map_err(|_| "stale_source_invalid_json")?;
    let namespaces = profile
        .get_mut("key_store")
        .and_then(|value| value.get_mut("namespaces"))
        .and_then(Value::as_object_mut)
        .ok_or("stale_source_invalid_namespaces")?;
    if namespaces.len() != 1 {
        return Err("stale_source_requires_one_namespace");
    }
    let peers = namespaces
        .values_mut()
        .next()
        .and_then(Value::as_object_mut)
        .ok_or("stale_source_invalid_peer_map")?;
    if peers.len() != 1 {
        return Err("stale_source_requires_one_peer");
    }
    let link_key = peers
        .values_mut()
        .next()
        .and_then(|value| value.get_mut("link_key"))
        .and_then(|value| value.get_mut("value"))
        .and_then(|value| value.as_str())
        .ok_or("stale_source_missing_link_key")?;
    if link_key.len() != 32 || !link_key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("stale_source_invalid_link_key");
    }
    let replacement = if link_key.starts_with('0') { '1' } else { '0' };
    let mut changed = link_key.to_owned();
    changed.replace_range(..1, &replacement.to_string());
    *peers
        .values_mut()
        .next()
        .and_then(|value| value.get_mut("link_key"))
        .and_then(|value| value.get_mut("value"))
        .expect("validated link-key location") = Value::String(changed);

    let mut bytes = serde_json::to_vec_pretty(&profile).map_err(|_| "stale_target_serialize")?;
    bytes.push(b'\n');
    Ok(bytes)
}

enum ProHardwareController {
    Periodic(ProController),
    Direct(DirectProController),
}

impl ProHardwareController {
    fn build(args: &RunnerArgs) -> swbt::Result<Self> {
        match args.mode {
            RunnerMode::Periodic => ProController::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .build()
                .map(Self::Periodic),
            RunnerMode::Direct => DirectProController::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .build()
                .map(Self::Direct),
        }
    }

    fn open(&mut self) -> swbt::Result<()> {
        match self {
            Self::Periodic(controller) => controller.open(),
            Self::Direct(controller) => controller.open(),
        }
    }

    fn reconnect(&mut self, timeout: Duration) -> swbt::Result<()> {
        match self {
            Self::Periodic(controller) => controller.reconnect(timeout),
            Self::Direct(controller) => controller.reconnect(timeout),
        }
    }

    fn tap(&mut self, buttons: &[ProButton], duration: Duration) -> swbt::Result<()> {
        match self {
            Self::Periodic(controller) => controller.tap(buttons.iter().copied(), duration),
            Self::Direct(controller) => controller.tap(buttons.iter().copied(), duration),
        }
    }

    fn send_state(&mut self, state: ProInputState) -> swbt::Result<()> {
        match self {
            Self::Periodic(controller) => controller.apply(state),
            Self::Direct(controller) => controller.send(state),
        }
    }

    fn neutral(&mut self) -> swbt::Result<()> {
        match self {
            Self::Periodic(controller) => controller.neutral(),
            Self::Direct(controller) => controller.neutral(),
        }
    }

    fn close(&mut self) -> swbt::Result<()> {
        match self {
            Self::Periodic(controller) => controller.close(),
            Self::Direct(controller) => controller.close(),
        }
    }

    fn close_without_neutral(&mut self) -> swbt::Result<()> {
        match self {
            Self::Periodic(controller) => controller.close_without_neutral(),
            Self::Direct(controller) => controller.close_without_neutral(),
        }
    }

    fn status(&self) -> GamepadStatus {
        match self {
            Self::Periodic(controller) => controller.status(),
            Self::Direct(controller) => controller.status(),
        }
    }

    fn snapshot(&self) -> ProInputState {
        match self {
            Self::Periodic(controller) => controller.snapshot(),
            Self::Direct(controller) => controller.snapshot(),
        }
    }

    fn settle(&self) {
        if let Self::Periodic(controller) = self {
            thread::sleep(controller.report_period().saturating_mul(2));
        }
    }
}

fn run_hardware(args: &RunnerArgs) -> bool {
    let started = Instant::now();
    emit(
        args,
        started,
        "runner_start",
        [
            ("mode", json!(args.mode.name())),
            ("operator_setup", json!(args.setup.name())),
            ("operator_setup_machine_verified", json!(false)),
            (
                "connect_timeout_ms",
                json!(duration_ms(args.connect_timeout)),
            ),
        ],
    );

    let baseline = match prepare_profile(args, started) {
        Ok(baseline) => baseline,
        Err(error_kind) => {
            emit_fixed_failure(args, started, "profile_preflight", error_kind);
            emit_completion(args, started, false);
            return false;
        }
    };
    let mut controller = match ProHardwareController::build(args) {
        Ok(controller) => controller,
        Err(error) => {
            emit_controller_failure(args, started, "profile_build", &error);
            emit_completion(args, started, false);
            return false;
        }
    };
    if let Err(error) = controller.open() {
        emit_controller_failure(args, started, "open", &error);
        emit_completion(args, started, false);
        return false;
    }
    emit(args, started, "reconnect_start", []);
    let reconnect_started = Instant::now();
    match controller.reconnect(args.connect_timeout) {
        Ok(()) if !args.setup.expects_connection() => {
            emit_fixed_failure(args, started, "reconnect", "unexpected_stale_bond_success");
            let _ = controller.close();
            emit_completion(args, started, false);
            return false;
        }
        Ok(()) => {
            emit(
                args,
                started,
                "reconnect_ready",
                [(
                    "reconnect_elapsed_ms",
                    json!(duration_ms(reconnect_started.elapsed())),
                )],
            );
            emit_status(args, started, "ready_status", None, &controller);
        }
        Err(error)
            if args.setup == OperatorSetup::StaleBond
                && matches!(
                    error.kind(),
                    ErrorKind::ConnectionFailed | ErrorKind::ConnectionTimeout
                ) =>
        {
            emit(
                args,
                started,
                "expected_reconnect_failure",
                [
                    ("error_kind", json!(error_kind_name(error.kind()))),
                    (
                        "reconnect_elapsed_ms",
                        json!(duration_ms(reconnect_started.elapsed())),
                    ),
                ],
            );
            let close_ok = close_controller(args, started, &mut controller, false);
            let reopen_ok = verify_adapter_reopen(args, started);
            let profile_ok = verify_profile_unchanged(args, started, &baseline);
            let success = close_ok && reopen_ok && profile_ok;
            emit_completion(args, started, success);
            return success;
        }
        Err(error) => {
            emit_controller_failure(args, started, "reconnect", &error);
            let _ = controller.close_without_neutral();
            verify_profile_unchanged(args, started, &baseline);
            emit_completion(args, started, false);
            return false;
        }
    }

    let idle_ok = if args.mode == RunnerMode::Direct {
        verify_direct_idle(args, started, &controller)
    } else {
        true
    };
    let input_ok = run_input_sequence(args, started, &mut controller).is_ok();
    if input_ok {
        emit_status(args, started, "pre_close_status", None, &controller);
    }
    let close_ok = close_controller(args, started, &mut controller, true);
    let reopen_ok = verify_adapter_reopen(args, started);
    let profile_ok = verify_profile_unchanged(args, started, &baseline);
    let success = idle_ok && input_ok && close_ok && reopen_ok && profile_ok;
    emit_completion(args, started, success);
    success
}

fn verify_direct_idle(
    args: &RunnerArgs,
    started: Instant,
    controller: &ProHardwareController,
) -> bool {
    let accepted_before = controller.status().input_reports_accepted;
    emit(
        args,
        started,
        "direct_idle_start",
        [("idle_ms", json!(duration_ms(DIRECT_IDLE)))],
    );
    thread::sleep(DIRECT_IDLE);
    let accepted_delta = controller
        .status()
        .input_reports_accepted
        .saturating_sub(accepted_before);
    emit(
        args,
        started,
        "direct_idle_complete",
        [("input_reports_accepted_delta", json!(accepted_delta))],
    );
    accepted_delta == 0
}

fn run_input_sequence(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut ProHardwareController,
) -> swbt::Result<()> {
    run_operation(args, started, controller, "a_500ms", |controller| {
        controller.tap(&[ProButton::A], BUTTON_HOLD)?;
        controller.settle();
        Ok(())
    })?;
    run_operation(args, started, controller, "l_plus_r_500ms", |controller| {
        controller.tap(&[ProButton::L, ProButton::R], BUTTON_HOLD)?;
        controller.settle();
        Ok(())
    })?;

    for (operation, state) in [
        (
            "left_stick_up_500ms",
            ProInputState::neutral().with_left_stick(Stick::up(1.0)?),
        ),
        (
            "left_stick_right_500ms",
            ProInputState::neutral().with_left_stick(Stick::right(1.0)?),
        ),
        (
            "left_stick_down_500ms",
            ProInputState::neutral().with_left_stick(Stick::down(1.0)?),
        ),
        (
            "left_stick_left_500ms",
            ProInputState::neutral().with_left_stick(Stick::left(1.0)?),
        ),
        (
            "right_stick_up_500ms",
            ProInputState::neutral().with_right_stick(Stick::up(1.0)?),
        ),
        (
            "right_stick_right_500ms",
            ProInputState::neutral().with_right_stick(Stick::right(1.0)?),
        ),
        (
            "right_stick_down_500ms",
            ProInputState::neutral().with_right_stick(Stick::down(1.0)?),
        ),
        (
            "right_stick_left_500ms",
            ProInputState::neutral().with_right_stick(Stick::left(1.0)?),
        ),
    ] {
        run_operation(args, started, controller, operation, |controller| {
            controller.send_state(state)?;
            thread::sleep(STICK_HOLD);
            controller.neutral()?;
            controller.settle();
            Ok(())
        })?;
    }

    run_operation(
        args,
        started,
        controller,
        "explicit_neutral",
        |controller| {
            controller.neutral()?;
            controller.settle();
            Ok(())
        },
    )
}

fn run_operation(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut ProHardwareController,
    operation: &'static str,
    action: impl FnOnce(&mut ProHardwareController) -> swbt::Result<()>,
) -> swbt::Result<()> {
    let operation_started = Instant::now();
    let accepted_before = controller.status().input_reports_accepted;
    emit(
        args,
        started,
        "operation_start",
        [
            ("operation", json!(operation)),
            ("ui_observation_required", json!(true)),
        ],
    );
    match action(controller) {
        Ok(()) => {
            let accepted_after = controller.status().input_reports_accepted;
            emit(
                args,
                started,
                "operation_complete",
                [
                    ("operation", json!(operation)),
                    (
                        "operation_elapsed_ms",
                        json!(duration_ms(operation_started.elapsed())),
                    ),
                    (
                        "input_reports_accepted_delta",
                        json!(accepted_after.saturating_sub(accepted_before)),
                    ),
                    ("ui_observed", Value::Null),
                ],
            );
            emit_status(
                args,
                started,
                "operation_status",
                Some(operation),
                controller,
            );
            Ok(())
        }
        Err(error) => {
            emit_controller_failure(args, started, operation, &error);
            Err(error)
        }
    }
}

fn close_controller(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut ProHardwareController,
    with_neutral: bool,
) -> bool {
    emit(
        args,
        started,
        "close_start",
        [("with_neutral", json!(with_neutral))],
    );
    let result = if with_neutral {
        controller.close()
    } else {
        controller.close_without_neutral()
    };
    match result {
        Ok(()) => {
            emit_status(args, started, "close_complete", None, controller);
            true
        }
        Err(error) => {
            emit_controller_failure(args, started, "close", &error);
            false
        }
    }
}

fn verify_adapter_reopen(args: &RunnerArgs, started: Instant) -> bool {
    emit(args, started, "adapter_reopen_start", []);
    let mut controller = match ProHardwareController::build(args) {
        Ok(controller) => controller,
        Err(error) => {
            emit_controller_failure(args, started, "adapter_reopen_build", &error);
            return false;
        }
    };
    if let Err(error) = controller.open() {
        emit_controller_failure(args, started, "adapter_reopen_open", &error);
        return false;
    }
    if let Err(error) = controller.close_without_neutral() {
        emit_controller_failure(args, started, "adapter_reopen_close", &error);
        return false;
    }
    emit_status(args, started, "adapter_reopen_complete", None, &controller);
    true
}

fn verify_profile_unchanged(
    args: &RunnerArgs,
    started: Instant,
    baseline: &ProfileBaseline,
) -> bool {
    let target_unchanged = fs::read(&args.profile).is_ok_and(|current| current == baseline.bytes);
    let source_unchanged = baseline
        .stale_source
        .as_ref()
        .is_none_or(|(path, expected)| fs::read(path).is_ok_and(|current| current == *expected));
    emit(
        args,
        started,
        "profile_postflight",
        [
            ("target_unchanged", json!(target_unchanged)),
            ("stale_source_unchanged", json!(source_unchanged)),
            ("raw_profile_emitted", json!(false)),
            ("key_material_emitted", json!(false)),
        ],
    );
    target_unchanged && source_unchanged
}

fn emit_status(
    args: &RunnerArgs,
    started: Instant,
    event: &'static str,
    operation: Option<&'static str>,
    controller: &ProHardwareController,
) {
    let status = controller.status();
    let mut fields = status_fields(&status);
    fields.push((
        "snapshot_neutral",
        json!(controller.snapshot() == ProInputState::neutral()),
    ));
    if let Some(operation) = operation {
        fields.push(("operation", json!(operation)));
    }
    emit(args, started, event, fields);
}

fn status_fields(status: &GamepadStatus) -> Vec<(&'static str, Value)> {
    vec![
        ("lifecycle", json!(lifecycle_name(status.lifecycle))),
        ("connected", json!(status.connected)),
        (
            "controller_kind",
            json!(status.controller_kind.profile_name()),
        ),
        (
            "reporting_kind",
            json!(reporting_name(status.reporting_kind)),
        ),
        ("report_mode", json!(status.report_mode)),
        (
            "input_reports_accepted",
            json!(status.input_reports_accepted),
        ),
        ("replies_accepted", json!(status.replies_accepted)),
        ("last_subcommand", json!(status.last_subcommand)),
        (
            "last_disconnect_reason",
            json!(status.last_disconnect_reason),
        ),
        (
            "worker_failure_present",
            json!(status.worker_failure.is_some()),
        ),
    ]
}

fn emit_controller_failure(
    args: &RunnerArgs,
    started: Instant,
    operation: &'static str,
    error: &Error,
) {
    emit(
        args,
        started,
        "operation_failure",
        [
            ("operation", json!(operation)),
            ("error_kind", json!(error_kind_name(error.kind()))),
            (
                "related_failure_present",
                json!(error.related_error().is_some()),
            ),
        ],
    );
}

fn emit_fixed_failure(
    args: &RunnerArgs,
    started: Instant,
    operation: &'static str,
    error_kind: &'static str,
) {
    emit(
        args,
        started,
        "operation_failure",
        [
            ("operation", json!(operation)),
            ("error_kind", json!(error_kind)),
            ("related_failure_present", json!(false)),
        ],
    );
}

fn emit_completion(args: &RunnerArgs, started: Instant, success: bool) {
    emit(
        args,
        started,
        "runner_complete",
        [("success", json!(success))],
    );
}

fn emit<I>(args: &RunnerArgs, started: Instant, event: &'static str, fields: I)
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let value = evidence_event(
        args.run_index,
        args.mode,
        args.setup,
        duration_ms(started.elapsed()),
        event,
        fields,
    );
    let encoded = serde_json::to_string(&value).expect("hardware evidence event must serialize");
    let mut output = io::stdout().lock();
    writeln!(output, "{encoded}").expect("hardware evidence event must be written");
    output
        .flush()
        .expect("hardware evidence event must be flushed");
}

fn evidence_event<I>(
    run_index: u8,
    mode: RunnerMode,
    setup: OperatorSetup,
    elapsed_ms: u64,
    event: &'static str,
    fields: I,
) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let mut object = Map::from_iter([
        ("schema".into(), json!(EVIDENCE_SCHEMA)),
        ("schema_version".into(), json!(EVIDENCE_SCHEMA_VERSION)),
        ("run_index".into(), json!(run_index)),
        ("mode".into(), json!(mode.name())),
        ("operator_setup".into(), json!(setup.name())),
        ("unix_time_ms".into(), json!(unix_time_ms())),
        ("elapsed_ms".into(), json!(elapsed_ms)),
        ("event".into(), json!(event)),
    ]);
    for (name, value) in fields {
        object.entry(name).or_insert(value);
    }
    Value::Object(object)
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn unix_time_ms() -> u64 {
    duration_ms(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO),
    )
}

fn lifecycle_name(lifecycle: LifecycleState) -> &'static str {
    match lifecycle {
        LifecycleState::Configured => "configured",
        LifecycleState::Open => "open",
        LifecycleState::Connecting => "connecting",
        LifecycleState::Ready => "ready",
        LifecycleState::Closing => "closing",
        LifecycleState::Closed => "closed",
        LifecycleState::Failed => "failed",
        _ => "unknown",
    }
}

fn reporting_name(reporting: ReportingKind) -> &'static str {
    match reporting {
        ReportingKind::Periodic => "periodic",
        ReportingKind::Direct => "direct",
    }
}

fn error_kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::AdapterDiscovery => "adapter_discovery",
        ErrorKind::TransportOpen => "transport_open",
        ErrorKind::ProfilePathRequired => "profile_path_required",
        ErrorKind::ProfileNotFound => "profile_not_found",
        ErrorKind::ProfileAlreadyExists => "profile_already_exists",
        ErrorKind::InvalidProfile => "invalid_profile",
        ErrorKind::ProfileControllerMismatch => "profile_controller_mismatch",
        ErrorKind::InvalidKeyStore => "invalid_key_store",
        ErrorKind::TransportClosed => "transport_closed",
        ErrorKind::NoBond => "no_bond",
        ErrorKind::ConnectionTimeout => "connection_timeout",
        ErrorKind::ConnectionFailed => "connection_failed",
        ErrorKind::Protocol => "protocol",
        ErrorKind::InvalidInput => "invalid_input",
        ErrorKind::UnsupportedInput => "unsupported_input",
        ErrorKind::UnsupportedCapability => "unsupported_capability",
        ErrorKind::WorkerFailed => "worker_failed",
        ErrorKind::Shutdown => "shutdown",
        ErrorKind::Internal => "internal",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
        time::Duration,
    };

    use serde_json::json;

    use super::{
        EVIDENCE_SCHEMA, EVIDENCE_SCHEMA_VERSION, OperatorSetup, RunnerMode, evidence_event,
        parse_args, stale_profile_bytes,
    };

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn runner_requires_explicit_reconnect_inputs() {
        let args = parse_args([
            "--adapter",
            "usb:0a12:0001",
            "--profile",
            "profile.json",
            "--mode",
            "direct",
            "--setup",
            "post-power-cycle",
            "--connect-timeout-secs",
            "60",
            "--run",
            "7",
        ])
        .expect("parse explicit reconnect inputs");

        assert_eq!(args.mode, RunnerMode::Direct);
        assert_eq!(args.setup, OperatorSetup::PostPowerCycle);
        assert_eq!(args.connect_timeout, Duration::from_secs(60));
        assert_eq!(args.run_index, 7);
        assert!(args.stale_source_profile.is_none());
        let rendered = format!("{args:?}");
        assert!(!rendered.contains("usb:0a12:0001"));
        assert!(!rendered.contains("profile.json"));
    }

    #[test]
    fn stale_bond_requires_distinct_source_and_target() {
        for args in [
            vec![
                "--adapter",
                "usb:0a12:0001",
                "--profile",
                "stale.json",
                "--mode",
                "periodic",
                "--setup",
                "stale-bond",
                "--connect-timeout-secs",
                "10",
                "--run",
                "3",
            ],
            vec![
                "--adapter",
                "usb:0a12:0001",
                "--profile",
                "same.json",
                "--stale-source-profile",
                "same.json",
                "--mode",
                "periodic",
                "--setup",
                "stale-bond",
                "--connect-timeout-secs",
                "10",
                "--run",
                "3",
            ],
        ] {
            assert!(parse_args(args).is_err());
        }
    }

    #[test]
    fn stale_copy_changes_one_key_nibble_without_changing_the_source_file() {
        let directory = TestDirectory::new();
        let source_path = directory.path.join("source.json");
        let source = profile_bytes();
        fs::write(&source_path, &source).expect("write source profile");

        let stale = stale_profile_bytes(&source).expect("create stale profile bytes");

        assert_ne!(stale, source);
        assert_eq!(
            fs::read(&source_path).expect("reread source"),
            source,
            "stale preparation must not mutate its source"
        );
        let source_json: serde_json::Value = serde_json::from_slice(&source).expect("source JSON");
        let stale_json: serde_json::Value = serde_json::from_slice(&stale).expect("stale JSON");
        assert_eq!(source_json["future_top"], stale_json["future_top"]);
        assert_ne!(
            source_json["key_store"]["namespaces"]["22:22:22:22:22:22"]["11:11:11:11:11:11"]["link_key"]
                ["value"],
            stale_json["key_store"]["namespaces"]["22:22:22:22:22:22"]["11:11:11:11:11:11"]["link_key"]
                ["value"]
        );
    }

    #[test]
    fn evidence_contains_mode_and_operator_claim_without_paths_or_keys() {
        let event = evidence_event(
            4,
            RunnerMode::Periodic,
            OperatorSetup::Normal,
            25,
            "runner_start",
            [
                ("raw_profile_emitted", json!(false)),
                ("key_material_emitted", json!(false)),
            ],
        );
        let encoded = event.to_string();

        assert_eq!(event["schema"], EVIDENCE_SCHEMA);
        assert_eq!(event["schema_version"], EVIDENCE_SCHEMA_VERSION);
        assert_eq!(event["mode"], "periodic");
        assert_eq!(event["operator_setup"], "normal");
        assert!(!encoded.contains("profile.json"));
        assert!(!encoded.contains("C7C7"));
    }

    fn profile_bytes() -> Vec<u8> {
        serde_json::to_vec_pretty(&json!({
            "format": "swbt.profile",
            "schema_version": 2,
            "controller_kind": "pro",
            "identity": {
                "kind": "adapter-default"
            },
            "key_store": {
                "namespaces": {
                    "22:22:22:22:22:22": {
                        "11:11:11:11:11:11": {
                            "link_key": {
                                "authenticated": true,
                                "value": "C7C7C7C7C7C7C7C7C7C7C7C7C7C7C7C7"
                            },
                            "link_key_type": 8
                        }
                    }
                }
            },
            "future_top": {
                "retained": true
            }
        }))
        .expect("serialize profile")
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "swbt-rs-pro-profile-runner-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create test directory");
            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
