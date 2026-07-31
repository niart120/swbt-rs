use std::{collections::BTreeSet, fmt::Write as _};

use serde_json::{Value, json};

use crate::{
    input::{Button, ImuFrame, InputState, Stick},
    model::{ButtonKind, ControllerModel, JoyConL, JoyConR, Pro},
    protocol::{
        facade::{OutputPreparation, PreparedOutputAction, SwitchHidProtocol},
        imu::{ImuEncodingState, ImuMode, encode_imu_block},
        input_report::encode_0x30,
        output_report::parse_output_report,
        session::ProtocolSession,
        spi::VirtualSpiFlash,
    },
};

const FIXTURE: &str =
    include_str!("../../../tests/fixtures/python-v0.6.0/protocol/protocol-fixtures.json");
const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
const EXPECTED_CASE_IDS: &[&str] = &[
    "conversion.stick.normalized",
    "conversion.imu.physical_scale",
    "input.pro.neutral",
    "input.pro.all_buttons",
    "input.joycon_l.neutral",
    "input.joycon_l.all_buttons",
    "input.joycon_r.neutral",
    "input.joycon_r.all_buttons",
    "input.pro.custom_sticks",
    "input.pro.standard_imu",
    "input.joycon_l.standard_imu",
    "input.joycon_r.standard_imu",
    "input.pro.quaternion_mode_02",
    "input.pro.quaternion_mode_03",
    "input.pro.quaternion_mode_04",
    "input.pro.quaternion_mode_05",
    "input.joycon_l.quaternion_mode_02",
    "input.joycon_l.quaternion_mode_03",
    "input.joycon_l.quaternion_mode_04",
    "input.joycon_l.quaternion_mode_05",
    "input.joycon_r.quaternion_mode_02",
    "input.joycon_r.quaternion_mode_03",
    "input.joycon_r.quaternion_mode_04",
    "input.joycon_r.quaternion_mode_05",
    "output.valid_01",
    "output.valid_10",
    "output.error.empty",
    "output.error.unknown",
    "output.error.truncated_01",
    "output.error.truncated_10",
    "spi.pro.device_type",
    "spi.pro.calibration",
    "spi.pro.colors",
    "spi.pro.erased",
    "spi.joycon_l.device_type",
    "spi.joycon_l.calibration",
    "spi.joycon_l.colors",
    "spi.joycon_l.erased",
    "spi.joycon_r.device_type",
    "spi.joycon_r.calibration",
    "spi.joycon_r.colors",
    "spi.joycon_r.erased",
    "subcommand.pro.device_info",
    "subcommand.joycon_l.device_info",
    "subcommand.joycon_r.device_info",
    "subcommand.pro.report_mode",
    "subcommand.pro.unsupported_report_mode",
    "subcommand.pro.trigger_elapsed",
    "subcommand.joycon_l.trigger_elapsed",
    "subcommand.pro.simple_ack",
    "subcommand.pro.spi_device_type",
    "subcommand.pro.mcu_config",
    "subcommand.pro.player_lights",
    "subcommand.pro.imu_mode",
    "subcommand.pro.vibration",
];

#[test]
fn committed_python_fixture_case_set_is_exactly_consumed() {
    let document = fixture_document();
    let cases = document["cases"]
        .as_array()
        .expect("cases must be an array");
    let actual = cases.iter().map(case_id).collect::<BTreeSet<_>>();
    let expected = EXPECTED_CASE_IDS.iter().copied().collect::<BTreeSet<_>>();

    assert_eq!(cases.len(), EXPECTED_CASE_IDS.len());
    assert_eq!(actual, expected);
}

#[test]
fn conversion_cases_project_the_python_expected_values() {
    let document = fixture_document();
    let cases = cases_of_kind(&document, "conversion").collect::<Vec<_>>();
    assert_eq!(cases.len(), 2);

    for case in cases {
        assert_case_keys(case);
        assert_eq!(case["model"], "model-independent");
        let actual = match case_id(case) {
            "conversion.stick.normalized" => project_stick_conversion(case),
            "conversion.imu.physical_scale" => project_imu_conversion(case),
            id => panic!("unhandled conversion fixture: {id}"),
        };
        assert_expected(case, actual);
    }
}

