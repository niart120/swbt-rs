use std::{
    fmt, fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use swbt::{
    CreateProfileOptions, DirectJoyConL, DirectJoyConR, ErrorKind, GamepadStatus, JoyConL,
    JoyConLButton, JoyConLInputState, JoyConR, JoyConRButton, JoyConRInputState, ProfileIdentity,
    Stick,
};

use crate::support::{
    EvidenceTarget, KeyValueArguments, UsageError, duration_ms, emit, emit_completion,
    emit_controller_failure, emit_fixed_failure, emit_status as emit_common_status,
    verify_adapter_reopen as verify_common_adapter_reopen,
};

const EVIDENCE_SCHEMA: &str = "swbt.m7.joycon-profile";
const BUTTON_HOLD: Duration = Duration::from_millis(500);
const RIGHT_FACE_BUTTON_HOLD: Duration = Duration::from_millis(200);
const STICK_HOLD: Duration = Duration::from_millis(500);
const DIRECT_IDLE: Duration = Duration::from_millis(500);
const MIN_TIMEOUT_SECS: u64 = 1;
const MAX_TIMEOUT_SECS: u64 = 600;
const MAX_PRE_INPUT_IDLE_MS: u64 = 10_000;
const MIN_RUN_INDEX: u8 = 1;
const MAX_RUN_INDEX: u8 = 99;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JoyConModel {
    Left,
    Right,
}

impl JoyConModel {
    const fn name(self) -> &'static str {
        match self {
            Self::Left => "joycon_l",
            Self::Right => "joycon_r",
        }
    }
}

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
enum ConnectionOperation {
    Pair,
    Reconnect,
}

impl ConnectionOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::Pair => "pair",
            Self::Reconnect => "reconnect",
        }
    }
}

struct RunnerArgs {
    adapter: String,
    profile: PathBuf,
    model: JoyConModel,
    mode: RunnerMode,
    connection: ConnectionOperation,
    timeout: Duration,
    pre_input_idle: Duration,
    run_index: u8,
}

impl fmt::Debug for RunnerArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunnerArgs")
            .field("adapter", &"<redacted>")
            .field("profile", &"<redacted>")
            .field("model", &self.model)
            .field("mode", &self.mode)
            .field("connection", &self.connection)
            .field("timeout", &self.timeout)
            .field("pre_input_idle", &self.pre_input_idle)
            .field("run_index", &self.run_index)
            .finish()
    }
}

impl EvidenceTarget for RunnerArgs {
    fn evidence_schema(&self) -> &'static str {
        EVIDENCE_SCHEMA
    }

    fn evidence_dimensions(&self) -> Vec<(&'static str, Value)> {
        vec![
            ("run_index", json!(self.run_index)),
            ("model", json!(self.model.name())),
            ("mode", json!(self.mode.name())),
            ("connection", json!(self.connection.name())),
        ]
    }
}

