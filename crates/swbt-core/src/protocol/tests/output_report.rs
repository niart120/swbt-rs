use crate::protocol::{
    error::ProtocolError,
    output_report::{OutputReport, parse_output_report},
};

#[test]
fn output_0x01_preserves_fields_and_borrows_payload() {
    let raw = [
        0x01, 0xAB, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40, 0x03, 0x30,
    ];

    let report = parse_output_report(&raw).unwrap();

    assert_eq!(report.report_id(), 0x01);
    assert_eq!(report.packet_id(), 0xAB);
    assert_eq!(
        report.rumble().bytes(),
        &[0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40]
    );
    let request = report.subcommand().unwrap();
    assert_eq!(request.id(), 0x03);
    assert_eq!(request.payload(), &[0x30]);
    assert_eq!(request.payload().as_ptr(), raw[11..].as_ptr());
}

#[test]
fn output_0x10_is_rumble_only_and_ignores_trailing_bytes() {
    let raw = [
        0x10, 0x2A, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40, 0xAA, 0xBB,
    ];

    let report = parse_output_report(&raw).unwrap();

    assert_eq!(report.report_id(), 0x10);
    assert_eq!(report.packet_id(), 0x2A);
    assert_eq!(
        report.rumble().bytes(),
        &[0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40]
    );
    assert_eq!(report.subcommand(), None);
}

#[test]
fn output_parser_returns_structured_errors_for_fixture_failures() {
    assert_eq!(
        parse_output_report(&[]),
        Err(ProtocolError::OutputReportEmpty)
    );
    assert_eq!(
        parse_output_report(&[0x99]),
        Err(ProtocolError::UnsupportedOutputReport { report_id: 0x99 })
    );
    assert_eq!(
        parse_output_report(&[0x01, 0xAB, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40]),
        Err(ProtocolError::TruncatedOutputReport {
            report_id: 0x01,
            minimum: 11,
            actual: 10,
        })
    );
    assert_eq!(
        parse_output_report(&[0x10, 0x2A, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40]),
        Err(ProtocolError::TruncatedOutputReport {
            report_id: 0x10,
            minimum: 10,
            actual: 9,
        })
    );
}

#[test]
fn output_parser_never_panics_for_short_arbitrary_byte_sequences() {
    assert_eq!(
        parse_output_report(&[]),
        Err(ProtocolError::OutputReportEmpty)
    );

    for report_id in u8::MIN..=u8::MAX {
        let mut raw = [0; 32];
        raw[0] = report_id;
        for (index, byte) in raw[1..].iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(37).wrapping_add(report_id);
        }
        for length in 1..=raw.len() {
            match parse_output_report(&raw[..length]) {
                Ok(OutputReport::Subcommand { .. }) => {
                    assert_eq!(report_id, 0x01);
                    assert!(length >= 11);
                }
                Ok(OutputReport::Rumble { .. }) => {
                    assert_eq!(report_id, 0x10);
                    assert!(length >= 10);
                }
                Err(_) => {}
            }
        }
    }
}