#[test]
fn input_report_cases_project_the_python_expected_bytes() {
    let document = fixture_document();
    let cases = cases_of_kind(&document, "input_report").collect::<Vec<_>>();
    assert_eq!(cases.len(), 22);

    for case in cases {
        assert_case_keys(case);
        let actual = if case_id(case) == "input.pro.custom_sticks" {
            project_custom_stick_input(case)
        } else {
            match case["model"].as_str().expect("model must be a string") {
                "pro" => project_input_report::<Pro>(case),
                "joycon_l" => project_input_report::<JoyConL>(case),
                "joycon_r" => project_input_report::<JoyConR>(case),
                model => panic!("unsupported input fixture model: {model}"),
            }
        };
        assert_expected(case, actual);
    }
}

#[test]
fn output_report_cases_project_the_python_parse_results() {
    let document = fixture_document();
    let cases = cases_of_kind(&document, "output_report").collect::<Vec<_>>();
    assert_eq!(cases.len(), 6);

    for case in cases {
        assert_case_keys(case);
        assert_eq!(case["model"], "model-independent");
        assert_exact_keys(&case["input"], &["raw_hex"], case_id(case));
        let raw = decode_hex(text(&case["input"]["raw_hex"], "raw_hex"));
        let actual = match parse_output_report(&raw) {
            Ok(report) => {
                let subcommand = report.subcommand();
                json!({
                    "outcome": "parsed",
                    "report_id": report.report_id(),
                    "packet_id": report.packet_id(),
                    "rumble_hex": encode_hex(report.rumble().bytes()),
                    "subcommand_id": subcommand.map(|request| request.id()),
                    "payload_hex": subcommand
                        .map_or_else(String::new, |request| encode_hex(request.payload())),
                })
            }
            Err(_) => json!({
                "outcome": "error",
                "error_type": "ProtocolError",
            }),
        };
        assert_expected(case, actual);
    }
}

#[test]
fn spi_cases_project_the_python_expected_bytes() {
    let document = fixture_document();
    let cases = cases_of_kind(&document, "spi").collect::<Vec<_>>();
    assert_eq!(cases.len(), 12);

    for case in cases {
        assert_case_keys(case);
        let actual = match case["model"].as_str().expect("model must be a string") {
            "pro" => project_spi::<Pro>(case),
            "joycon_l" => project_spi::<JoyConL>(case),
            "joycon_r" => project_spi::<JoyConR>(case),
            model => panic!("unsupported SPI fixture model: {model}"),
        };
        assert_expected(case, actual);
    }
}

#[test]
fn subcommand_cases_project_the_python_reply_and_session() {
    let document = fixture_document();
    let cases = cases_of_kind(&document, "subcommand").collect::<Vec<_>>();
    assert_eq!(cases.len(), 13);

    for case in cases {
        assert_case_keys(case);
        let actual = match case["model"].as_str().expect("model must be a string") {
            "pro" => project_subcommand::<Pro>(case),
            "joycon_l" => project_subcommand::<JoyConL>(case),
            "joycon_r" => project_subcommand::<JoyConR>(case),
            model => panic!("unsupported subcommand fixture model: {model}"),
        };
        assert_expected(case, actual);
    }
}

fn project_stick_conversion(case: &Value) -> Value {
    assert_exact_keys(&case["input"], &["values"], case_id(case));
    let values = case["input"]["values"]
        .as_array()
        .expect("values must be an array");
    let normalized = values
        .iter()
        .map(|value| {
            let value = value.as_f64().expect("normalized value must be numeric");
            let normalized = value as f32;
            let (x, y) = Stick::normalized(normalized, normalized).unwrap().axes();
            json!({"value": value, "raw": [x, y]})
        })
        .collect::<Vec<_>>();
    json!({"outcome": "values", "normalized": normalized})
}

fn project_imu_conversion(case: &Value) -> Value {
    assert_exact_keys(&case["input"], &["accel_g", "gyro_dps"], case_id(case));
    let gyro = numeric_triplet(&case["input"]["gyro_dps"]);
    let accel = numeric_triplet(&case["input"]["accel_g"]);
    let gyro_frame = ImuFrame::gyro_rate(
        gyro[0].to_radians(),
        gyro[1].to_radians(),
        gyro[2].to_radians(),
    )
    .unwrap();
    let accel_frame = ImuFrame::accel_g(accel[0], accel[1], accel[2]).unwrap();

    json!({
        "outcome": "values",
        "gyro_raw": gyro_frame.gyro(),
        "accel_raw": accel_frame.accel(),
    })
}

