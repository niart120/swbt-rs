use std::{
    env, fmt, fs,
    io::{self, Write},
    path::PathBuf,
    process, thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use swbt::{
    CreateProfileOptions, Error, ErrorKind, GamepadStatus, ImuFrame, LifecycleState, ProButton,
    ProController, ProInputState, ProfileIdentity, ReportingKind, Stick,
};

const EVIDENCE_SCHEMA: &str = "swbt.m5.pro-periodic";
const EVIDENCE_SCHEMA_VERSION: u64 = 1;
const BUTTON_HOLD: Duration = Duration::from_millis(500);
const STICK_HOLD: Duration = Duration::from_millis(500);
const IMU_HOLD: Duration = Duration::from_secs(1);
const MIN_PAIR_TIMEOUT_SECS: u64 = 1;
const MAX_PAIR_TIMEOUT_SECS: u64 = 600;
const MIN_RUN_INDEX: u8 = 1;
const MAX_RUN_INDEX: u8 = 20;

struct RunnerArgs {
    adapter: String,
    profile: PathBuf,
    pair_timeout: Duration,
    run_index: u8,
}

impl fmt::Debug for RunnerArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunnerArgs")
            .field("adapter", &"<redacted>")
            .field("profile", &"<redacted>")
            .field("pair_timeout", &self.pair_timeout)
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

fn main() {
    let args = match parse_args(env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: pro_periodic_hardware --adapter <selector> --profile <new-path> \
                 --pair-timeout-secs <1..600> --run <1..20>"
            );
            process::exit(2);
        }
    };

    let success = run_hardware(&args);
    process::exit(if success { 0 } else { 1 });
}

fn parse_args<I, S>(args: I) -> Result<RunnerArgs, UsageError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut adapter = None;
    let mut profile = None;
    let mut pair_timeout_secs = None;
    let mut run_index = None;
    let mut args = args.into_iter().map(Into::into);

    while let Some(flag) = args.next() {
        let value = args.next().ok_or(UsageError("argument value is missing"))?;
        match flag.as_str() {
            "--adapter" if adapter.is_none() => adapter = Some(value),
            "--profile" if profile.is_none() => profile = Some(PathBuf::from(value)),
            "--pair-timeout-secs" if pair_timeout_secs.is_none() => {
                pair_timeout_secs = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| UsageError("invalid --pair-timeout-secs"))?,
                );
            }
            "--run" if run_index.is_none() => {
                run_index = Some(
                    value
                        .parse::<u8>()
                        .map_err(|_| UsageError("invalid --run"))?,
                );
            }
            "--adapter" | "--profile" | "--pair-timeout-secs" | "--run" => {
                return Err(UsageError("duplicate argument"));
            }
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
    let pair_timeout_secs = pair_timeout_secs.ok_or(UsageError("missing --pair-timeout-secs"))?;
    if !(MIN_PAIR_TIMEOUT_SECS..=MAX_PAIR_TIMEOUT_SECS).contains(&pair_timeout_secs) {
        return Err(UsageError("invalid --pair-timeout-secs"));
    }
    let run_index = run_index.ok_or(UsageError("missing --run"))?;
    if !(MIN_RUN_INDEX..=MAX_RUN_INDEX).contains(&run_index) {
        return Err(UsageError("invalid --run"));
    }

    Ok(RunnerArgs {
        adapter,
        profile,
        pair_timeout: Duration::from_secs(pair_timeout_secs),
        run_index,
    })
}