pub(crate) fn run(arguments: Vec<String>) -> u8 {
    let args = match parse_args(arguments) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            eprintln!(
                "usage: swbt-hardware-runner joycon-profile --adapter <selector> --profile <path> \
                 --model <left|right> --mode <periodic|direct> \
                 --connection <pair|reconnect> --timeout-secs <1..600> \
                 [--pre-input-idle-ms <0..10000>] --run <1..99>"
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
        &[
            "--adapter",
            "--profile",
            "--model",
            "--mode",
            "--connection",
            "--timeout-secs",
            "--pre-input-idle-ms",
            "--run",
        ],
    )?;

    let adapter = args.required("--adapter", "missing --adapter")?;
    if adapter.is_empty() {
        return Err(UsageError("invalid --adapter"));
    }
    let profile = PathBuf::from(args.required("--profile", "missing --profile")?);
    if profile.as_os_str().is_empty() {
        return Err(UsageError("invalid --profile"));
    }
    let model = match args.required("--model", "missing --model")?.as_str() {
        "left" => JoyConModel::Left,
        "right" => JoyConModel::Right,
        _ => return Err(UsageError("invalid --model")),
    };
    let mode = match args.required("--mode", "missing --mode")?.as_str() {
        "periodic" => RunnerMode::Periodic,
        "direct" => RunnerMode::Direct,
        _ => return Err(UsageError("invalid --mode")),
    };
    let connection = match args
        .required("--connection", "missing --connection")?
        .as_str()
    {
        "pair" => ConnectionOperation::Pair,
        "reconnect" => ConnectionOperation::Reconnect,
        _ => return Err(UsageError("invalid --connection")),
    };
    if connection == ConnectionOperation::Pair && mode != RunnerMode::Periodic {
        return Err(UsageError("pair requires periodic mode"));
    }
    let timeout_secs = args
        .required("--timeout-secs", "missing --timeout-secs")?
        .parse::<u64>()
        .map_err(|_| UsageError("invalid --timeout-secs"))?;
    if !(MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&timeout_secs) {
        return Err(UsageError("invalid --timeout-secs"));
    }
    let pre_input_idle_ms = args
        .optional("--pre-input-idle-ms")
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|_| UsageError("invalid --pre-input-idle-ms"))
        })
        .transpose()?
        .unwrap_or(0);
    if pre_input_idle_ms > MAX_PRE_INPUT_IDLE_MS {
        return Err(UsageError("invalid --pre-input-idle-ms"));
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
        model,
        mode,
        connection,
        timeout: Duration::from_secs(timeout_secs),
        pre_input_idle: Duration::from_millis(pre_input_idle_ms),
        run_index,
    })
}

enum ProfileBaseline {
    New,
    Existing(Vec<u8>),
}

fn prepare_profile(args: &RunnerArgs, started: Instant) -> Result<ProfileBaseline, &'static str> {
    match args.connection {
        ConnectionOperation::Pair => match fs::symlink_metadata(&args.profile) {
            Ok(_) => Err("pair_target_exists"),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                emit(
                    args,
                    started,
                    "profile_preflight",
                    [
                        ("profile_exists", json!(false)),
                        ("raw_profile_emitted", json!(false)),
                        ("key_material_emitted", json!(false)),
                    ],
                );
                Ok(ProfileBaseline::New)
            }
            Err(_) => Err("profile_preflight_filesystem"),
        },
        ConnectionOperation::Reconnect => {
            let bytes = fs::read(&args.profile).map_err(|_| "profile_read")?;
            emit(
                args,
                started,
                "profile_preflight",
                [
                    ("profile_exists", json!(true)),
                    ("profile_size_bytes", json!(bytes.len())),
                    ("raw_profile_emitted", json!(false)),
                    ("key_material_emitted", json!(false)),
                ],
            );
            Ok(ProfileBaseline::Existing(bytes))
        }
    }
}

enum JoyConHardwareController {
    LeftPeriodic(JoyConL),
    LeftDirect(DirectJoyConL),
    RightPeriodic(JoyConR),
    RightDirect(DirectJoyConR),
}

impl JoyConHardwareController {
    fn create_paired(args: &RunnerArgs) -> swbt::Result<Self> {
        let options = CreateProfileOptions {
            identity: ProfileIdentity::AdapterDefault,
            pair_timeout: args.timeout,
        };
        match args.model {
            JoyConModel::Left => JoyConL::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .create_profile(options)
                .map(Self::LeftPeriodic),
            JoyConModel::Right => JoyConR::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .create_profile(options)
                .map(Self::RightPeriodic),
        }
    }

