use std::{
    fmt, fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use swbt::{
    CreateProfileOptions, ErrorKind, ImuFrame, ProButton, ProController, ProInputState,
    ProfileIdentity, Stick,
};

use crate::support::{
    EvidenceTarget, KeyValueArguments, UsageError, duration_ms, emit, emit_completion,
    emit_controller_failure, emit_fixed_failure, emit_status as emit_common_status,
    verify_adapter_reopen as verify_common_adapter_reopen,
};

const EVIDENCE_SCHEMA: &str = "swbt.m5.pro-periodic";
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

impl EvidenceTarget for RunnerArgs {
    fn evidence_schema(&self) -> &'static str {
        EVIDENCE_SCHEMA
    }

    fn evidence_dimensions(&self) -> Vec<(&'static str, Value)> {
        vec![("run_index", json!(self.run_index))]
    }

    fn error_kind_name(&self, kind: ErrorKind) -> &'static str {
        match kind {
            ErrorKind::InvalidKeyStore | ErrorKind::NoBond => "unknown",
            _ => crate::support::error_kind_name(kind),
        }
    }
}

pub(crate) fn run(arguments: Vec<String>) -> u8 {
    let args = match parse_args(arguments) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: swbt-hardware-runner pro-periodic --adapter <selector> --profile <new-path> \
                 --pair-timeout-secs <1..600> --run <1..20>"
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
    let mut args = KeyValueArguments::parse(
        args,
        &["--adapter", "--profile", "--pair-timeout-secs", "--run"],
    )?;

    let adapter = args.required("--adapter", "missing --adapter")?;
    if adapter.is_empty() {
        return Err(UsageError("invalid --adapter"));
    }
    let profile = PathBuf::from(args.required("--profile", "missing --profile")?);
    if profile.as_os_str().is_empty() {
        return Err(UsageError("invalid --profile"));
    }
    let pair_timeout_secs = args
        .required("--pair-timeout-secs", "missing --pair-timeout-secs")?
        .parse::<u64>()
        .map_err(|_| UsageError("invalid --pair-timeout-secs"))?;
    if !(MIN_PAIR_TIMEOUT_SECS..=MAX_PAIR_TIMEOUT_SECS).contains(&pair_timeout_secs) {
        return Err(UsageError("invalid --pair-timeout-secs"));
    }
    let run_index = args
        .required("--run", "missing --run")?
        .parse::<u8>()
        .map_err(|_| UsageError("invalid --run"))?;
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
    verify_common_adapter_reopen(
        args,
        started,
        || {
            ProController::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .build()
        },
        |controller| controller.open(),
        |controller| controller.close_without_neutral(),
        |controller| {
            (
                controller.status(),
                controller.snapshot() == ProInputState::neutral(),
            )
        },
    )
}

fn emit_status(
    args: &RunnerArgs,
    started: Instant,
    event: &'static str,
    operation: Option<&'static str>,
    controller: &ProController,
) {
    let status = controller.status();
    emit_common_status(
        args,
        started,
        event,
        operation,
        &status,
        controller.snapshot() == ProInputState::neutral(),
    );
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use serde_json::json;

    use swbt::ErrorKind;

    use crate::support::{EvidenceTarget, evidence_event};

    use super::{EVIDENCE_SCHEMA, RunnerArgs, parse_args};

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
        let args = RunnerArgs {
            adapter: "secret-adapter".to_owned(),
            profile: "secret-profile".into(),
            pair_timeout: Duration::from_secs(60),
            run_index: 4,
        };
        let event = evidence_event(
            &args,
            125,
            "runner_start",
            [
                ("pair_timeout_ms", json!(60_000)),
                ("schema", json!("must-not-replace-the-schema")),
            ],
        );
        let encoded = serde_json::to_string(&event).expect("serialize evidence event");

        assert_eq!(event["schema"], EVIDENCE_SCHEMA);
        assert_eq!(event["schema_version"], 1);
        assert_eq!(event["run_index"], 4);
        assert_eq!(event["elapsed_ms"], 125);
        assert_eq!(event["event"], "runner_start");
        assert_eq!(event["pair_timeout_ms"], 60_000);
        assert!(!encoded.contains("adapter"));
        assert!(!encoded.contains("profile"));
        assert!(!encoded.contains("key"));
        assert!(!encoded.contains("usb:"));
        assert_eq!(args.error_kind_name(ErrorKind::NoBond), "unknown");
        assert_eq!(
            args.error_kind_name(ErrorKind::AdapterIdentityRecoveryRequired),
            "adapter_identity_recovery_required"
        );
    }
}