fn run_hardware(args: &RunnerArgs) -> bool {
    let started = Instant::now();
    emit(
        args,
        started,
        "runner_start",
        [("pair_timeout_ms", json!(duration_ms(args.pair_timeout)))],
    );

    match fs::symlink_metadata(&args.profile) {
        Ok(_) => {
            emit_fixed_failure(args, started, "profile_preflight", "target_exists");
            emit_completion(args, started, false);
            return false;
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            emit_fixed_failure(args, started, "profile_preflight", "filesystem");
            emit_completion(args, started, false);
            return false;
        }
    }

    emit(args, started, "pair_start", []);
    let pair_started = Instant::now();
    let controller = ProController::builder(args.adapter.clone())
        .profile_path(args.profile.clone())
        .create_profile(CreateProfileOptions {
            identity: ProfileIdentity::AdapterDefault,
            pair_timeout: args.pair_timeout,
        });
    let mut controller = match controller {
        Ok(controller) => controller,
        Err(error) => {
            emit_controller_failure(args, started, "create_profile", &error);
            let profile_valid = validate_profile(args, started);
            let reopen_ok = profile_valid && verify_adapter_reopen(args, started);
            if reopen_ok {
                emit(args, started, "failed_run_cleanup_verified", []);
            }
            emit_completion(args, started, false);
            return false;
        }
    };

    emit(
        args,
        started,
        "pair_ready",
        [(
            "pair_elapsed_ms",
            json!(duration_ms(pair_started.elapsed())),
        )],
    );
    emit_status(args, started, "ready_status", None, &controller);

    let input_result = run_input_sequence(args, started, &mut controller);
    if input_result.is_ok() {
        emit_status(args, started, "pre_close_status", None, &controller);
    }

    emit(args, started, "close_start", []);
    let close_result = controller.close();
    match &close_result {
        Ok(()) => emit_status(args, started, "close_complete", None, &controller),
        Err(error) => emit_controller_failure(args, started, "close", error),
    }

    let profile_valid = validate_profile(args, started);
    let reopen_ok = profile_valid && verify_adapter_reopen(args, started);
    let success = input_result.is_ok() && close_result.is_ok() && profile_valid && reopen_ok;
    emit_completion(args, started, success);
    success
}

fn run_input_sequence(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut ProController,
) -> swbt::Result<()> {
    run_operation(args, started, controller, "a_500ms", true, |controller| {
        controller.tap([ProButton::A], BUTTON_HOLD)?;
        settle_periodic(controller);
        Ok(())
    })?;

    run_operation(
        args,
        started,
        controller,
        "l_plus_r_500ms",
        true,
        |controller| {
            controller.tap([ProButton::L, ProButton::R], BUTTON_HOLD)?;
            settle_periodic(controller);
            Ok(())
        },
    )?;

    for (operation, left, stick) in [
        ("left_stick_up_500ms", true, Stick::up(1.0)?),
        ("left_stick_right_500ms", true, Stick::right(1.0)?),
        ("left_stick_down_500ms", true, Stick::down(1.0)?),
        ("left_stick_left_500ms", true, Stick::left(1.0)?),
        ("right_stick_up_500ms", false, Stick::up(1.0)?),
        ("right_stick_right_500ms", false, Stick::right(1.0)?),
        ("right_stick_down_500ms", false, Stick::down(1.0)?),
        ("right_stick_left_500ms", false, Stick::left(1.0)?),
    ] {
        run_operation(args, started, controller, operation, true, |controller| {
            let state = if left {
                ProInputState::neutral().with_left_stick(stick)
            } else {
                ProInputState::neutral().with_right_stick(stick)
            };
            controller.apply(state)?;
            thread::sleep(STICK_HOLD);
            controller.neutral()?;
            settle_periodic(controller);
            Ok(())
        })?;
    }

    let imu = ImuFrame::accel_g(0.25, -0.25, 1.25)?.with_gyro_rate(0.5, -0.25, 0.125)?;
    run_operation(
        args,
        started,
        controller,
        "imu_non_neutral_1000ms",
        false,
        |controller| {
            controller.apply(ProInputState::neutral().with_imu(imu))?;
            thread::sleep(IMU_HOLD);
            controller.neutral()?;
            settle_periodic(controller);
            Ok(())
        },
    )?;

    run_operation(
        args,
        started,
        controller,
        "explicit_neutral",
        true,
        |controller| {
            controller.neutral()?;
            settle_periodic(controller);
            Ok(())
        },
    )
}

