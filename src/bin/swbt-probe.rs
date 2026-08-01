use std::{
    ffi::OsString,
    fs, io,
    path::PathBuf,
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use swbt::{
    ButtonKind, Controller, ControllerKind, CreateProfileOptions, DirectProController, ErrorKind,
    ImuFrame, InputState, LocalAddress, ProfileIdentity, ProfileIdentityKind, ProfileSummary,
    inspect_profile, list_adapters,
};
use swbt::{
    model::{self, ControllerModel},
    reporting::{self, ReportingMode},
};

#[path = "swbt-probe/trace.rs"]
mod trace;

use trace::TraceSession;

const PROBE_SCHEMA: &str = "swbt.probe";
const PROBE_SCHEMA_VERSION: u64 = 1;
const EXIT_OPERATION_ERROR: u8 = 1;
const EXIT_USAGE: u8 = 2;
const DEFAULT_ADAPTER: &str = "usb:0";
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);
const BUTTON_TAP_DURATION: Duration = Duration::from_millis(100);
const MIN_IMU_RUN_SECS: u64 = 1;
const MAX_IMU_RUN_SECS: u64 = 3600;
const HELP: &str = "\
Usage:
  swbt-probe adapters
  swbt-probe open --adapter <selector>
  swbt-probe pair --controller <pro|joycon-l|joycon-r> --profile <path> --trace <path> [--local-address <XX:XX:XX:XX:XX:XX>] [--button <button>]
  swbt-probe reconnect --controller <pro|joycon-l|joycon-r> --profile <path> --trace <path> [--reporting <periodic|direct>] [--button <button>] [--imu-seconds <1..3600>]
  swbt-probe profile inspect <path>
  swbt-probe profile verify <path>
  swbt-probe help
";

fn main() -> ExitCode {
    let mut backend = SystemBackend;
    match parse(std::env::args_os().skip(1).collect()) {
        Ok(Command::Help) => match write_text(io::stdout().lock(), HELP) {
            Ok(()) => ExitCode::SUCCESS,
            Err(()) => operation_write_failure(),
        },
        Ok(command) => match execute(command, &mut backend) {
            Ok(record) => match write_record(io::stdout().lock(), &record) {
                Ok(()) => ExitCode::SUCCESS,
                Err(()) => operation_write_failure(),
            },
            Err(kind) => {
                let _ = write_record(io::stderr().lock(), &error_record(error_kind_name(kind)));
                ExitCode::from(EXIT_OPERATION_ERROR)
            }
        },
        Err(()) => {
            let _ = write_record(io::stderr().lock(), &usage_error_record());
            ExitCode::from(EXIT_USAGE)
        }
    }
}

