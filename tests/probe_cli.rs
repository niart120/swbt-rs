#![cfg(feature = "probe")]

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::Value;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

#[test]
fn profile_inspect_and_verify_emit_safe_versioned_ndjson() {
    let fixture = ProbeFixture::new();
    fs::write(&fixture.profile, profile_json()).expect("write probe profile fixture");

    let inspect = run(["profile", "inspect", fixture.profile_text()]);
    assert_eq!(inspect.status.code(), Some(0));
    assert!(inspect.stderr.is_empty());
    let inspected = one_json_line(&inspect.stdout);
    assert_eq!(inspected["schema"], "swbt.probe");
    assert_eq!(inspected["schema_version"], 1);
    assert_eq!(inspected["event"], "profile_inspected");
    assert_eq!(inspected["profile_schema_version"], 2);
    assert_eq!(inspected["controller_kind"], "joycon_l");
    assert_eq!(inspected["identity_kind"], "adapter_default");
    assert_eq!(inspected["namespace_count"], 1);
    assert_eq!(inspected["bond_count"], 1);
    assert_safe_output(&inspect, &fixture);

    let verify = run(["profile", "verify", fixture.profile_text()]);
    assert_eq!(verify.status.code(), Some(0));
    assert!(verify.stderr.is_empty());
    let verified = one_json_line(&verify.stdout);
    assert_eq!(verified["schema"], "swbt.probe");
    assert_eq!(verified["schema_version"], 1);
    assert_eq!(verified["event"], "profile_verified");
    assert_eq!(verified["controller_kind"], "joycon_l");
    assert_eq!(verified["valid"], true);
    assert_safe_output(&verify, &fixture);
}

#[test]
fn profile_errors_are_classified_without_path_or_source_disclosure() {
    let fixture = ProbeFixture::new();
    fs::write(&fixture.profile, b"{").expect("write malformed probe profile");

    let invalid = run(["profile", "verify", fixture.profile_text()]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    let error = one_json_line(&invalid.stderr);
    assert_eq!(error["schema"], "swbt.probe");
    assert_eq!(error["schema_version"], 1);
    assert_eq!(error["event"], "error");
    assert_eq!(error["error_kind"], "invalid_profile");
    assert_safe_output(&invalid, &fixture);

    fs::remove_file(&fixture.profile).expect("remove malformed probe profile");
    let missing = run(["profile", "inspect", fixture.profile_text()]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        one_json_line(&missing.stderr)["error_kind"],
        "profile_not_found"
    );
    assert_safe_output(&missing, &fixture);
}

#[test]
fn profile_usage_rejects_missing_extra_and_unknown_arguments_with_exit_two() {
    for arguments in [
        vec!["profile"],
        vec!["profile", "inspect"],
        vec!["profile", "verify", "one", "two"],
        vec!["profile", "unknown", "one"],
        vec!["unknown"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "arguments: {arguments:?}");
        assert!(output.stdout.is_empty());
        let error = one_json_line(&output.stderr);
        assert_eq!(error["event"], "error");
        assert_eq!(error["error_kind"], "usage");
        assert_eq!(error["usage"], "swbt-probe help");
    }

    for argument in ["help", "--help", "-h"] {
        let output = run([argument]);
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let help = String::from_utf8(output.stdout).unwrap();
        assert!(help.contains("swbt-probe profile inspect <path>"));
        assert!(help.contains("swbt-probe profile verify <path>"));
    }
}

#[test]
fn adapter_commands_use_strict_usage_and_redact_operation_errors() {
    let secret_selector = "usb:T06_SECRET_SELECTOR";
    let invalid = run(["open", "--adapter", secret_selector]);
    assert_eq!(invalid.status.code(), Some(1));
    assert!(invalid.stdout.is_empty());
    let error = one_json_line(&invalid.stderr);
    assert_eq!(error["event"], "error");
    assert_eq!(error["error_kind"], "transport_open");
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains(secret_selector));

    for arguments in [
        vec!["adapters", "extra"],
        vec!["open"],
        vec!["open", "--adapter"],
        vec!["open", "--wrong", "usb:0"],
        vec!["open", "--adapter", "usb:0", "--adapter", "usb:1"],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "arguments: {arguments:?}");
        assert!(output.stdout.is_empty());
        assert_eq!(one_json_line(&output.stderr)["error_kind"], "usage");
    }
}

