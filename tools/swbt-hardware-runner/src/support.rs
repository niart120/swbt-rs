use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde_json::{Map, Value, json};
use swbt::{Error, ErrorKind, GamepadStatus, LifecycleState, ReportingKind};

const EVIDENCE_SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UsageError(pub(crate) &'static str);

impl fmt::Display for UsageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for UsageError {}

pub(crate) struct KeyValueArguments {
    values: BTreeMap<String, String>,
}

impl KeyValueArguments {
    pub(crate) fn parse<I, S>(arguments: I, allowed: &[&str]) -> Result<Self, UsageError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut values = BTreeMap::new();
        let mut arguments = arguments.into_iter().map(Into::into);
        while let Some(flag) = arguments.next() {
            let value = arguments
                .next()
                .ok_or(UsageError("argument value is missing"))?;
            if !allowed.contains(&flag.as_str()) {
                return Err(UsageError("unknown argument"));
            }
            if values.insert(flag, value).is_some() {
                return Err(UsageError("duplicate argument"));
            }
        }
        Ok(Self { values })
    }

    pub(crate) fn required(
        &mut self,
        flag: &str,
        missing: &'static str,
    ) -> Result<String, UsageError> {
        self.values.remove(flag).ok_or(UsageError(missing))
    }

    pub(crate) fn optional(&mut self, flag: &str) -> Option<String> {
        self.values.remove(flag)
    }
}

pub(crate) trait EvidenceTarget {
    fn evidence_schema(&self) -> &'static str;

    fn evidence_dimensions(&self) -> Vec<(&'static str, Value)>;

    fn error_kind_name(&self, kind: ErrorKind) -> &'static str {
        error_kind_name(kind)
    }
}

pub(crate) fn emit<T, I>(target: &T, started: Instant, event: &'static str, fields: I)
where
    T: EvidenceTarget,
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let value = evidence_event(target, duration_ms(started.elapsed()), event, fields);
    let encoded = serde_json::to_string(&value).expect("hardware evidence event must serialize");
    let mut output = io::stdout().lock();
    writeln!(output, "{encoded}").expect("hardware evidence event must be written");
    output
        .flush()
        .expect("hardware evidence event must be flushed");
}

pub(crate) fn evidence_event<T, I>(
    target: &T,
    elapsed_ms: u64,
    event: &'static str,
    fields: I,
) -> Value
where
    T: EvidenceTarget,
    I: IntoIterator<Item = (&'static str, Value)>,
{
    let mut object = Map::from_iter([
        ("schema".into(), json!(target.evidence_schema())),
        ("schema_version".into(), json!(EVIDENCE_SCHEMA_VERSION)),
        ("unix_time_ms".into(), json!(unix_time_ms())),
        ("elapsed_ms".into(), json!(elapsed_ms)),
        ("event".into(), json!(event)),
    ]);
    for (name, value) in target.evidence_dimensions().into_iter().chain(fields) {
        object.entry(name).or_insert(value);
    }
    Value::Object(object)
}

pub(crate) fn emit_status<T: EvidenceTarget>(
    target: &T,
    started: Instant,
    event: &'static str,
    operation: Option<&'static str>,
    status: &GamepadStatus,
    snapshot_neutral: bool,
) {
    let mut fields = status_fields(status);
    fields.push(("snapshot_neutral", json!(snapshot_neutral)));
    if let Some(operation) = operation {
        fields.push(("operation", json!(operation)));
    }
    emit(target, started, event, fields);
}

pub(crate) fn emit_controller_failure<T: EvidenceTarget>(
    target: &T,
    started: Instant,
    operation: &'static str,
    error: &Error,
) {
    emit(
        target,
        started,
        "operation_failure",
        [
            ("operation", json!(operation)),
            ("error_kind", json!(target.error_kind_name(error.kind()))),
            (
                "related_failure_present",
                json!(error.related_error().is_some()),
            ),
        ],
    );
}

pub(crate) fn emit_fixed_failure<T: EvidenceTarget>(
    target: &T,
    started: Instant,
    operation: &'static str,
    error_kind: &'static str,
) {
    emit(
        target,
        started,
        "operation_failure",
        [
            ("operation", json!(operation)),
            ("error_kind", json!(error_kind)),
            ("related_failure_present", json!(false)),
        ],
    );
}

pub(crate) fn emit_completion<T: EvidenceTarget>(target: &T, started: Instant, success: bool) {
    emit(
        target,
        started,
        "runner_complete",
        [("success", json!(success))],
    );
}

pub(crate) fn verify_adapter_reopen<T, C>(
    target: &T,
    started: Instant,
    build: impl FnOnce() -> swbt::Result<C>,
    open: impl FnOnce(&mut C) -> swbt::Result<()>,
    close_without_neutral: impl FnOnce(&mut C) -> swbt::Result<()>,
    status: impl FnOnce(&C) -> (GamepadStatus, bool),
) -> bool
where
    T: EvidenceTarget,
{
    emit(target, started, "adapter_reopen_start", []);
    let mut controller = match build() {
        Ok(controller) => controller,
        Err(error) => {
            emit_controller_failure(target, started, "adapter_reopen_build", &error);
            return false;
        }
    };
    if let Err(error) = open(&mut controller) {
        emit_controller_failure(target, started, "adapter_reopen_open", &error);
        return false;
    }
    if let Err(error) = close_without_neutral(&mut controller) {
        emit_controller_failure(target, started, "adapter_reopen_close", &error);
        return false;
    }
    let (controller_status, snapshot_neutral) = status(&controller);
    emit_status(
        target,
        started,
        "adapter_reopen_complete",
        None,
        &controller_status,
        snapshot_neutral,
    );
    true
}

pub(crate) fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn unix_time_ms() -> u64 {
    duration_ms(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO),
    )
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

pub(crate) fn error_kind_name(kind: ErrorKind) -> &'static str {
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
    use super::{KeyValueArguments, UsageError};

    #[test]
    fn key_value_arguments_reject_missing_unknown_and_duplicate_inputs() {
        assert!(matches!(
            KeyValueArguments::parse(["--known"], &["--known"]),
            Err(UsageError("argument value is missing"))
        ));
        assert!(matches!(
            KeyValueArguments::parse(["--unknown", "value"], &["--known"]),
            Err(UsageError("unknown argument"))
        ));
        assert!(matches!(
            KeyValueArguments::parse(["--known", "first", "--known", "second"], &["--known"]),
            Err(UsageError("duplicate argument"))
        ));
    }
}
