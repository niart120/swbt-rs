use std::collections::BTreeSet;

use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/python-v0.6.0/runtime/runtime-semantics.json");
const SOURCE_COMMIT: &str = "84d2723b127f70fc78e12f4496f5c40af0ccfb0a";
const SOURCE_TREE: &str = "ee0654eac1d443e7814eb9816bd9d75328ff8485";
const EXPECTED_SOURCE_PATHS: &[&str] = &[
    "src/swbt/diagnostics.py",
    "src/swbt/errors.py",
    "src/swbt/gamepad/_config.py",
    "src/swbt/gamepad/connection.py",
    "src/swbt/gamepad/output.py",
    "src/swbt/gamepad/protocol_handshake.py",
    "src/swbt/gamepad/runtime.py",
    "src/swbt/imu.py",
    "src/swbt/input.py",
    "src/swbt/protocol/buttons.py",
    "src/swbt/protocol/imu_report.py",
    "src/swbt/protocol/input_report.py",
    "src/swbt/protocol/output_report.py",
    "src/swbt/protocol/profiles/base.py",
    "src/swbt/protocol/profiles/pro_controller.py",
    "src/swbt/protocol/session.py",
    "src/swbt/protocol/spi.py",
    "src/swbt/protocol/subcommand.py",
    "src/swbt/report_loop.py",
    "src/swbt/state_store.py",
    "src/swbt/transport/base.py",
    "src/swbt/transport/fake.py",
];
const EXPECTED_CASES: &[(&str, &str, &str)] = &[
    ("direct.accepted_commit", "parity", "direct"),
    ("direct.rejected_no_commit", "parity", "direct"),
    (
        "handshake.retry_after_send_latency",
        "baseline_observation",
        "model-independent",
    ),
    (
        "imu.rejected_quaternion_input",
        "baseline_observation",
        "direct",
    ),
    ("periodic.disconnect_neutralize", "parity", "periodic"),
    (
        "periodic.pre_connection_update",
        "baseline_observation",
        "periodic",
    ),
    ("sender.imu_mode_inflight_order", "parity", "periodic"),
    (
        "sender.pre_ready_neutral_prefix",
        "parity",
        "model-independent",
    ),
    ("sender.ready_current_prefix", "parity", "model-independent"),
    ("sender.shared_timer", "parity", "model-independent"),
    (
        "subcommand.imu_mode_rejected_reply",
        "parity",
        "model-independent",
    ),
    ("tap.direct_release_rejected", "parity", "direct"),
    ("tap.periodic_release_rejected", "parity", "periodic"),
];

fn fixture_document() -> Value {
    serde_json::from_str(FIXTURE).expect("generated runtime fixture must be valid JSON")
}

fn case<'a>(fixture: &'a Value, id: &str) -> &'a Value {
    fixture["cases"]
        .as_array()
        .expect("cases must be an array")
        .iter()
        .find(|case| case["id"] == id)
        .unwrap_or_else(|| panic!("missing fixture case: {id}"))
}

fn checkpoint<'a>(case: &'a Value, id: &str) -> &'a Value {
    case["expected"]["checkpoints"]
        .as_array()
        .expect("checkpoints must be an array")
        .iter()
        .find(|checkpoint| checkpoint["id"] == id)
        .unwrap_or_else(|| panic!("missing fixture checkpoint: {id}"))
}

#[test]
fn runtime_fixture_has_pinned_reproducible_provenance() {
    let fixture = fixture_document();

    assert_eq!(fixture["format"], "swbt.runtime-semantics");
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["source_repository"],
        "https://github.com/niart120/swbt-python"
    );
    assert_eq!(fixture["source_commit"], SOURCE_COMMIT);
    assert_eq!(fixture["source_tree"], SOURCE_TREE);
    assert_eq!(fixture["source_version"], "0.6.0");
    assert_eq!(fixture["python_version"], "3.13");
    assert_eq!(
        fixture["generator"],
        "tools/generate_python_runtime_fixtures.py"
    );

    let source_paths = fixture["source_paths"]
        .as_array()
        .expect("source_paths must be an array");
    assert!(!source_paths.is_empty());
    assert!(source_paths.iter().all(|path| {
        path.as_str()
            .is_some_and(|path| path.starts_with("src/swbt/") || path.starts_with("tests/"))
    }));
    let source_paths = source_paths
        .iter()
        .map(|path| path.as_str().expect("source path must be a string"))
        .collect::<Vec<_>>();
    assert!(source_paths.windows(2).all(|paths| paths[0] < paths[1]));
    assert_eq!(source_paths, EXPECTED_SOURCE_PATHS);
}

