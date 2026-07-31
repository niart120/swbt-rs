use std::{
    fs::OpenOptions,
    io::{BufWriter, Write as _},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

use serde_json::{Map, Number, Value};
use swbt::{ErrorKind, model::ControllerModel, reporting::ReportingMode};
use tracing::{
    Event, Metadata, Subscriber,
    field::{Field, Visit},
    level_filters::LevelFilter,
    span::{Attributes, Id, Record},
};

const TARGET: &str = "swbt::diagnostics";
const SCHEMA: &str = "swbt.diagnostics";
const SCHEMA_VERSION: u64 = 1;

pub(super) struct TraceSession {
    state: Arc<TraceState>,
}

impl TraceSession {
    pub(super) fn install(path: &Path) -> Result<Self, ErrorKind> {
        let (session, subscriber) = Self::create(path)?;
        tracing::subscriber::set_global_default(subscriber).map_err(|_| ErrorKind::Trace)?;
        Ok(session)
    }

    fn create(path: &Path) -> Result<(Self, NdjsonSubscriber), ErrorKind> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| ErrorKind::Trace)?;
        let state = Arc::new(TraceState {
            output: Mutex::new(BufWriter::new(file)),
            failed: AtomicBool::new(false),
            started: Instant::now(),
        });
        Ok((
            Self {
                state: Arc::clone(&state),
            },
            NdjsonSubscriber { state },
        ))
    }

    pub(super) fn finish(self) -> Result<(), ErrorKind> {
        let flushed = self
            .state
            .output
            .lock()
            .map_err(|_| ErrorKind::Trace)?
            .flush()
            .is_ok();
        if flushed && !self.state.failed.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ErrorKind::Trace)
        }
    }
}

pub(super) fn emit_environment<M: ControllerModel, R: ReportingMode>() {
    tracing::event!(
        target: TARGET,
        tracing::Level::INFO,
        schema = SCHEMA,
        schema_version = SCHEMA_VERSION,
        event = "environment",
        controller_kind = M::KIND.profile_name(),
        reporting_kind = reporting_kind_name(R::KIND),
        package_version = env!("CARGO_PKG_VERSION"),
        target_os = std::env::consts::OS,
        target_arch = std::env::consts::ARCH,
    );
}

const fn reporting_kind_name(kind: swbt::ReportingKind) -> &'static str {
    match kind {
        swbt::ReportingKind::Periodic => "periodic",
        swbt::ReportingKind::Direct => "direct",
    }
}

struct TraceState {
    output: Mutex<BufWriter<std::fs::File>>,
    failed: AtomicBool,
    started: Instant,
}

impl TraceState {
    fn write(&self, mut record: Map<String, Value>) {
        let elapsed_ns = u64::try_from(self.started.elapsed().as_nanos()).unwrap_or(u64::MAX);
        record.insert(
            "trace_elapsed_ns".to_owned(),
            Value::Number(elapsed_ns.into()),
        );
        let Ok(mut bytes) = serde_json::to_vec(&record) else {
            self.failed.store(true, Ordering::Release);
            return;
        };
        bytes.push(b'\n');
        let Ok(mut output) = self.output.lock() else {
            self.failed.store(true, Ordering::Release);
            return;
        };
        if output
            .write_all(&bytes)
            .and_then(|()| output.flush())
            .is_err()
        {
            self.failed.store(true, Ordering::Release);
        }
    }
}

struct NdjsonSubscriber {
    state: Arc<TraceState>,
}