fn project_custom_stick_input(case: &Value) -> Value {
    assert_eq!(case["model"], "pro");
    assert_exact_keys(&case["input"], &["left", "right"], case_id(case));
    let left = raw_stick(&case["input"]["left"]);
    let right = raw_stick(&case["input"]["right"]);
    let state = InputState::<Pro>::neutral().with_sticks(left, right);
    let bytes = *encode_0x30(&state, 0, &[0; 36]).bytes();
    let decoded = json!({
        "left_stick_hex": encode_hex(&bytes[6..9]),
        "right_stick_hex": encode_hex(&bytes[9..12]),
    });
    bytes_expected(&bytes, decoded)
}

fn project_input_report<M: ControllerModel>(case: &Value) -> Value {
    let id = case_id(case);
    assert_eq!(case["model"], M::PROFILE_NAME, "{id}: model");
    let input = &case["input"];
    match id {
        "input.pro.neutral" | "input.joycon_l.neutral" | "input.joycon_r.neutral" => {
            assert_exact_keys(input, &["buttons", "imu", "sticks", "timer"], id);
            assert_eq!(input["imu"], "neutral");
            assert_eq!(input["sticks"], "neutral");
        }
        "input.pro.all_buttons" | "input.joycon_l.all_buttons" | "input.joycon_r.all_buttons" => {
            assert_exact_keys(input, &["buttons", "timer"], id);
        }
        id if id.ends_with(".standard_imu") => {
            assert_exact_keys(input, &["frames", "imu_mode"], id);
        }
        id if id.contains(".quaternion_mode_") => {
            assert_exact_keys(
                input,
                &["frames", "imu_mode", "now_ns", "previous_report_ns"],
                id,
            );
        }
        _ => panic!("unhandled input fixture: {id}"),
    }

    let mut state = InputState::<M>::neutral();
    if let Some(buttons) = input.get("buttons") {
        let buttons = buttons
            .as_array()
            .expect("buttons must be an array")
            .iter()
            .map(|button| {
                Button::<M>::try_from(button_kind(text(button, "button")))
                    .unwrap_or_else(|error| panic!("{id}: {error}"))
            })
            .collect::<Vec<_>>();
        state = state.with_buttons(buttons);
    }
    if let Some(frames) = input.get("frames") {
        state = state.with_imu(raw_frames(frames));
    }

    let timer = input.get("timer").map_or(0, u8_number);
    let imu_block = input.get("imu_mode").map_or([0; 36], |value| {
        let mode = ImuMode::from_wire(u8_number(value)).expect("fixture IMU mode must be valid");
        let previous = input.get("previous_report_ns").map(u64_number);
        let current = ImuEncodingState::new([0.0, 0.0, 0.0, 1.0], previous);
        let now_ns = input.get("now_ns").map_or(0, u64_number);
        *encode_imu_block(
            current,
            mode,
            state.imu_frames(),
            M::SPEC.protocol.gyroscope_calibration,
            now_ns,
        )
        .block()
    });
    let bytes = *encode_0x30(&state, timer, &imu_block).bytes();
    let decoded = match id {
        "input.pro.neutral" | "input.joycon_l.neutral" | "input.joycon_r.neutral" => {
            json!({
                "report_id": bytes[0],
                "timer": bytes[1],
                "battery_connection": bytes[2],
                "button_hex": encode_hex(&bytes[3..6]),
                "left_stick_hex": encode_hex(&bytes[6..9]),
                "right_stick_hex": encode_hex(&bytes[9..12]),
                "vibrator": bytes[12],
                "imu_hex": encode_hex(&bytes[13..49]),
            })
        }
        "input.pro.all_buttons" | "input.joycon_l.all_buttons" | "input.joycon_r.all_buttons" => {
            json!({"button_hex": encode_hex(&bytes[3..6])})
        }
        id if id.ends_with(".standard_imu") || id.contains(".quaternion_mode_") => {
            json!({"imu_hex": encode_hex(&bytes[13..49])})
        }
        _ => unreachable!("input fixture ID was checked above"),
    };
    bytes_expected(&bytes, decoded)
}