fn run_operation(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut ProController,
    operation: &'static str,
    observation_required: bool,
    action: impl FnOnce(&mut ProController) -> swbt::Result<()>,
) -> swbt::Result<()> {
    let operation_started = Instant::now();
    let accepted_before = controller.status().input_reports_accepted;
    emit(
        args,
        started,
        "operation_start",
        [
            ("operation", json!(operation)),
            ("ui_observation_required", json!(observation_required)),
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

fn settle_periodic(controller: &ProController) {
    thread::sleep(controller.report_period().saturating_mul(2));
}

fn validate_profile(args: &RunnerArgs, started: Instant) -> bool {
    let result = ProController::builder(args.adapter.clone())
        .profile_path(args.profile.clone())
        .build();
    match result {
        Ok(_) => {
            emit(
                args,
                started,
                "profile_validation",
                [
                    ("valid", json!(true)),
                    ("controller_kind", json!("pro")),
                    ("raw_profile_emitted", json!(false)),
                    ("key_material_emitted", json!(false)),
                ],
            );
            true
        }
        Err(error) => {
            emit_controller_failure(args, started, "profile_validation", &error);
            false
        }
    }
}

fn verify_adapter_reopen(args: &RunnerArgs, started: Instant) -> bool {
    emit(args, started, "adapter_reopen_start", []);
    let controller = ProController::builder(args.adapter.clone())
        .profile_path(args.profile.clone())
        .build();
    let mut controller = match controller {
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

fn emit_status(
    args: &RunnerArgs,
    started: Instant,
    event: &'static str,
    operation: Option<&'static str>,
    controller: &ProController,
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

fn evidence_event<I>(run_index: u8, elapsed_ms: u64, event: &'static str, fields: I) -> Value
where
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let mut object = Map::from_iter([
        ("schema".into(), json!(EVIDENCE_SCHEMA)),
        ("schema_version".into(), json!(EVIDENCE_SCHEMA_VERSION)),
        ("run_index".into(), json!(run_index)),
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
        ErrorKind::TransportClosed => "transport_closed",
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
    use std::{path::Path, time::Duration};

    use serde_json::json;

    use super::{EVIDENCE_SCHEMA, EVIDENCE_SCHEMA_VERSION, evidence_event, parse_args};

    #[test]
    fn runner_requires_all_explicit_inputs() {
        let args = parse_args([
            "--adapter",
            "usb:0a12:0001",
            "--profile",
            "evidence/run-07.json",
            "--pair-timeout-secs",
            "60",
            "--run",
            "7",
        ])
        .expect("parse explicit hardware-runner inputs");

        assert_eq!(args.adapter, "usb:0a12:0001");
        assert_eq!(args.profile, Path::new("evidence/run-07.json"));
        assert_eq!(args.pair_timeout, Duration::from_secs(60));
        assert_eq!(args.run_index, 7);
    }

    #[test]
    fn runner_rejects_an_omitted_pair_timeout() {
        let error = parse_args([
            "--adapter",
            "usb:0a12:0001",
            "--profile",
            "evidence/run-01.json",
            "--run",
            "1",
        ])
        .expect_err("pair timeout is a required run input");

        assert_eq!(error.to_string(), "missing --pair-timeout-secs");
    }

    #[test]
    fn evidence_schema_has_no_field_for_sensitive_inputs() {
        let event = evidence_event(
            4,
            125,
            "runner_start",
            [
                ("pair_timeout_ms", json!(60_000)),
                ("schema", json!("must-not-replace-the-schema")),
            ],
        );
        let encoded = serde_json::to_string(&event).expect("serialize evidence event");

        assert_eq!(event["schema"], EVIDENCE_SCHEMA);
        assert_eq!(event["schema_version"], EVIDENCE_SCHEMA_VERSION);
        assert_eq!(event["run_index"], 4);
        assert_eq!(event["elapsed_ms"], 125);
        assert_eq!(event["event"], "runner_start");
        assert_eq!(event["pair_timeout_ms"], 60_000);
        assert!(!encoded.contains("adapter"));
        assert!(!encoded.contains("profile"));
        assert!(!encoded.contains("key"));
        assert!(!encoded.contains("usb:"));
    }
}