impl Subscriber for NdjsonSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.is_event() && metadata.target() == TARGET
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::INFO)
    }

    fn new_span(&self, _span: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut visitor = JsonVisitor::default();
        event.record(&mut visitor);
        if visitor.invalid || !normalize_and_validate(&mut visitor.fields) {
            self.state.failed.store(true, Ordering::Release);
            return;
        }
        self.state.write(visitor.fields);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

#[derive(Default)]
struct JsonVisitor {
    fields: Map<String, Value>,
    invalid: bool,
}

impl JsonVisitor {
    fn insert(&mut self, field: &Field, value: Value) {
        if self.fields.insert(field.name().to_owned(), value).is_some() {
            self.invalid = true;
        }
    }
}

impl Visit for JsonVisitor {
    fn record_f64(&mut self, field: &Field, value: f64) {
        match Number::from_f64(value) {
            Some(value) => self.insert(field, Value::Number(value)),
            None => self.invalid = true,
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, Value::Number(value.into()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, Value::Number(value.into()));
    }

    fn record_i128(&mut self, field: &Field, value: i128) {
        match i64::try_from(value) {
            Ok(value) => self.record_i64(field, value),
            Err(_) => self.invalid = true,
        }
    }

    fn record_u128(&mut self, field: &Field, value: u128) {
        match u64::try_from(value) {
            Ok(value) => self.record_u64(field, value),
            Err(_) => self.invalid = true,
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, Value::Bool(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, Value::String(value.to_owned()));
    }

    fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {
        self.invalid = true;
    }
}

fn normalize_and_validate(record: &mut Map<String, Value>) -> bool {
    if record.get("schema").and_then(Value::as_str) != Some(SCHEMA)
        || record.get("schema_version").and_then(Value::as_u64) != Some(SCHEMA_VERSION)
    {
        return false;
    }
    let Some(event) = record.get("event").and_then(Value::as_str) else {
        return false;
    };
    match event {
        "environment" => exact_fields(
            record,
            &[
                "schema",
                "schema_version",
                "event",
                "controller_kind",
                "reporting_kind",
                "package_version",
                "target_os",
                "target_arch",
            ],
        ),
        "session_started" => exact_fields(record, &runtime_fields(&[])),
        "lifecycle_changed" => exact_fields(record, &runtime_fields(&["lifecycle"])),
        "subcommand_observed" => exact_fields(record, &runtime_fields(&["subcommand_id"])),
        "report_tx_accepted" => {
            record
                .entry("report_mode".to_owned())
                .or_insert(Value::Null);
            exact_fields(
                record,
                &runtime_fields(&["report_mode", "imu_mode", "input_reports_accepted"]),
            )
        }
        "reply_tx_accepted" => {
            record
                .entry("report_mode".to_owned())
                .or_insert(Value::Null);
            exact_fields(
                record,
                &runtime_fields(&["report_mode", "imu_mode", "replies_accepted"]),
            )
        }
        "session_ended" => {
            record
                .entry("disconnect_reason".to_owned())
                .or_insert(Value::Null);
            exact_fields(record, &runtime_fields(&["lifecycle", "disconnect_reason"]))
        }
        "worker_failed" => exact_fields(record, &runtime_fields(&["failure_category"])),
        "unsupported_button" => exact_fields(record, &runtime_fields(&["button_kind"])),
        _ => false,
    }
}

fn runtime_fields(extra: &[&'static str]) -> Vec<&'static str> {
    let mut fields = vec![
        "schema",
        "schema_version",
        "event",
        "controller_kind",
        "reporting_kind",
        "session_id",
    ];
    fields.extend_from_slice(extra);
    fields
}

fn exact_fields(record: &Map<String, Value>, expected: &[&str]) -> bool {
    record.len() == expected.len() && expected.iter().all(|field| record.contains_key(*field))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use serde_json::Value;

    use super::{SCHEMA, TARGET, TraceSession, emit_environment};

    static NEXT_TRACE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn subscriber_writes_only_valid_diagnostics_as_independent_json_lines() {
        let path = trace_path();
        let (session, subscriber) = TraceSession::create(&path).expect("create trace");

        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "swbt::transport", message = "T08_SECRET_NOISE");
            emit_environment::<swbt::model::Pro, swbt::reporting::Periodic>();
            let report_mode: Option<u8> = None;
            tracing::event!(
                target: TARGET,
                tracing::Level::INFO,
                schema = SCHEMA,
                schema_version = 1_u64,
                event = "report_tx_accepted",
                controller_kind = "pro",
                reporting_kind = "periodic",
                session_id = 7_u64,
                report_mode,
                imu_mode = 2_u64,
                input_reports_accepted = 3_u64,
            );
        });
        session.finish().expect("finish trace");

        let trace = fs::read_to_string(&path).expect("read trace");
        let records = trace
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("valid JSON line"))
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0]["event"], "environment");
        assert_eq!(records[1]["event"], "report_tx_accepted");
        assert!(records[1]["report_mode"].is_null());
        assert!(
            records
                .iter()
                .all(|record| record["trace_elapsed_ns"].is_u64())
        );
        assert!(
            records[1]["trace_elapsed_ns"].as_u64().unwrap()
                >= records[0]["trace_elapsed_ns"].as_u64().unwrap()
        );
        assert!(!trace.contains("T08_SECRET_NOISE"));

        fs::remove_file(path).expect("remove trace");
    }

    #[test]
    fn subscriber_rejects_unknown_diagnostics_fields_without_writing_them() {
        let path = trace_path();
        let (session, subscriber) = TraceSession::create(&path).expect("create trace");

        tracing::subscriber::with_default(subscriber, || {
            tracing::event!(
                target: TARGET,
                tracing::Level::INFO,
                schema = SCHEMA,
                schema_version = 1_u64,
                event = "environment",
                controller_kind = "pro",
                reporting_kind = "periodic",
                package_version = "0.1.0",
                target_os = "windows",
                target_arch = "x86_64",
                profile_path = "T08_SECRET_PROFILE",
            );
        });

        assert_eq!(session.finish(), Err(swbt::ErrorKind::Trace));
        let trace = fs::read_to_string(&path).expect("read rejected trace");
        assert!(trace.is_empty());
        assert!(!trace.contains("T08_SECRET_PROFILE"));

        fs::remove_file(path).expect("remove trace");
    }

    fn trace_path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "swbt-probe-trace-{}-{}.jsonl",
            std::process::id(),
            NEXT_TRACE.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
