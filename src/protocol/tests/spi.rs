use crate::{
    profile::{ControllerColors, ControllerKind, Rgb24},
    protocol::{
        error::ProtocolError,
        spi::{MAX_READ_SIZE, VirtualSpiFlash},
    },
};

#[test]
fn virtual_spi_projects_python_model_seeds() {
    let cases = [
        (ControllerKind::Pro, 0x03, "323232ffffff00b2ffff3b30"),
        (ControllerKind::JoyConL, 0x01, "00b2ff32323200b2ff00b2ff"),
        (ControllerKind::JoyConR, 0x02, "ff3b30323232ff3b30ff3b30"),
    ];

    for (model, device_type, colors_hex) in cases {
        let spi = VirtualSpiFlash::new(model, None);
        assert_eq!(spi.read(0x6012, 1).unwrap().as_slice(), [device_type]);
        assert_eq!(spi.read(0x601B, 1).unwrap().as_slice(), [0x01]);
        assert_eq!(
            spi.read(0x6020, 24).unwrap().as_slice(),
            decode_hex("0000000000000040004000400000000000003b343b343b34")
        );
        assert_eq!(
            spi.read(0x6050, 12).unwrap().as_slice(),
            decode_hex(colors_hex)
        );
        assert_eq!(spi.read(0x70000, 2).unwrap().as_slice(), [0xFF, 0xFF]);
    }
}

#[test]
fn virtual_spi_uses_explicit_colors_without_changing_other_seeds() {
    let colors = ControllerColors::new(
        Rgb24::new(0x01, 0x02, 0x03),
        Rgb24::new(0x04, 0x05, 0x06),
        Rgb24::new(0x07, 0x08, 0x09),
        Rgb24::new(0x0A, 0x0B, 0x0C),
    );
    let spi = VirtualSpiFlash::new(ControllerKind::JoyConR, Some(colors));

    assert_eq!(
        spi.read(0x6050, 12).unwrap().as_slice(),
        [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
    );
    assert_eq!(spi.read(0x6012, 1).unwrap().as_slice(), [0x02]);
}

#[test]
fn virtual_spi_enforces_python_read_boundaries() {
    let spi = VirtualSpiFlash::new(ControllerKind::Pro, None);

    assert_eq!(
        spi.read(0x6012, MAX_READ_SIZE).unwrap().as_slice(),
        decode_hex("03ffffffffffffffff01ffffffff000000000000004000400040000000")
    );
    assert_eq!(
        spi.read(0x70000, MAX_READ_SIZE).unwrap().as_slice(),
        [0xFF; MAX_READ_SIZE]
    );
    assert_eq!(
        spi.read(0x70000, MAX_READ_SIZE + 1),
        Err(ProtocolError::SpiReadTooLarge {
            size: MAX_READ_SIZE + 1,
            maximum: MAX_READ_SIZE,
        })
    );
    assert_eq!(spi.read(0x80000, 0).unwrap().as_slice(), []);
    assert_eq!(spi.read(0x7FFFF, 1).unwrap().as_slice(), [0xFF]);
    assert_eq!(
        spi.read(0x7FFFF, 2),
        Err(ProtocolError::SpiAddressOutOfRange {
            address: 0x7FFFF,
            size: 2,
        })
    );
    assert_eq!(
        spi.read(0x80001, 0),
        Err(ProtocolError::SpiAddressOutOfRange {
            address: 0x80001,
            size: 0,
        })
    );
    assert_eq!(
        spi.read(u32::MAX, 1),
        Err(ProtocolError::SpiAddressOutOfRange {
            address: u32::MAX,
            size: 1,
        })
    );
}

fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|digits| {
            let digits = std::str::from_utf8(digits).unwrap();
            u8::from_str_radix(digits, 16).unwrap()
        })
        .collect()
}
