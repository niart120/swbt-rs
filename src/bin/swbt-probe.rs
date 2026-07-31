use std::{ffi::OsString, io, path::PathBuf, process::ExitCode};

use serde_json::{Value, json};
use swbt::{
    DirectProController, ErrorKind, ProfileIdentityKind, ProfileSummary, inspect_profile,
    list_adapters,
};

const PROBE_SCHEMA: &str = "swbt.probe";
const PROBE_SCHEMA_VERSION: u64 = 1;
const EXIT_OPERATION_ERROR: u8 = 1;
const EXIT_USAGE: u8 = 2;
const HELP: &str = "\
Usage:
  swbt-probe adapters
  swbt-probe open --adapter <selector>
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
    ProfileInspect(PathBuf),
    ProfileVerify(PathBuf),
}

fn parse(arguments: Vec<OsString>) -> Result<Command, ()> {
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

fn execute(command: Command, backend: &mut impl ProbeBackend) -> Result<Value, ErrorKind> {
    match command {
        Command::Help => unreachable!("help bypasses command execution"),
        Command::Adapters => backend.list_adapters().map(adapters_listed_record),
        Command::Open(selector) => backend
            .open_adapter(&selector)
            .map(|()| adapter_opened_record()),
        Command::ProfileInspect(path) => inspect_profile(path)
            .map(profile_inspected_record)
            .map_err(|error| error.kind()),
        Command::ProfileVerify(path) => inspect_profile(path)
            .map(profile_verified_record)
            .map_err(|error| error.kind()),
    }
}

struct SafeAdapter {
    vendor_id: u16,
    product_id: u16,
}

trait ProbeBackend {
    fn list_adapters(&mut self) -> Result<Vec<SafeAdapter>, ErrorKind>;
    fn open_adapter(&mut self, selector: &str) -> Result<(), ErrorKind>;
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

const fn error_kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::AdapterDiscovery => "adapter_discovery",
        ErrorKind::TransportOpen => "transport_open",
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
        ErrorKind::Busy => "busy",
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
mod tests {
    use super::{
        Command, ErrorKind, ProbeBackend, ProbeController, SafeAdapter, execute, open_and_close,
    };

    #[test]
    fn fake_adapter_listing_emits_only_safe_descriptor_fields() {
        let mut backend = FakeBackend {
            adapters: vec![SafeAdapter {
                vendor_id: 0x0a12,
                product_id: 0x0001,
            }],
            ..FakeBackend::default()
        };

        let record = execute(Command::Adapters, &mut backend).expect("list fake adapters");
        assert_eq!(record["event"], "adapters_listed");
        assert_eq!(record["adapter_count"], 1);
        assert_eq!(record["adapters"][0]["vendor_id"], 0x0a12);
        assert_eq!(record["adapters"][0]["product_id"], 0x0001);
        let text = record.to_string();
        for forbidden in ["selector", "serial", "bus", "port"] {
            assert!(
                !text.contains(forbidden),
                "record contains {forbidden}: {text}"
            );
        }
    }

    #[test]
    fn fake_adapter_open_does_not_echo_the_selector() {
        let mut backend = FakeBackend::default();
        let selector = "usb:T06_SECRET_SELECTOR";

        let record =
            execute(Command::Open(selector.to_owned()), &mut backend).expect("open fake adapter");

        assert_eq!(backend.opened_selectors, [selector]);
        assert_eq!(record["event"], "adapter_opened");
        assert!(!record.to_string().contains(selector));
    }

    #[test]
    fn adapter_open_success_requires_explicit_close_success() {
        let mut success = FakeController::default();
        assert_eq!(open_and_close(&mut success), Ok(()));
        assert_eq!(success.calls, ["open", "close"]);

        let mut close_failure = FakeController {
            close_result: Err(ErrorKind::WorkerFailed),
            ..FakeController::default()
        };
        assert_eq!(
            open_and_close(&mut close_failure),
            Err(ErrorKind::WorkerFailed)
        );
        assert_eq!(close_failure.calls, ["open", "close"]);
    }

    #[derive(Default)]
    struct FakeBackend {
        adapters: Vec<SafeAdapter>,
        opened_selectors: Vec<String>,
    }

    impl ProbeBackend for FakeBackend {
        fn list_adapters(&mut self) -> Result<Vec<SafeAdapter>, ErrorKind> {
            Ok(std::mem::take(&mut self.adapters))
        }

        fn open_adapter(&mut self, selector: &str) -> Result<(), ErrorKind> {
            self.opened_selectors.push(selector.to_owned());
            Ok(())
        }
    }

    struct FakeController {
        calls: Vec<&'static str>,
        open_result: Result<(), ErrorKind>,
        close_result: Result<(), ErrorKind>,
    }

    impl Default for FakeController {
        fn default() -> Self {
            Self {
                calls: Vec::new(),
                open_result: Ok(()),
                close_result: Ok(()),
            }
        }
    }

    impl ProbeController for FakeController {
        fn open(&mut self) -> Result<(), ErrorKind> {
            self.calls.push("open");
            self.open_result
        }

        fn close(&mut self) -> Result<(), ErrorKind> {
            self.calls.push("close");
            self.close_result
        }
    }
}