enum Command {
    Help,
    Adapters,
    Open(String),
    Connection(ConnectionRequest),
    ProfileInspect(PathBuf),
    ProfileVerify(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConnectionOperation {
    Pair,
    Reconnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ControllerSelection {
    Pro,
    JoyConL,
    JoyConR,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReportingSelection {
    Periodic,
    Direct,
}

struct ConnectionRequest {
    operation: ConnectionOperation,
    controller: ControllerSelection,
    reporting: ReportingSelection,
    identity: ProfileIdentity,
    profile: PathBuf,
    trace: PathBuf,
    button: Option<ButtonKind>,
    imu_duration: Option<Duration>,
}

fn parse(arguments: Vec<OsString>) -> Result<Command, ()> {
    if let Some(operation) = arguments.first().and_then(parse_connection_operation) {
        return parse_connection(operation, &arguments[1..]).map(Command::Connection);
    }

    match arguments.as_slice() {
        [command] if matches!(command.to_str(), Some("help" | "--help" | "-h")) => {
            Ok(Command::Help)
        }
        [command] if command == "adapters" => Ok(Command::Adapters),
        [command, option, selector] if command == "open" && option == "--adapter" => selector
            .to_str()
            .map(str::to_owned)
            .map(Command::Open)
            .ok_or(()),
        [profile, action, path] if profile == "profile" && action == "inspect" => {
            Ok(Command::ProfileInspect(PathBuf::from(path)))
        }
        [profile, action, path] if profile == "profile" && action == "verify" => {
            Ok(Command::ProfileVerify(PathBuf::from(path)))
        }
        _ => Err(()),
    }
}

fn parse_connection_operation(value: &OsString) -> Option<ConnectionOperation> {
    match value.to_str()? {
        "pair" => Some(ConnectionOperation::Pair),
        "reconnect" => Some(ConnectionOperation::Reconnect),
        _ => None,
    }
}

fn parse_connection(
    operation: ConnectionOperation,
    arguments: &[OsString],
) -> Result<ConnectionRequest, ()> {
    if !arguments.len().is_multiple_of(2) {
        return Err(());
    }

    let mut controller = None;
    let mut profile = None;
    let mut trace = None;
    let mut reporting = None;
    let mut identity = None;
    let mut button = None;
    let mut imu_duration = None;
    for option in arguments.chunks_exact(2) {
        match option[0].to_str() {
            Some("--controller") => set_once(
                &mut controller,
                option[1]
                    .to_str()
                    .and_then(parse_controller_selection)
                    .ok_or(())?,
            )?,
            Some("--profile") => set_once(&mut profile, PathBuf::from(&option[1]))?,
            Some("--trace") => set_once(&mut trace, PathBuf::from(&option[1]))?,
            Some("--reporting") => set_once(
                &mut reporting,
                option[1]
                    .to_str()
                    .and_then(parse_reporting_selection)
                    .ok_or(())?,
            )?,
            Some("--local-address") => set_once(
                &mut identity,
                option[1]
                    .to_str()
                    .and_then(|value| LocalAddress::parse(value).ok())
                    .map(ProfileIdentity::LocalAddress)
                    .ok_or(())?,
            )?,
            Some("--button") => set_once(
                &mut button,
                option[1].to_str().and_then(parse_button_kind).ok_or(())?,
            )?,
            Some("--imu-seconds") => {
                let seconds = option[1]
                    .to_str()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|seconds| (MIN_IMU_RUN_SECS..=MAX_IMU_RUN_SECS).contains(seconds))
                    .ok_or(())?;
                set_once(&mut imu_duration, Duration::from_secs(seconds))?;
            }
            _ => return Err(()),
        }
    }

    let reporting = match (operation, reporting) {
        (ConnectionOperation::Pair, None) => ReportingSelection::Periodic,
        (ConnectionOperation::Pair, Some(_)) => return Err(()),
        (ConnectionOperation::Reconnect, reporting) => {
            reporting.unwrap_or(ReportingSelection::Periodic)
        }
    };
    let identity = match (operation, identity) {
        (ConnectionOperation::Pair, identity) => {
            identity.unwrap_or(ProfileIdentity::AdapterDefault)
        }
        (ConnectionOperation::Reconnect, None) => ProfileIdentity::AdapterDefault,
        (ConnectionOperation::Reconnect, Some(_)) => return Err(()),
    };
    let controller = controller.ok_or(())?;
    if imu_duration.is_some()
        && (operation != ConnectionOperation::Reconnect
            || controller != ControllerSelection::Pro
            || reporting != ReportingSelection::Periodic)
    {
        return Err(());
    }
    Ok(ConnectionRequest {
        operation,
        controller,
        reporting,
        identity,
        profile: profile.ok_or(())?,
        trace: trace.ok_or(())?,
        button,
        imu_duration,
    })
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), ()> {
    if slot.is_some() {
        return Err(());
    }
    *slot = Some(value);
    Ok(())
}

const fn parse_controller_selection(value: &str) -> Option<ControllerSelection> {
    match value.as_bytes() {
        b"pro" => Some(ControllerSelection::Pro),
        b"joycon-l" => Some(ControllerSelection::JoyConL),
        b"joycon-r" => Some(ControllerSelection::JoyConR),
        _ => None,
    }
}

const fn parse_reporting_selection(value: &str) -> Option<ReportingSelection> {
    match value.as_bytes() {
        b"periodic" => Some(ReportingSelection::Periodic),
        b"direct" => Some(ReportingSelection::Direct),
        _ => None,
    }
}

const fn parse_button_kind(value: &str) -> Option<ButtonKind> {
    match value.as_bytes() {
        b"a" => Some(ButtonKind::A),
        b"b" => Some(ButtonKind::B),
        b"x" => Some(ButtonKind::X),
        b"y" => Some(ButtonKind::Y),
        b"l" => Some(ButtonKind::L),
        b"r" => Some(ButtonKind::R),
        b"zl" => Some(ButtonKind::ZL),
        b"zr" => Some(ButtonKind::ZR),
        b"plus" => Some(ButtonKind::Plus),
        b"minus" => Some(ButtonKind::Minus),
        b"home" => Some(ButtonKind::Home),
        b"capture" => Some(ButtonKind::Capture),
        b"left-stick" => Some(ButtonKind::LeftStick),
        b"right-stick" => Some(ButtonKind::RightStick),
        b"sl" => Some(ButtonKind::SL),
        b"sr" => Some(ButtonKind::SR),
        b"dpad-up" => Some(ButtonKind::DpadUp),
        b"dpad-down" => Some(ButtonKind::DpadDown),
        b"dpad-left" => Some(ButtonKind::DpadLeft),
        b"dpad-right" => Some(ButtonKind::DpadRight),
        _ => None,
    }
}

fn execute(command: Command, backend: &mut impl ProbeBackend) -> Result<Value, ErrorKind> {
    match command {
        Command::Help => unreachable!("help bypasses command execution"),
        Command::Adapters => backend.list_adapters().map(adapters_listed_record),
        Command::Open(selector) => backend
            .open_adapter(&selector)
            .map(|()| adapter_opened_record()),
        Command::Connection(request) => dispatch_connection(&request, backend)
            .map(|evidence| connection_completed_record(&request, evidence)),
        Command::ProfileInspect(path) => inspect_profile(path)
            .map(profile_inspected_record)
            .map_err(|error| error.kind()),
        Command::ProfileVerify(path) => inspect_profile(path)
            .map(profile_verified_record)
            .map_err(|error| error.kind()),
    }
}

fn dispatch_connection(
    request: &ConnectionRequest,
    backend: &mut impl ProbeBackend,
) -> Result<ConnectionEvidence, ErrorKind> {
    match request.controller {
        ControllerSelection::Pro => dispatch_model::<model::Pro>(request, backend),
        ControllerSelection::JoyConL => dispatch_model::<model::JoyConL>(request, backend),
        ControllerSelection::JoyConR => dispatch_model::<model::JoyConR>(request, backend),
    }
}

fn dispatch_model<M: ControllerModel>(
    request: &ConnectionRequest,
    backend: &mut impl ProbeBackend,
) -> Result<ConnectionEvidence, ErrorKind> {
    match request.operation {
        ConnectionOperation::Pair => backend.pair::<M>(request),
        ConnectionOperation::Reconnect => match request.reporting {
            ReportingSelection::Periodic => backend.reconnect::<M, reporting::Periodic>(request),
            ReportingSelection::Direct => backend.reconnect::<M, reporting::Direct>(request),
        },
    }
}

struct SafeAdapter {
    vendor_id: u16,
    product_id: u16,
}

struct ConnectionEvidence {
    identity_kind: ProfileIdentityKind,
    imu: Option<ImuRunEvidence>,
    shutdown_latency_ns: Option<u64>,
    neutral_close: bool,
    profile_unchanged: Option<bool>,
    adapter_reopened: Option<bool>,
}

impl Default for ConnectionEvidence {
    fn default() -> Self {
        Self {
            identity_kind: ProfileIdentityKind::AdapterDefault,
            imu: None,
            shutdown_latency_ns: None,
            neutral_close: false,
            profile_unchanged: None,
            adapter_reopened: None,
        }
    }
}

struct ImuRunEvidence {
    duration_seconds: u64,
    apply_command_latency_ns: u64,
    non_neutral_reports_accepted: u64,
    neutral_reports_accepted: u64,
}

trait ProbeBackend {
    fn list_adapters(&mut self) -> Result<Vec<SafeAdapter>, ErrorKind>;
    fn open_adapter(&mut self, selector: &str) -> Result<(), ErrorKind>;
    fn pair<M: ControllerModel>(
        &mut self,
        request: &ConnectionRequest,
    ) -> Result<ConnectionEvidence, ErrorKind>;
    fn reconnect<M: ControllerModel, R: ProbeReporting<M>>(
        &mut self,
        request: &ConnectionRequest,
    ) -> Result<ConnectionEvidence, ErrorKind>;
}

struct SystemBackend;

impl ProbeBackend for SystemBackend {
    fn list_adapters(&mut self) -> Result<Vec<SafeAdapter>, ErrorKind> {
        list_adapters()
            .map(|adapters| {
                adapters
                    .into_iter()
                    .map(|adapter| SafeAdapter {
                        vendor_id: adapter.vendor_id(),
                        product_id: adapter.product_id(),
                    })
                    .collect()
            })
            .map_err(|error| error.kind())
    }

    fn open_adapter(&mut self, selector: &str) -> Result<(), ErrorKind> {
        let mut controller = DirectProController::builder(selector)
            .build()
            .map_err(|error| error.kind())?;
        open_and_close(&mut controller)
    }

    fn pair<M: ControllerModel>(
        &mut self,
        request: &ConnectionRequest,
    ) -> Result<ConnectionEvidence, ErrorKind> {
        let trace = TraceSession::install(&request.trace)?;
        trace::emit_environment::<M, reporting::Periodic>();
        let operation = (|| {
            let mut controller = Controller::<M, reporting::Periodic>::builder(DEFAULT_ADAPTER)
                .profile_path(&request.profile)
                .create_profile(CreateProfileOptions {
                    identity: request.identity,
                    pair_timeout: CONNECTION_TIMEOUT,
                })
                .map_err(|error| error.kind())?;
            apply_button_and_close(&mut controller, request.button)?;
            Ok(ConnectionEvidence {
                identity_kind: profile_identity_kind(request.identity),
                neutral_close: true,
                ..ConnectionEvidence::default()
            })
        })();
        finish_trace(trace, operation)
    }

    fn reconnect<M: ControllerModel, R: ProbeReporting<M>>(
        &mut self,
        request: &ConnectionRequest,
    ) -> Result<ConnectionEvidence, ErrorKind> {
        let trace = TraceSession::install(&request.trace)?;
        trace::emit_environment::<M, R>();
        let operation = (|| {
            let identity_kind = inspect_profile(&request.profile)
                .map_err(|error| error.kind())?
                .identity_kind();
            let profile_before = request
                .imu_duration
                .map(|_| read_profile_bytes(&request.profile))
                .transpose()?;
            let mut controller = Controller::<M, R>::builder(DEFAULT_ADAPTER)
                .profile_path(&request.profile)
                .build()
                .map_err(|error| error.kind())?;
            controller.open().map_err(|error| error.kind())?;
            let connection = controller
                .reconnect(CONNECTION_TIMEOUT)
                .map_err(|error| error.kind())
                .and_then(|()| R::run_ready(&mut controller, request));
            let shutdown_started = Instant::now();
            let imu = finish_connection(&mut controller, connection)?;
            let mut evidence = ConnectionEvidence {
                identity_kind,
                imu,
                shutdown_latency_ns: Some(duration_ns(shutdown_started.elapsed())),
                neutral_close: true,
                ..ConnectionEvidence::default()
            };
            if let Some(profile_before) = profile_before {
                let profile_after = read_profile_bytes(&request.profile)?;
                if profile_after != profile_before {
                    return Err(ErrorKind::InvalidProfile);
                }
                reopen_after_connection::<M, R>(request)?;
                if read_profile_bytes(&request.profile)? != profile_before {
                    return Err(ErrorKind::InvalidProfile);
                }
                evidence.profile_unchanged = Some(true);
                evidence.adapter_reopened = Some(true);
            }
            Ok(evidence)
        })();
        finish_trace(trace, operation)
    }
}

fn read_profile_bytes(path: &std::path::Path) -> Result<Vec<u8>, ErrorKind> {
    fs::read(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => ErrorKind::ProfileNotFound,
        _ => ErrorKind::InvalidProfile,
    })
}

trait ProbeReporting<M: ControllerModel>: ReportingMode {
    fn run_ready(
        controller: &mut Controller<M, Self>,
        request: &ConnectionRequest,
    ) -> Result<Option<ImuRunEvidence>, ErrorKind>
    where
        Self: Sized;
}

impl<M: ControllerModel> ProbeReporting<M> for reporting::Periodic {
    fn run_ready(
        controller: &mut Controller<M, Self>,
        request: &ConnectionRequest,
    ) -> Result<Option<ImuRunEvidence>, ErrorKind> {
        apply_button(controller, request.button)?;
        request
            .imu_duration
            .map(|duration| run_periodic_imu(controller, duration))
            .transpose()
    }
}

impl<M: ControllerModel> ProbeReporting<M> for reporting::Direct {
    fn run_ready(
        controller: &mut Controller<M, Self>,
        request: &ConnectionRequest,
    ) -> Result<Option<ImuRunEvidence>, ErrorKind> {
        if request.imu_duration.is_some() {
            return Err(ErrorKind::UnsupportedCapability);
        }
        apply_button(controller, request.button)?;
        Ok(None)
    }
}

fn run_periodic_imu<M: ControllerModel>(
    controller: &mut Controller<M, reporting::Periodic>,
    duration: Duration,
) -> Result<ImuRunEvidence, ErrorKind> {
    let frame = horizontal_yaw_frame().map_err(|error| error.kind())?;
    let started = Instant::now();
    let apply_started = Instant::now();
    controller
        .apply(InputState::neutral().with_imu(frame))
        .map_err(|error| error.kind())?;
    let apply_command_latency_ns = duration_ns(apply_started.elapsed());
    let non_neutral_baseline = controller.status().input_reports_accepted;
    wait_for_periodic_report(controller, non_neutral_baseline)?;
    thread::sleep(duration.saturating_sub(started.elapsed()));
    let non_neutral_reports_accepted = controller
        .status()
        .input_reports_accepted
        .saturating_sub(non_neutral_baseline);
    controller.neutral().map_err(|error| error.kind())?;
    let neutral_baseline = controller.status().input_reports_accepted;
    wait_for_periodic_report(controller, neutral_baseline)?;
    let neutral_reports_accepted = controller
        .status()
        .input_reports_accepted
        .saturating_sub(neutral_baseline);
    Ok(ImuRunEvidence {
        duration_seconds: duration.as_secs(),
        apply_command_latency_ns,
        non_neutral_reports_accepted,
        neutral_reports_accepted,
    })
}

fn horizontal_yaw_frame() -> swbt::Result<ImuFrame> {
    ImuFrame::accel_g(0.0, 0.0, 1.0).and_then(|frame| frame.with_gyro_rate(0.0, 0.0, 1.0))
}

fn wait_for_periodic_report<M: ControllerModel>(
    controller: &Controller<M, reporting::Periodic>,
    accepted_before: u64,
) -> Result<(), ErrorKind> {
    let deadline = Instant::now()
        + controller
            .report_period()
            .saturating_mul(4)
            .max(Duration::from_secs(1));
    while Instant::now() < deadline {
        if controller.status().input_reports_accepted > accepted_before {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(1));
    }
    Err(ErrorKind::WorkerFailed)
}

fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

fn reopen_after_connection<M: ControllerModel, R: ReportingMode>(
    request: &ConnectionRequest,
) -> Result<(), ErrorKind> {
    let mut controller = Controller::<M, R>::builder(DEFAULT_ADAPTER)
        .profile_path(&request.profile)
        .build()
        .map_err(|error| error.kind())?;
    controller.open().map_err(|error| error.kind())?;
    controller
        .close_without_neutral()
        .map_err(|error| error.kind())
}

fn finish_trace<T>(trace: TraceSession, operation: Result<T, ErrorKind>) -> Result<T, ErrorKind> {
    match trace.finish() {
        Ok(()) => operation,
        Err(error) => Err(error),
    }
}

fn apply_button_and_close<M: ControllerModel, R: ReportingMode>(
    controller: &mut Controller<M, R>,
    button: Option<ButtonKind>,
) -> Result<(), ErrorKind> {
    let operation = apply_button(controller, button);
    finish_connection(controller, operation)
}

fn apply_button<M: ControllerModel, R: ReportingMode>(
    controller: &mut Controller<M, R>,
    button: Option<ButtonKind>,
) -> Result<(), ErrorKind> {
    let Some(kind) = button else {
        return Ok(());
    };
    let button = controller.button(kind).map_err(|error| error.kind())?;
    controller
        .tap([button], BUTTON_TAP_DURATION)
        .map_err(|error| error.kind())
}

fn finish_connection<M: ControllerModel, R: ReportingMode, T>(
    controller: &mut Controller<M, R>,
    operation: Result<T, ErrorKind>,
) -> Result<T, ErrorKind> {
    let close = controller.close().map_err(|error| error.kind());
    match (operation, close) {
        (Err(primary), _) => Err(primary),
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
    }
}

trait ProbeController {
    fn open(&mut self) -> Result<(), ErrorKind>;
    fn close(&mut self) -> Result<(), ErrorKind>;
}

impl ProbeController for DirectProController {
    fn open(&mut self) -> Result<(), ErrorKind> {
        self.open().map_err(|error| error.kind())
    }

    fn close(&mut self) -> Result<(), ErrorKind> {
        self.close_without_neutral().map_err(|error| error.kind())
    }
}

fn open_and_close(controller: &mut impl ProbeController) -> Result<(), ErrorKind> {
    controller.open()?;
    controller.close()
}

fn adapters_listed_record(adapters: Vec<SafeAdapter>) -> Value {
    let adapter_count = adapters.len();
    let adapters = adapters
        .into_iter()
        .map(|adapter| {
            json!({
                "vendor_id": adapter.vendor_id,
                "product_id": adapter.product_id,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "schema": PROBE_SCHEMA,
        "schema_version": PROBE_SCHEMA_VERSION,
        "event": "adapters_listed",
        "adapter_count": adapter_count,
        "adapters": adapters,
    })
}

fn adapter_opened_record() -> Value {
    json!({
        "schema": PROBE_SCHEMA,
        "schema_version": PROBE_SCHEMA_VERSION,
        "event": "adapter_opened",
    })
}

fn connection_completed_record(request: &ConnectionRequest, evidence: ConnectionEvidence) -> Value {
    let imu = evidence.imu;
    json!({
        "schema": PROBE_SCHEMA,
        "schema_version": PROBE_SCHEMA_VERSION,
        "event": "connection_completed",
        "operation": connection_operation_name(request.operation),
        "controller_kind": controller_kind(request.controller).profile_name(),
        "reporting_kind": reporting_name(request.reporting),
        "identity_kind": identity_kind_name(evidence.identity_kind),
        "imu_run_seconds": imu.as_ref().map(|evidence| evidence.duration_seconds),
        "imu_apply_command_latency_ns": imu.as_ref().map(|evidence| evidence.apply_command_latency_ns),
        "imu_non_neutral_reports_accepted": imu.as_ref().map(|evidence| evidence.non_neutral_reports_accepted),
        "neutral_reports_accepted": imu.as_ref().map(|evidence| evidence.neutral_reports_accepted),
        "shutdown_latency_ns": evidence.shutdown_latency_ns,
        "neutral_close": evidence.neutral_close,
        "profile_unchanged": evidence.profile_unchanged,
        "adapter_reopened": evidence.adapter_reopened,
    })
}

const fn connection_operation_name(operation: ConnectionOperation) -> &'static str {
    match operation {
        ConnectionOperation::Pair => "pair",
        ConnectionOperation::Reconnect => "reconnect",
    }
}

const fn controller_kind(controller: ControllerSelection) -> ControllerKind {
    match controller {
        ControllerSelection::Pro => ControllerKind::Pro,
        ControllerSelection::JoyConL => ControllerKind::JoyConL,
        ControllerSelection::JoyConR => ControllerKind::JoyConR,
    }
}

const fn reporting_name(reporting: ReportingSelection) -> &'static str {
    match reporting {
        ReportingSelection::Periodic => "periodic",
        ReportingSelection::Direct => "direct",
    }
}

fn profile_inspected_record(summary: ProfileSummary) -> Value {
    json!({
        "schema": PROBE_SCHEMA,
        "schema_version": PROBE_SCHEMA_VERSION,
        "event": "profile_inspected",
        "profile_schema_version": summary.schema_version(),
        "controller_kind": summary.controller_kind().profile_name(),
        "identity_kind": identity_kind_name(summary.identity_kind()),
        "namespace_count": summary.namespace_count(),
        "bond_count": summary.bond_count(),
    })
}

fn profile_verified_record(summary: ProfileSummary) -> Value {
    json!({
        "schema": PROBE_SCHEMA,
        "schema_version": PROBE_SCHEMA_VERSION,
        "event": "profile_verified",
        "controller_kind": summary.controller_kind().profile_name(),
        "valid": true,
    })
}

const fn identity_kind_name(kind: ProfileIdentityKind) -> &'static str {
    match kind {
        ProfileIdentityKind::AdapterDefault => "adapter_default",
        ProfileIdentityKind::LocalAddress => "local_address",
        _ => "unknown",
    }
}

const fn profile_identity_kind(identity: ProfileIdentity) -> ProfileIdentityKind {
    match identity {
        ProfileIdentity::AdapterDefault => ProfileIdentityKind::AdapterDefault,
        ProfileIdentity::LocalAddress(_) => ProfileIdentityKind::LocalAddress,
    }
}

const fn error_kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::AdapterDiscovery => "adapter_discovery",
        ErrorKind::TransportOpen => "transport_open",
        ErrorKind::AdapterIdentityRecoveryRequired => "adapter_identity_recovery_required",
        ErrorKind::Trace => "trace",
        ErrorKind::ProfilePathRequired => "profile_path_required",
        ErrorKind::ProfileNotFound => "profile_not_found",
        ErrorKind::ProfileAlreadyExists => "profile_already_exists",
        ErrorKind::InvalidProfile => "invalid_profile",
        ErrorKind::ProfileControllerMismatch => "profile_controller_mismatch",
        ErrorKind::InvalidKeyStore => "invalid_key_store",
        ErrorKind::NoBond => "no_bond",
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
        _ => "operation_failed",
    }
}

fn error_record(kind: &'static str) -> Value {
    json!({
        "schema": PROBE_SCHEMA,
        "schema_version": PROBE_SCHEMA_VERSION,
        "event": "error",
        "error_kind": kind,
    })
}

fn usage_error_record() -> Value {
    let mut record = error_record("usage");
    record
        .as_object_mut()
        .expect("error record is an object")
        .insert("usage".to_owned(), json!("swbt-probe help"));
    record
}

fn write_record(mut writer: impl io::Write, record: &Value) -> Result<(), ()> {
    serde_json::to_writer(&mut writer, record).map_err(|_| ())?;
    writer.write_all(b"\n").map_err(|_| ())
}

fn write_text(mut writer: impl io::Write, text: &str) -> Result<(), ()> {
    writer.write_all(text.as_bytes()).map_err(|_| ())
}

fn operation_write_failure() -> ExitCode {
    let _ = write_record(io::stderr().lock(), &error_record("output_write_failed"));
    ExitCode::from(EXIT_OPERATION_ERROR)
}

#[cfg(test)]
#[path = "swbt-probe/tests.rs"]
mod tests;