fn project_spi<M: ControllerModel>(case: &Value) -> Value {
    assert_eq!(case["model"], M::PROFILE_NAME, "{}: model", case_id(case));
    assert_exact_keys(&case["input"], &["address", "size"], case_id(case));
    let address =
        u32::try_from(u64_number(&case["input"]["address"])).expect("SPI address must fit u32");
    let size =
        usize::try_from(u64_number(&case["input"]["size"])).expect("SPI size must fit usize");
    let spi = VirtualSpiFlash::<M>::new(None);
    let read = spi.read(address, size).unwrap();
    bytes_expected(read.as_slice(), json!({}))
}

fn project_subcommand<M: ControllerModel>(case: &Value) -> Value {
    assert_eq!(case["model"], M::PROFILE_NAME, "{}: model", case_id(case));
    assert_exact_keys(
        &case["input"],
        &["bluetooth_address_hex", "payload_hex", "subcommand_id"],
        case_id(case),
    );
    let input = &case["input"];
    let address = decode_hex(text(
        &input["bluetooth_address_hex"],
        "bluetooth_address_hex",
    ))
    .try_into()
    .expect("fixture Bluetooth address must be 6 bytes");
    let payload = decode_hex(text(&input["payload_hex"], "payload_hex"));
    let subcommand_id = u8_number(&input["subcommand_id"]);
    let protocol = SwitchHidProtocol::<M>::new(None, address);
    let mut raw = vec![0x01, 0x0A];
    raw.extend_from_slice(&NEUTRAL_RUMBLE);
    raw.push(subcommand_id);
    raw.extend_from_slice(&payload);
    let current = ProtocolSession::default();
    let prepared = protocol
        .prepare_output_report(&raw, &InputState::<M>::neutral(), 0, current)
        .unwrap();
    let OutputPreparation::Subcommand {
        request, outcome, ..
    } = prepared
    else {
        panic!("subcommand fixture produced rumble-only output");
    };
    assert_eq!(request.id(), subcommand_id);
    assert_eq!(request.payload(), payload);

    let (bytes, next_timer, next_session) = match outcome.unwrap() {
        PreparedOutputAction::Reply(reply) => (*reply.bytes(), reply.next_timer(), current),
        PreparedOutputAction::SessionReply(reply) => {
            (*reply.bytes(), reply.next_timer(), reply.next_session())
        }
    };
    assert_eq!(next_timer, 1, "{} timer candidate", case_id(case));
    let decoded = json!({
        "ack": bytes[13],
        "reply_to": bytes[14],
        "data_hex": encode_hex(&bytes[15..]),
    });
    let mut actual = bytes_expected(&bytes, decoded);
    actual
        .as_object_mut()
        .expect("bytes projection must be an object")
        .insert("session".to_owned(), session_expected(next_session));
    actual
}

fn session_expected(session: ProtocolSession) -> Value {
    json!({
        "report_mode": session.report_mode(),
        "report_mode_supported": session.report_mode_supported(),
        "unsupported_report_mode": session.unsupported_report_mode(),
        "player_lights": session.player_lights(),
        "imu_mode": session.imu_mode() as u8,
        "vibration_enabled": session.vibration_enabled(),
        "protocol_ready": session.protocol_ready(),
    })
}

fn bytes_expected(bytes: &[u8], decoded: Value) -> Value {
    json!({
        "outcome": "bytes",
        "hex": encode_hex(bytes),
        "length": bytes.len(),
        "decoded": decoded,
    })
}

fn raw_frames(value: &Value) -> [ImuFrame; 3] {
    let frames = value.as_array().expect("frames must be an array");
    assert_eq!(frames.len(), 3);
    std::array::from_fn(|index| {
        let values = frames[index].as_array().expect("frame must be an array");
        assert_eq!(values.len(), 6);
        let values = std::array::from_fn::<i16, 6, _>(|value_index| {
            i16::try_from(
                values[value_index]
                    .as_i64()
                    .expect("frame values must be signed integers"),
            )
            .expect("frame value must fit i16")
        });
        ImuFrame::raw(
            [values[0], values[1], values[2]],
            [values[3], values[4], values[5]],
        )
    })
}