#[test]
fn runtime_fixture_case_set_and_causal_shape_are_exact() {
    let fixture = fixture_document();
    let cases = fixture["cases"].as_array().expect("cases must be an array");
    let expected_cases = EXPECTED_CASES.iter().copied().collect::<BTreeSet<_>>();
    let actual_cases = cases
        .iter()
        .map(|case| {
            (
                case["id"].as_str().expect("case id must be a string"),
                case["classification"]
                    .as_str()
                    .expect("classification must be a string"),
                case["reporting"]
                    .as_str()
                    .expect("reporting must be a string"),
            )
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(cases.len(), EXPECTED_CASES.len());
    assert_eq!(actual_cases, expected_cases);

    let mut classifications = BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().expect("case id must be a string");
        let classification = case["classification"]
            .as_str()
            .expect("classification must be a string");
        classifications.insert(classification);
        assert!(
            matches!(classification, "parity" | "baseline_observation"),
            "{id}: unsupported classification"
        );
        assert!(case["model"].is_string(), "{id}: model");
        assert!(case["reporting"].is_string(), "{id}: reporting");

        let steps = case["steps"].as_array().expect("steps must be an array");
        assert!(!steps.is_empty(), "{id}: steps");
        let step_ids = steps
            .iter()
            .map(|step| {
                assert!(step.is_object(), "{id}: every step must be an object");
                let action = step["action"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{id}: step action"));
                match action {
                    "advance_clock" | "wait_after_completion" => {
                        assert!(step["duration_ns"].is_number(), "{id}: {action} duration")
                    }
                    "commit_input" | "commit_periodic_input" | "press" => {
                        assert!(step["buttons"].is_array(), "{id}: {action} buttons")
                    }
                    "dispatch_subcommand" | "queue_subcommand_reply" | "send_reply" => assert!(
                        step["subcommand_id"].is_number(),
                        "{id}: {action} subcommand"
                    ),
                    "open_runtime" => {
                        assert!(step["connection_state"].is_string(), "{id}: open state");
                    }
                    "send_automatic_input" => {
                        assert!(step["acceptance"].is_string(), "{id}: automatic acceptance");
                    }
                    "send_bootstrap" => {
                        assert!(step["latency_ns"].is_number(), "{id}: bootstrap latency");
                    }
                    "send_direct" | "send_input" | "tap_press" | "tap_release" => {
                        assert!(step["buttons"].is_array(), "{id}: {action} buttons");
                        assert!(step["acceptance"].is_string(), "{id}: {action} acceptance");
                    }
                    "set_imu_mode" => {
                        assert!(step["imu_mode"].is_number(), "{id}: IMU mode");
                    }
                    "set_protocol_session" => {
                        assert!(step["report_mode"].is_number(), "{id}: report mode");
                        assert!(step["player_lights"].is_number(), "{id}: lights");
                        assert!(
                            step["protocol_ready"].is_boolean(),
                            "{id}: protocol readiness"
                        );
                    }
                    "start_handshake" => {
                        assert!(
                            step["protocol_ready"].is_boolean(),
                            "{id}: initial readiness"
                        );
                        assert!(
                            step["bootstrap_retry_ns"].is_number(),
                            "{id}: bootstrap retry"
                        );
                    }
                    "transport_disconnect" => {
                        assert!(step["reason"].is_number(), "{id}: disconnect reason");
                    }
                    "block_input_snapshot"
                    | "complete_protocol_handshake"
                    | "release_input_snapshot"
                    | "send_current_input" => {}
                    _ => panic!("{id}: unsupported step action {action}"),
                }
                step["id"].as_str().expect("step id must be a string")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(step_ids.len(), steps.len(), "{id}: duplicate step id");

        let checkpoints = case["expected"]["checkpoints"]
            .as_array()
            .expect("expected checkpoints must be an array");
        assert!(!checkpoints.is_empty(), "{id}: checkpoints");
        let checkpoint_ids = checkpoints
            .iter()
            .map(|checkpoint| {
                assert!(
                    checkpoint.is_object(),
                    "{id}: every checkpoint must be an object"
                );
                let after_step = checkpoint["after_step"]
                    .as_str()
                    .expect("checkpoint after_step must be a string");
                assert!(
                    step_ids.contains(after_step),
                    "{id}: checkpoint references unknown step {after_step}"
                );
                checkpoint["id"]
                    .as_str()
                    .expect("checkpoint id must be a string")
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            checkpoint_ids.len(),
            checkpoints.len(),
            "{id}: duplicate checkpoint id"
        );
    }

    assert_eq!(
        classifications,
        BTreeSet::from(["baseline_observation", "parity"])
    );
}

#[test]
fn runtime_fixture_pins_python_causal_observations() {
    let fixture = fixture_document();

    let shared_timer = checkpoint(case(&fixture, "sender.shared_timer"), "shared_sequence");
    assert_eq!(shared_timer["next_timer"], 3);
    assert_eq!(
        shared_timer["report_attempts"]
            .as_array()
            .expect("shared timer attempts")
            .iter()
            .map(|attempt| (attempt["report_id"].as_u64(), attempt["timer"].as_u64()))
            .collect::<Vec<_>>(),
        [
            (Some(0x30), Some(0)),
            (Some(0x21), Some(1)),
            (Some(0x30), Some(2)),
        ]
    );

    let pre_ready = checkpoint(
        case(&fixture, "sender.pre_ready_neutral_prefix"),
        "neutral_prefix",
    );
    assert_eq!(pre_ready["report_attempts"][0]["button_hex"], "000000");
    assert_eq!(pre_ready["protocol_session"]["protocol_ready"], false);
    let ready = checkpoint(
        case(&fixture, "sender.ready_current_prefix"),
        "current_prefix",
    );
    assert_eq!(ready["report_attempts"][0]["button_hex"], "080000");
    assert_eq!(ready["protocol_session"]["protocol_ready"], true);

    let direct_accepted = checkpoint(case(&fixture, "direct.accepted_commit"), "committed");
    assert_eq!(direct_accepted["next_timer"], 1);
    assert_eq!(direct_accepted["committed_input"]["buttons"], json!(["A"]));
    let direct_rejected = case(&fixture, "direct.rejected_no_commit");
    let rejected = checkpoint(direct_rejected, "rejected");
    assert_eq!(rejected["next_timer"], 1);
    assert_eq!(rejected["committed_input"]["buttons"], json!(["A"]));
    assert_eq!(rejected["report_attempts"][1]["result"], "rejected");
    assert_eq!(rejected["report_attempts"][1]["timer"], 1);
    let retry = checkpoint(direct_rejected, "retry_committed");
    assert_eq!(retry["next_timer"], 2);
    assert_eq!(retry["committed_input"]["buttons"], json!(["X"]));
    assert_eq!(retry["report_attempts"][2]["result"], "accepted");
    assert_eq!(retry["report_attempts"][2]["timer"], 1);

    let imu_order = checkpoint(
        case(&fixture, "sender.imu_mode_inflight_order"),
        "accepted_order",
    );
    assert_eq!(imu_order["protocol_session"]["imu_mode"], 2);
    assert_eq!(
        imu_order["report_attempts"]
            .as_array()
            .expect("IMU order attempts")
            .iter()
            .map(|attempt| (attempt["report_id"].as_u64(), attempt["timer"].as_u64()))
            .collect::<Vec<_>>(),
        [
            (Some(0x30), Some(0)),
            (Some(0x21), Some(1)),
            (Some(0x30), Some(2)),
        ]
    );
    assert_eq!(
        imu_order["report_attempts"][0]["imu_hex"],
        "000000000000000000000000000000000000000000000000000000000000000000000000"
    );
    assert_ne!(
        imu_order["report_attempts"][2]["imu_hex"],
        imu_order["report_attempts"][0]["imu_hex"]
    );

    let rejected_reply = case(&fixture, "subcommand.imu_mode_rejected_reply");
    let failed_reply = checkpoint(rejected_reply, "reply_rejected");
    assert_eq!(failed_reply["next_timer"], 0);
    assert_eq!(failed_reply["automatic_holdoff_until_ns"], 0);
    assert_eq!(failed_reply["protocol_session"]["imu_mode"], 0);
    assert_eq!(
        failed_reply["protocol_session"]["observed_subcommands"],
        json!([0x40])
    );
    assert_eq!(failed_reply["report_attempts"][0]["result"], "rejected");
    let held_off = checkpoint(rejected_reply, "accepted_reply_holds_off_periodic");
    assert_eq!(held_off["automatic_input_sent"], false);
    assert_eq!(held_off["automatic_holdoff_until_ns"], 300_000_000);
    assert_eq!(held_off["next_timer"], 1);
    let after_holdoff = checkpoint(rejected_reply, "retry_then_input");
    assert_eq!(after_holdoff["automatic_input_sent"], true);
    assert_eq!(after_holdoff["report_attempts"][2]["report_id"], 0x30);
    assert_eq!(after_holdoff["report_attempts"][2]["timer"], 1);
    assert_eq!(after_holdoff["next_timer"], 2);

    let preconnection = checkpoint(
        case(&fixture, "periodic.pre_connection_update"),
        "python_carries_state",
    );
    assert_eq!(
        preconnection["preconnection_input"]["buttons"],
        json!(["A"])
    );
    assert!(
        preconnection["handshake_report_attempts"]
            .as_array()
            .expect("handshake attempts")
            .iter()
            .all(|attempt| attempt["button_hex"] == "000000")
    );
    assert_eq!(
        preconnection["handshake_report_attempts"]
            .as_array()
            .expect("handshake attempts")
            .iter()
            .map(|attempt| (attempt["report_id"].as_u64(), attempt["timer"].as_u64()))
            .collect::<Vec<_>>(),
        [
            (Some(0x30), Some(0)),
            (Some(0x21), Some(1)),
            (Some(0x21), Some(2)),
        ]
    );
    assert_eq!(
        preconnection["first_current_report_attempts"][0]["button_hex"],
        "080000"
    );
    assert_eq!(
        preconnection["first_current_report_attempts"][0]["timer"],
        3
    );
    let disconnected = checkpoint(
        case(&fixture, "periodic.disconnect_neutralize"),
        "neutralized",
    );
    assert_eq!(disconnected["before_disconnect"]["buttons"], json!(["A"]));
    assert_eq!(disconnected["committed_input"]["buttons"], json!([]));
    assert_eq!(disconnected["connection_state"], "closed");

    let periodic_tap = checkpoint(
        case(&fixture, "tap.periodic_release_rejected"),
        "released_state_retained",
    );
    assert_eq!(periodic_tap["committed_input"]["buttons"], json!(["ZL"]));
    assert_eq!(periodic_tap["transport_rejection_propagated"], true);
    assert_eq!(periodic_tap["report_attempts"][0]["button_hex"], "080080");
    assert_eq!(periodic_tap["report_attempts"][0]["result"], "accepted");
    assert_eq!(periodic_tap["report_attempts"][1]["button_hex"], "000080");
    assert_eq!(periodic_tap["report_attempts"][1]["result"], "rejected");
    let direct_tap = checkpoint(
        case(&fixture, "tap.direct_release_rejected"),
        "pressed_state_retained",
    );
    assert_eq!(direct_tap["committed_input"]["buttons"], json!(["A", "ZL"]));
    assert_eq!(direct_tap["transport_rejection_propagated"], true);
    assert_eq!(direct_tap["report_attempts"][0]["button_hex"], "080080");
    assert_eq!(direct_tap["report_attempts"][0]["result"], "accepted");
    assert_eq!(direct_tap["report_attempts"][1]["button_hex"], "000080");
    assert_eq!(direct_tap["report_attempts"][1]["result"], "rejected");

    let rejected_imu = case(&fixture, "imu.rejected_quaternion_input");
    let advanced_before_acceptance = checkpoint(rejected_imu, "python_advances_before_acceptance");
    assert_eq!(advanced_before_acceptance["next_timer"], 1);
    assert_eq!(
        advanced_before_acceptance["committed_input"]["buttons"],
        json!(["A"])
    );
    assert_eq!(
        advanced_before_acceptance["protocol_session"]["imu_previous_report_ns"],
        2_000_000_000_u64
    );
    let same_time_retry = checkpoint(rejected_imu, "same_time_retry");
    assert_eq!(same_time_retry["retry_wire_matches_rejected"], true);
    assert_eq!(same_time_retry["report_attempts"][1]["timer"], 1);
    assert_eq!(same_time_retry["report_attempts"][2]["timer"], 1);
    assert_eq!(same_time_retry["committed_input"]["buttons"], json!(["X"]));

    let relative_retry = checkpoint(
        case(&fixture, "handshake.retry_after_send_latency"),
        "python_relative_retry",
    );
    assert_eq!(
        relative_retry["requested_waits_ns"],
        json!([1_000_000_000_u64, 1_000_000_000_u64])
    );
    assert_eq!(
        relative_retry["second_start_minus_first_start_ns"],
        1_250_000_000_u64
    );
    assert_eq!(
        relative_retry["second_start_minus_first_completion_ns"],
        1_000_000_000_u64
    );
}
