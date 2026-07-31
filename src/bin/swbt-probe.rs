use std::{ffi::OsString, io, path::PathBuf, process::ExitCode};

use serde_json::{Value, json};
use swbt::{ErrorKind, ProfileIdentityKind, ProfileSummary, inspect_profile};

const PROBE_SCHEMA: &str = "swbt.probe";
const PROBE_SCHEMA_VERSION: u64 = 1;
const EXIT_OPERATION_ERROR: u8 = 1;
const EXIT_USAGE: u8 = 2;
const HELP: &str = "\
Usage:
  swbt-probe profile inspect <path>
  swbt-probe profile verify <path>
  swbt-probe help
";

fn main() -> ExitCode {
    match parse(std::env::args_os().skip(1).collect()) {
        Ok(Command::Help) => match write_text(io::stdout().lock(), HELP) {
            Ok(()) => ExitCode::SUCCESS,
            Err(()) => operation_write_failure(),
        },
        Ok(command) => match execute(command) {
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
    ProfileInspect(PathBuf),
    ProfileVerify(PathBuf),
}

fn parse(arguments: Vec<OsString>) -> Result<Command, ()> {
    match arguments.as_slice() {
        [command] if matches!(command.to_str(), Some("help" | "--help" | "-h")) => {
            Ok(Command::Help)
        }
        [profile, action, path] if profile == "profile" && action == "inspect" => {
            Ok(Command::ProfileInspect(PathBuf::from(path)))
        }
        [profile, action, path] if profile == "profile" && action == "verify" => {
            Ok(Command::ProfileVerify(PathBuf::from(path)))
        }
        _ => Err(()),
    }
}

fn execute(command: Command) -> Result<Value, ErrorKind> {
    match command {
        Command::Help => unreachable!("help bypasses command execution"),
        Command::ProfileInspect(path) => inspect_profile(path)
            .map(profile_inspected_record)
            .map_err(|error| error.kind()),
        Command::ProfileVerify(path) => inspect_profile(path)
            .map(profile_verified_record)
            .map_err(|error| error.kind()),
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

const fn error_kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::ProfileNotFound => "profile_not_found",
        ErrorKind::InvalidProfile => "invalid_profile",
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
