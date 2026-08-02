use crate::error::ErrorKind;

use super::LocalAddress;

#[test]
fn local_address_parses_the_python_profile_notation_and_redacts_debug() {
    let address = LocalAddress::parse("02:12:34:56:78:9a").expect("valid local address must parse");

    assert_eq!(address.octets(), [0x02, 0x12, 0x34, 0x56, 0x78, 0x9A]);
    assert_eq!(
        address,
        LocalAddress::try_from([0x02, 0x12, 0x34, 0x56, 0x78, 0x9A])
            .expect("valid octets must construct the same address")
    );
    let debug = format!("{address:?}");
    assert!(debug.contains("<redacted>"));
    assert!(!debug.contains("02:12:34:56:78:9A"));
}

#[test]
fn local_address_rejects_invalid_shape_and_address_classes() {
    for value in [
        "02:12:34:56:78",
        "02-12-34-56-78-9A",
        "02:12:34:56:78:GG",
        "03:12:34:56:78:9A",
        "00:12:34:56:78:9A",
        "02:12:34:9E:8B:00",
        "02:12:34:9E:8B:33",
        "02:12:34:9E:8B:3F",
    ] {
        let error = LocalAddress::parse(value).expect_err("invalid local address must fail");
        assert_eq!(error.kind(), ErrorKind::InvalidInput, "{value}");
        assert!(!error.to_string().contains(value));
    }
}

#[test]
fn local_address_accepts_values_adjacent_to_the_reserved_inquiry_lap() {
    for octets in [
        [0x02, 0x12, 0x34, 0x9E, 0x8A, 0xFF],
        [0x02, 0x12, 0x34, 0x9E, 0x8B, 0x40],
    ] {
        let address =
            LocalAddress::try_from(octets).expect("adjacent inquiry LAP must remain valid");

        assert_eq!(address.octets(), octets);
    }
}