    fn build(args: &RunnerArgs) -> swbt::Result<Self> {
        match (args.model, args.mode) {
            (JoyConModel::Left, RunnerMode::Periodic) => JoyConL::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .build()
                .map(Self::LeftPeriodic),
            (JoyConModel::Left, RunnerMode::Direct) => DirectJoyConL::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .build()
                .map(Self::LeftDirect),
            (JoyConModel::Right, RunnerMode::Periodic) => JoyConR::builder(args.adapter.clone())
                .profile_path(args.profile.clone())
                .build()
                .map(Self::RightPeriodic),
            (JoyConModel::Right, RunnerMode::Direct) => {
                DirectJoyConR::builder(args.adapter.clone())
                    .profile_path(args.profile.clone())
                    .build()
                    .map(Self::RightDirect)
            }
        }
    }

    fn open(&mut self) -> swbt::Result<()> {
        match self {
            Self::LeftPeriodic(controller) => controller.open(),
            Self::LeftDirect(controller) => controller.open(),
            Self::RightPeriodic(controller) => controller.open(),
            Self::RightDirect(controller) => controller.open(),
        }
    }

    fn reconnect(&mut self, timeout: Duration) -> swbt::Result<()> {
        match self {
            Self::LeftPeriodic(controller) => controller.reconnect(timeout),
            Self::LeftDirect(controller) => controller.reconnect(timeout),
            Self::RightPeriodic(controller) => controller.reconnect(timeout),
            Self::RightDirect(controller) => controller.reconnect(timeout),
        }
    }

    fn tap_left(&mut self, buttons: &[JoyConLButton], duration: Duration) -> swbt::Result<()> {
        match self {
            Self::LeftPeriodic(controller) => {
                controller.press(buttons.iter().copied())?;
                thread::sleep(duration);
                controller.release(buttons.iter().copied())
            }
            Self::LeftDirect(controller) => controller.tap(buttons.iter().copied(), duration),
            Self::RightPeriodic(_) | Self::RightDirect(_) => {
                unreachable!("runner model and left input sequence must agree")
            }
        }
    }

    fn tap_right(&mut self, buttons: &[JoyConRButton], duration: Duration) -> swbt::Result<()> {
        match self {
            Self::RightPeriodic(controller) => {
                controller.press(buttons.iter().copied())?;
                thread::sleep(duration);
                controller.release(buttons.iter().copied())
            }
            Self::RightDirect(controller) => controller.tap(buttons.iter().copied(), duration),
            Self::LeftPeriodic(_) | Self::LeftDirect(_) => {
                unreachable!("runner model and right input sequence must agree")
            }
        }
    }

    fn send_left(&mut self, state: JoyConLInputState) -> swbt::Result<()> {
        match self {
            Self::LeftPeriodic(controller) => controller.apply(state),
            Self::LeftDirect(controller) => controller.send(state),
            Self::RightPeriodic(_) | Self::RightDirect(_) => {
                unreachable!("runner model and left input state must agree")
            }
        }
    }

    fn send_right(&mut self, state: JoyConRInputState) -> swbt::Result<()> {
        match self {
            Self::RightPeriodic(controller) => controller.apply(state),
            Self::RightDirect(controller) => controller.send(state),
            Self::LeftPeriodic(_) | Self::LeftDirect(_) => {
                unreachable!("runner model and right input state must agree")
            }
        }
    }

    fn neutral(&mut self) -> swbt::Result<()> {
        match self {
            Self::LeftPeriodic(controller) => controller.neutral(),
            Self::LeftDirect(controller) => controller.neutral(),
            Self::RightPeriodic(controller) => controller.neutral(),
            Self::RightDirect(controller) => controller.neutral(),
        }
    }

    fn close(&mut self) -> swbt::Result<()> {
        match self {
            Self::LeftPeriodic(controller) => controller.close(),
            Self::LeftDirect(controller) => controller.close(),
            Self::RightPeriodic(controller) => controller.close(),
            Self::RightDirect(controller) => controller.close(),
        }
    }

    fn close_without_neutral(&mut self) -> swbt::Result<()> {
        match self {
            Self::LeftPeriodic(controller) => controller.close_without_neutral(),
            Self::LeftDirect(controller) => controller.close_without_neutral(),
            Self::RightPeriodic(controller) => controller.close_without_neutral(),
            Self::RightDirect(controller) => controller.close_without_neutral(),
        }
    }