#[test]
fn connection_commands_dispatch_known_models_without_fallback() {
    let fixture = ProbeFixture::new();
    let trace_text = fixture.trace_text();
    fs::write(&fixture.profile, profile_json()).expect("write existing profile");

    let pair = run([
        "pair",
        "--controller",
        "pro",
        "--profile",
        fixture.profile_text(),
        "--trace",
        trace_text,
    ]);
    assert_eq!(pair.status.code(), Some(1));
    assert_eq!(
        one_json_line(&pair.stderr)["error_kind"],
        "profile_already_exists"
    );
    assert_safe_output(&pair, &fixture);
    assert!(!String::from_utf8_lossy(&pair.stderr).contains(trace_text));

    fs::remove_file(&fixture.trace).expect("remove pair preflight trace");
    fs::remove_file(&fixture.profile).expect("remove profile before reconnect preflight");
    let reconnect = run([
        "reconnect",
        "--controller",
        "joycon-r",
        "--profile",
        fixture.profile_text(),
        "--trace",
        trace_text,
        "--reporting",
        "direct",
    ]);
    assert_eq!(reconnect.status.code(), Some(1));
    assert_eq!(
        one_json_line(&reconnect.stderr)["error_kind"],
        "profile_not_found"
    );
    assert_safe_output(&reconnect, &fixture);
    assert!(!String::from_utf8_lossy(&reconnect.stderr).contains(trace_text));

    for arguments in [
        vec![
            "pair",
            "--controller",
            "unknown",
            "--profile",
            fixture.profile_text(),
            "--trace",
            trace_text,
        ],
        vec![
            "reconnect",
            "--controller",
            "pro",
            "--profile",
            fixture.profile_text(),
            "--trace",
            trace_text,
            "--reporting",
            "unknown",
        ],
        vec![
            "pair",
            "--controller",
            "pro",
            "--profile",
            fixture.profile_text(),
            "--trace",
            trace_text,
            "--reporting",
            "direct",
        ],
    ] {
        let output = run(&arguments);
        assert_eq!(output.status.code(), Some(2), "arguments: {arguments:?}");
        assert_eq!(one_json_line(&output.stderr)["error_kind"], "usage");
    }
}

#[test]
fn trace_is_create_new_valid_ndjson_and_redacted_on_operation_failure() {
    let fixture = ProbeFixture::new();
    fs::write(&fixture.profile, profile_json()).expect("write existing profile");
    fs::write(&fixture.trace, b"T08_EXISTING_TRACE\n").expect("write existing trace");

    let occupied = run([
        "pair",
        "--controller",
        "pro",
        "--profile",
        fixture.profile_text(),
        "--trace",
        fixture.trace_text(),
    ]);
    assert_eq!(occupied.status.code(), Some(1));
    assert_eq!(one_json_line(&occupied.stderr)["error_kind"], "trace");
    assert_eq!(
        fs::read(&fixture.trace).expect("read occupied trace"),
        b"T08_EXISTING_TRACE\n"
    );
    assert_safe_output(&occupied, &fixture);

    fs::remove_file(&fixture.trace).expect("remove occupied trace");
    let preflight_failure = run([
        "pair",
        "--controller",
        "pro",
        "--profile",
        fixture.profile_text(),
        "--trace",
        fixture.trace_text(),
    ]);
    assert_eq!(preflight_failure.status.code(), Some(1));
    assert_eq!(
        one_json_line(&preflight_failure.stderr)["error_kind"],
        "profile_already_exists"
    );
    let trace = fs::read_to_string(&fixture.trace).expect("read new trace");
    let lines = trace.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "trace: {trace:?}");
    let environment: Value = serde_json::from_str(lines[0]).expect("valid NDJSON line");
    assert_eq!(environment["schema"], "swbt.diagnostics");
    assert_eq!(environment["schema_version"], 1);
    assert_eq!(environment["event"], "environment");
    assert_eq!(environment["controller_kind"], "pro");
    assert_eq!(environment["reporting_kind"], "periodic");
    for forbidden in [
        "T05_SECRET_PATH",
        fixture.profile_text(),
        fixture.trace_text(),
        "02:12:34:56:78:9A",
        "AA:BB:CC:DD:EE:FF",
        "7055EC0E7A7055EC",
    ] {
        assert!(!trace.contains(forbidden), "trace disclosed {forbidden}");
    }
}

fn run<I, S>(arguments: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    Command::new(env!("CARGO_BIN_EXE_swbt-probe"))
        .args(arguments)
        .output()
        .expect("run swbt-probe")
}

fn one_json_line(bytes: &[u8]) -> Value {
    let text = std::str::from_utf8(bytes).expect("probe output must be UTF-8");
    assert_eq!(text.lines().count(), 1, "probe output: {text:?}");
    serde_json::from_str(text).expect("probe output must be one JSON object")
}

fn assert_safe_output(output: &Output, fixture: &ProbeFixture) {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!combined.contains("T05_SECRET_PATH"));
    assert!(!combined.contains(fixture.profile_text()));
    assert!(!combined.contains("02:12:34:56:78:9A"));
    assert!(!combined.contains("AA:BB:CC:DD:EE:FF"));
    assert!(!combined.contains("7055EC0E7A7055EC"));
}

struct ProbeFixture {
    directory: PathBuf,
    profile: PathBuf,
    trace: PathBuf,
}

impl ProbeFixture {
    fn new() -> Self {
        let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir().join(format!(
            "swbt-probe-T05_SECRET_PATH-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).expect("create probe test directory");
        let profile = directory.join("profile.json");
        let trace = directory.join("trace.jsonl");
        Self {
            directory,
            profile,
            trace,
        }
    }

    fn profile_text(&self) -> &str {
        self.profile.to_str().expect("test path must be UTF-8")
    }

    fn trace_text(&self) -> &str {
        self.trace.to_str().expect("test path must be UTF-8")
    }
}

impl Drop for ProbeFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.profile);
        let _ = fs::remove_file(&self.trace);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn profile_json() -> &'static str {
    r#"{
  "format": "swbt.profile",
  "schema_version": 2,
  "controller_kind": "joycon_l",
  "identity": { "kind": "adapter-default" },
  "key_store": {
    "namespaces": {
      "02:12:34:56:78:9A": {
        "AA:BB:CC:DD:EE:FF": {
          "link_key": { "value": "7055EC0E7A7055EC" }
        }
      }
    }
  }
}"#
}