fn raw_stick(value: &Value) -> Stick {
    let axes = value.as_array().expect("stick axes must be an array");
    assert_eq!(axes.len(), 2);
    Stick::raw(
        u16::try_from(u64_number(&axes[0])).expect("stick x must fit u16"),
        u16::try_from(u64_number(&axes[1])).expect("stick y must fit u16"),
    )
    .unwrap()
}

fn numeric_triplet(value: &Value) -> [f64; 3] {
    let values = value.as_array().expect("triplet must be an array");
    assert_eq!(values.len(), 3);
    std::array::from_fn(|index| values[index].as_f64().expect("value must be numeric"))
}

fn button_kind(value: &str) -> ButtonKind {
    match value {
        "A" => ButtonKind::A,
        "B" => ButtonKind::B,
        "X" => ButtonKind::X,
        "Y" => ButtonKind::Y,
        "L" => ButtonKind::L,
        "R" => ButtonKind::R,
        "ZL" => ButtonKind::ZL,
        "ZR" => ButtonKind::ZR,
        "PLUS" => ButtonKind::Plus,
        "MINUS" => ButtonKind::Minus,
        "HOME" => ButtonKind::Home,
        "CAPTURE" => ButtonKind::Capture,
        "LEFT_STICK" => ButtonKind::LeftStick,
        "RIGHT_STICK" => ButtonKind::RightStick,
        "SL" => ButtonKind::SL,
        "SR" => ButtonKind::SR,
        "DPAD_UP" => ButtonKind::DpadUp,
        "DPAD_DOWN" => ButtonKind::DpadDown,
        "DPAD_LEFT" => ButtonKind::DpadLeft,
        "DPAD_RIGHT" => ButtonKind::DpadRight,
        other => panic!("unknown fixture button: {other}"),
    }
}

fn fixture_document() -> Value {
    serde_json::from_str(FIXTURE).expect("generated protocol fixture must be valid JSON")
}

fn cases_of_kind<'a>(document: &'a Value, kind: &'a str) -> impl Iterator<Item = &'a Value> {
    document["cases"]
        .as_array()
        .expect("cases must be an array")
        .iter()
        .filter(move |case| case["kind"] == kind)
}

fn assert_case_keys(case: &Value) {
    assert_exact_keys(
        case,
        &["expected", "id", "input", "kind", "model"],
        case_id(case),
    );
}

fn assert_exact_keys(value: &Value, expected: &[&str], context: &str) {
    let actual = value
        .as_object()
        .unwrap_or_else(|| panic!("{context}: expected an object"))
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected, "{context}: object keys");
}

fn assert_expected(case: &Value, actual: Value) {
    if actual != case["expected"]
        && actual["outcome"] == "bytes"
        && case["expected"]["outcome"] == "bytes"
    {
        let expected = decode_hex(text(&case["expected"]["hex"], "expected hex"));
        let actual_bytes = decode_hex(text(&actual["hex"], "actual hex"));
        if actual_bytes != expected {
            let offset = expected
                .iter()
                .zip(&actual_bytes)
                .position(|(expected, actual)| expected != actual)
                .unwrap_or(expected.len().min(actual_bytes.len()));
            let expected_byte = expected
                .get(offset)
                .map_or_else(|| "<end>".to_owned(), |byte| format!("0x{byte:02x}"));
            let actual_byte = actual_bytes
                .get(offset)
                .map_or_else(|| "<end>".to_owned(), |byte| format!("0x{byte:02x}"));
            panic!(
                "{} ({}) first byte mismatch at offset 0x{offset:02x}: expected {expected_byte}, actual {actual_byte}; expected decoded: {}; actual decoded: {}",
                case_id(case),
                case["model"],
                case["expected"]["decoded"],
                actual["decoded"],
            );
        }
    }
    assert_eq!(actual, case["expected"], "fixture case: {}", case_id(case));
}

fn case_id(case: &Value) -> &str {
    text(&case["id"], "case id")
}

fn text<'a>(value: &'a Value, context: &str) -> &'a str {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{context} must be a string"))
}

fn u8_number(value: &Value) -> u8 {
    u8::try_from(u64_number(value)).expect("value must fit u8")
}

fn u64_number(value: &Value) -> u64 {
    value.as_u64().expect("value must be an unsigned integer")
}

fn decode_hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2), "hex length must be even");
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).expect("fixture hex must be valid")
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