    fn status(&self) -> GamepadStatus {
        match self {
            Self::LeftPeriodic(controller) => controller.status(),
            Self::LeftDirect(controller) => controller.status(),
            Self::RightPeriodic(controller) => controller.status(),
            Self::RightDirect(controller) => controller.status(),
        }
    }

    fn snapshot_neutral(&self) -> bool {
        match self {
            Self::LeftPeriodic(controller) => controller.snapshot() == JoyConLInputState::neutral(),
            Self::LeftDirect(controller) => controller.snapshot() == JoyConLInputState::neutral(),
            Self::RightPeriodic(controller) => {
                controller.snapshot() == JoyConRInputState::neutral()
            }
            Self::RightDirect(controller) => controller.snapshot() == JoyConRInputState::neutral(),
        }
    }

    fn settle(&self) {
        match self {
            Self::LeftPeriodic(controller) => {
                thread::sleep(controller.report_period().saturating_mul(2));
            }
            Self::RightPeriodic(controller) => {
                thread::sleep(controller.report_period().saturating_mul(2));
            }
            Self::LeftDirect(_) | Self::RightDirect(_) => {}
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
            ("timeout_ms", json!(duration_ms(args.timeout))),
            ("pre_input_idle_ms", json!(duration_ms(args.pre_input_idle))),
            ("ui_observation_machine_verified", json!(false)),
            ("switch_system_version_reported", json!("22.5.0")),
            ("switch_system_version_machine_verified", json!(false)),
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

    emit(args, started, "connection_start", []);
    let connection_started = Instant::now();
    let controller = match args.connection {
        ConnectionOperation::Pair => JoyConHardwareController::create_paired(args),
        ConnectionOperation::Reconnect => {
            let mut controller = match JoyConHardwareController::build(args) {
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
            match controller.reconnect(args.timeout) {
                Ok(()) => Ok(controller),
                Err(error) => {
                    emit_status(
                        args,
                        started,
                        "connection_failure_status",
                        Some("connection"),
                        &controller,
                    );
                    let _ = controller.close_without_neutral();
                    Err(error)
                }
            }
        }
    };
    let mut controller = match controller {
        Ok(controller) => controller,
        Err(error) => {
            emit_controller_failure(args, started, "connection", &error);
            let _ = verify_profile_postflight(args, started, &baseline);
            emit_completion(args, started, false);
            return false;
        }
    };

    emit(
        args,
        started,
        "connection_ready",
        [(
            "connection_elapsed_ms",
            json!(duration_ms(connection_started.elapsed())),
        )],
    );
    emit_status(args, started, "ready_status", None, &controller);

    let pre_input_ok = verify_pre_input_idle(args, started, &controller);
    let idle_ok = pre_input_ok
        && (args.mode != RunnerMode::Direct || verify_direct_idle(args, started, &controller));
    let input_ok = pre_input_ok && run_input_sequence(args, started, &mut controller).is_ok();
    if input_ok {
        emit_status(args, started, "pre_close_status", None, &controller);
    }
    let close_ok = close_controller(args, started, &mut controller);
    let reopen_ok = verify_adapter_reopen(args, started);
    let profile_ok = verify_profile_postflight(args, started, &baseline);
    let success = idle_ok && input_ok && close_ok && reopen_ok && profile_ok;
    emit_completion(args, started, success);
    success
}

fn verify_pre_input_idle(
    args: &RunnerArgs,
    started: Instant,
    controller: &JoyConHardwareController,
) -> bool {
    if args.pre_input_idle.is_zero() {
        return true;
    }
    let accepted_before = controller.status().input_reports_accepted;
    emit(
        args,
        started,
        "pre_input_idle_start",
        [("idle_ms", json!(duration_ms(args.pre_input_idle)))],
    );
    thread::sleep(args.pre_input_idle);
    let status = controller.status();
    let accepted_delta = status
        .input_reports_accepted
        .saturating_sub(accepted_before);
    emit(
        args,
        started,
        "pre_input_idle_complete",
        [
            ("connected", json!(status.connected)),
            ("input_reports_accepted_delta", json!(accepted_delta)),
        ],
    );
    let reporting_valid = match args.mode {
        RunnerMode::Periodic => accepted_delta > 0,
        RunnerMode::Direct => accepted_delta == 0,
    };
    if status.connected && reporting_valid {
        true
    } else {
        emit_fixed_failure(args, started, "pre_input_idle", "idle_verification_failed");
        false
    }
}

fn verify_direct_idle(
    args: &RunnerArgs,
    started: Instant,
    controller: &JoyConHardwareController,
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
    controller: &mut JoyConHardwareController,
) -> Result<(), InputSequenceFailure> {
    match args.model {
        JoyConModel::Left => run_left_input_sequence(args, started, controller),
        JoyConModel::Right => run_right_input_sequence(args, started, controller),
    }
}

fn run_left_input_sequence(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut JoyConHardwareController,
) -> Result<(), InputSequenceFailure> {
    for (operation, button) in [
        ("dpad_up_500ms", JoyConLButton::DPAD_UP),
        ("dpad_right_500ms", JoyConLButton::DPAD_RIGHT),
        ("dpad_down_500ms", JoyConLButton::DPAD_DOWN),
        ("dpad_left_500ms", JoyConLButton::DPAD_LEFT),
    ] {
        run_operation(args, started, controller, operation, |controller| {
            controller.tap_left(&[button], BUTTON_HOLD)?;
            controller.settle();
            Ok(())
        })?;
    }
    for (operation, buttons) in [
        ("l_plus_zl_500ms", [JoyConLButton::L, JoyConLButton::ZL]),
        ("sl_plus_sr_500ms", [JoyConLButton::SL, JoyConLButton::SR]),
    ] {
        run_operation(args, started, controller, operation, |controller| {
            controller.tap_left(&buttons, BUTTON_HOLD)?;
            controller.settle();
            Ok(())
        })?;
    }
    for (operation, stick) in [
        (
            "left_stick_up_500ms",
            Stick::up(1.0).expect("runner uses an in-range stick amount"),
        ),
        (
            "left_stick_right_500ms",
            Stick::right(1.0).expect("runner uses an in-range stick amount"),
        ),
        (
            "left_stick_down_500ms",
            Stick::down(1.0).expect("runner uses an in-range stick amount"),
        ),
        (
            "left_stick_left_500ms",
            Stick::left(1.0).expect("runner uses an in-range stick amount"),
        ),
    ] {
        run_operation(args, started, controller, operation, |controller| {
            controller.send_left(JoyConLInputState::neutral().with_left_stick(stick))?;
            thread::sleep(STICK_HOLD);
            controller.neutral()?;
            controller.settle();
            Ok(())
        })?;
    }
    run_explicit_neutral(args, started, controller)
}

fn run_right_input_sequence(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut JoyConHardwareController,
) -> Result<(), InputSequenceFailure> {
    for (operation, button) in [
        ("a_200ms", JoyConRButton::A),
        ("b_200ms", JoyConRButton::B),
        ("x_200ms", JoyConRButton::X),
        ("y_200ms", JoyConRButton::Y),
    ] {
        run_operation(args, started, controller, operation, |controller| {
            controller.tap_right(&[button], RIGHT_FACE_BUTTON_HOLD)?;
            controller.settle();
            Ok(())
        })?;
    }
    for (operation, buttons) in [
        ("r_plus_zr_500ms", [JoyConRButton::R, JoyConRButton::ZR]),
        ("sl_plus_sr_500ms", [JoyConRButton::SL, JoyConRButton::SR]),
    ] {
        run_operation(args, started, controller, operation, |controller| {
            controller.tap_right(&buttons, BUTTON_HOLD)?;
            controller.settle();
            Ok(())
        })?;
    }
    for (operation, stick) in [
        (
            "right_stick_up_500ms",
            Stick::up(1.0).expect("runner uses an in-range stick amount"),
        ),
        (
            "right_stick_right_500ms",
            Stick::right(1.0).expect("runner uses an in-range stick amount"),
        ),
        (
            "right_stick_down_500ms",
            Stick::down(1.0).expect("runner uses an in-range stick amount"),
        ),
        (
            "right_stick_left_500ms",
            Stick::left(1.0).expect("runner uses an in-range stick amount"),
        ),
    ] {
        run_operation(args, started, controller, operation, |controller| {
            controller.send_right(JoyConRInputState::neutral().with_right_stick(stick))?;
            thread::sleep(STICK_HOLD);
            controller.neutral()?;
            controller.settle();
            Ok(())
        })?;
    }
    run_explicit_neutral(args, started, controller)
}

fn run_explicit_neutral(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut JoyConHardwareController,
) -> Result<(), InputSequenceFailure> {
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
    controller: &mut JoyConHardwareController,
    operation: &'static str,
    action: impl FnOnce(&mut JoyConHardwareController) -> swbt::Result<()>,
) -> Result<(), InputSequenceFailure> {
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
            let status = controller.status();
            let accepted_delta = status
                .input_reports_accepted
                .saturating_sub(accepted_before);
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
                    ("input_reports_accepted_delta", json!(accepted_delta)),
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
            if !status.connected {
                emit_fixed_failure(args, started, operation, "connection_lost");
                return Err(InputSequenceFailure);
            }
            if accepted_delta == 0 {
                emit_fixed_failure(args, started, operation, "no_input_report_accepted");
                return Err(InputSequenceFailure);
            }
            Ok(())
        }
        Err(error) => {
            emit_controller_failure(args, started, operation, &error);
            Err(InputSequenceFailure)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct InputSequenceFailure;

fn close_controller(
    args: &RunnerArgs,
    started: Instant,
    controller: &mut JoyConHardwareController,
) -> bool {
    emit(
        args,
        started,
        "close_start",
        [("with_neutral", json!(true))],
    );
    match controller.close() {
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
    verify_common_adapter_reopen(
        args,
        started,
        || JoyConHardwareController::build(args),
        JoyConHardwareController::open,
        JoyConHardwareController::close_without_neutral,
        |controller| (controller.status(), controller.snapshot_neutral()),
    )
}

fn verify_profile_postflight(
    args: &RunnerArgs,
    started: Instant,
    baseline: &ProfileBaseline,
) -> bool {
    let current = fs::read(&args.profile);
    let (profile_exists, profile_unchanged) = match (&current, baseline) {
        (Ok(bytes), ProfileBaseline::New) => (!bytes.is_empty(), true),
        (Ok(bytes), ProfileBaseline::Existing(original)) => (true, bytes == original),
        (Err(_), _) => (false, false),
    };
    let model_valid = match args.model {
        JoyConModel::Left => JoyConL::builder("adapter-must-not-open")
            .profile_path(args.profile.clone())
            .build()
            .is_ok(),
        JoyConModel::Right => JoyConR::builder("adapter-must-not-open")
            .profile_path(args.profile.clone())
            .build()
            .is_ok(),
    };
    let opposite_rejected = match args.model {
        JoyConModel::Left => match JoyConR::builder("adapter-must-not-open")
            .profile_path(args.profile.clone())
            .build()
        {
            Ok(_) => false,
            Err(error) => error.kind() == ErrorKind::ProfileControllerMismatch,
        },
        JoyConModel::Right => match JoyConL::builder("adapter-must-not-open")
            .profile_path(args.profile.clone())
            .build()
        {
            Ok(_) => false,
            Err(error) => error.kind() == ErrorKind::ProfileControllerMismatch,
        },
    };
    emit(
        args,
        started,
        "profile_postflight",
        [
            ("profile_exists", json!(profile_exists)),
            ("profile_unchanged", json!(profile_unchanged)),
            ("controller_model_valid", json!(model_valid)),
            ("opposite_model_rejected", json!(opposite_rejected)),
            ("raw_profile_emitted", json!(false)),
            ("key_material_emitted", json!(false)),
        ],
    );
    profile_exists && profile_unchanged && model_valid && opposite_rejected
}

fn emit_status(
    args: &RunnerArgs,
    started: Instant,
    event: &'static str,
    operation: Option<&'static str>,
    controller: &JoyConHardwareController,
) {
    let status = controller.status();
    emit_common_status(
        args,
        started,
        event,
        operation,
        &status,
        controller.snapshot_neutral(),
    );
}

#[cfg(test)]
mod tests {
    use std::{path::Path, time::Duration};

    use serde_json::json;

    use crate::support::{UsageError, evidence_event};

    use super::{
        ConnectionOperation, EVIDENCE_SCHEMA, JoyConModel, RunnerArgs, RunnerMode, parse_args,
    };

    #[test]
    fn runner_requires_explicit_model_reporting_connection_and_redacts_local_inputs() {
        let args = parse_args([
            "--adapter",
            "usb:0a12:0001",
            "--profile",
            "joycon-left.json",
            "--model",
            "left",
            "--mode",
            "periodic",
            "--connection",
            "pair",
            "--timeout-secs",
            "60",
            "--pre-input-idle-ms",
            "3000",
            "--run",
            "7",
        ])
        .expect("parse explicit Joy-Con hardware inputs");

        assert_eq!(args.profile, Path::new("joycon-left.json"));
        assert_eq!(args.model, JoyConModel::Left);
        assert_eq!(args.mode, RunnerMode::Periodic);
        assert_eq!(args.connection, ConnectionOperation::Pair);
        assert_eq!(args.timeout, Duration::from_secs(60));
        assert_eq!(args.pre_input_idle, Duration::from_secs(3));
        assert_eq!(args.run_index, 7);
        let rendered = format!("{args:?}");
        assert!(!rendered.contains("usb:0a12:0001"));
        assert!(!rendered.contains("joycon-left.json"));
    }

    #[test]
    fn pairing_rejects_direct_mode_and_missing_inputs() {
        let direct_pair = [
            "--adapter",
            "usb:0a12:0001",
            "--profile",
            "joycon-right.json",
            "--model",
            "right",
            "--mode",
            "direct",
            "--connection",
            "pair",
            "--timeout-secs",
            "60",
            "--run",
            "8",
        ];
        assert_eq!(
            parse_args(direct_pair).expect_err("Direct pairing is outside the M7 runner contract"),
            UsageError("pair requires periodic mode")
        );
        assert!(parse_args(["--model", "left"]).is_err());
    }

    #[test]
    fn evidence_identifies_the_model_and_path_without_local_or_key_material() {
        let args = RunnerArgs {
            adapter: "secret-adapter".to_owned(),
            profile: "secret-profile".into(),
            model: JoyConModel::Right,
            mode: RunnerMode::Direct,
            connection: ConnectionOperation::Reconnect,
            timeout: Duration::from_secs(10),
            pre_input_idle: Duration::ZERO,
            run_index: 4,
        };
        let event = evidence_event(
            &args,
            25,
            "runner_start",
            [
                ("raw_profile_emitted", json!(false)),
                ("key_material_emitted", json!(false)),
            ],
        );
        let encoded = event.to_string();

        assert_eq!(event["schema"], EVIDENCE_SCHEMA);
        assert_eq!(event["schema_version"], 1);
        assert_eq!(event["model"], "joycon_r");
        assert_eq!(event["mode"], "direct");
        assert_eq!(event["connection"], "reconnect");
        assert!(!encoded.contains("profile.json"));
        assert!(!encoded.contains("C7C7"));
    }
}
