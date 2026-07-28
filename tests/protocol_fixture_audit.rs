use std::collections::BTreeSet;

use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/python-v0.6.0/protocol/protocol-fixtures.json");
const SOURCE_COMMIT: &str = "84d2723b127f70fc78e12f4496f5c40af0ccfb0a";

fn fixture_document() -> Value {
    serde_json::from_str(FIXTURE).expect("generated protocol fixture must be valid JSON")
}

#[test]
fn protocol_fixture_has_pinned_reproducible_provenance() {
    let fixture = fixture_document();

    assert_eq!(fixture["format"], "swbt.protocol-fixtures");
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["source_repository"],
        "https://github.com/niart120/swbt-python"
    );
    assert_eq!(fixture["source_commit"], SOURCE_COMMIT);
    assert_eq!(fixture["source_version"], "0.6.0");
    assert_eq!(fixture["python_version"], "3.13");
    assert_eq!(fixture["generator"], "tools/generate_python_fixtures.py");

    let source_paths = fixture["source_paths"]
        .as_array()
        .expect("source_paths must be an array");
    assert!(!source_paths.is_empty());
    assert!(source_paths.iter().all(|path| {
        path.as_str()
            .is_some_and(|path| path.starts_with("src/swbt/"))
    }));
}

#[test]
fn every_fixture_case_has_semantic_input_and_expected_result() {
    let fixture = fixture_document();
    let cases = fixture["cases"].as_array().expect("cases must be an array");
    assert!(!cases.is_empty());

    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().expect("case id must be a string");
        assert!(ids.insert(id), "duplicate fixture case: {id}");
        let kind = case["kind"].as_str().expect("case kind must be a string");
        kinds.insert(kind);
        assert!(case["model"].is_string());
        assert!(case["input"].is_object());
        assert!(case["expected"].is_object());
    }

    assert_eq!(
        kinds,
        BTreeSet::from([
            "conversion",
            "input_report",
            "output_report",
            "spi",
            "subcommand",
        ])
    );
}
