use std::fmt;

use crate::error::{Error, ErrorKind};

const ADDRESS_TEXT_LENGTH: usize = 17;
const ADDRESS_OCTETS: usize = 6;
const RESERVED_INQUIRY_LAP_MIN: u32 = 0x9E_8B_00;
const RESERVED_INQUIRY_LAP_MAX: u32 = 0x9E_8B_3F;

/// An individual, locally administered six-octet Bluetooth address.
///
/// The reserved inquiry LAP range `0x9E8B00..=0x9E8B3F` is excluded. Its
/// [`Debug`](std::fmt::Debug) representation redacts all address octets.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalAddress([u8; ADDRESS_OCTETS]);

impl LocalAddress {
    /// Parses `XX:XX:XX:XX:XX:XX` notation.
    ///
    /// Hexadecimal digits are case-insensitive.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when the notation is malformed, the
    /// address is not individual and locally administered, or its lower three
    /// octets are a reserved inquiry LAP.
    pub fn parse(value: &str) -> crate::Result<Self> {
        let bytes = value.as_bytes();
        if bytes.len() != ADDRESS_TEXT_LENGTH
            || [2, 5, 8, 11, 14]
                .into_iter()
                .any(|index| bytes[index] != b':')
        {
            return Err(invalid_address(
                "local address must contain six hexadecimal octets in XX:XX:XX:XX:XX:XX form",
            ));
        }

        let mut octets = [0_u8; ADDRESS_OCTETS];
        for (index, octet) in octets.iter_mut().enumerate() {
            let offset = index * 3;
            let high = hex_nibble(bytes[offset]);
            let low = hex_nibble(bytes[offset + 1]);
            let (Some(high), Some(low)) = (high, low) else {
                return Err(invalid_address(
                    "local address must contain six hexadecimal octets in XX:XX:XX:XX:XX:XX form",
                ));
            };
            *octet = high << 4 | low;
        }

        Self::try_from(octets)
    }

    /// Returns the six address octets in display order.
    #[must_use]
    pub const fn octets(self) -> [u8; ADDRESS_OCTETS] {
        self.0
    }
}

impl TryFrom<[u8; ADDRESS_OCTETS]> for LocalAddress {
    type Error = Error;

    fn try_from(octets: [u8; ADDRESS_OCTETS]) -> Result<Self, Self::Error> {
        let first = octets[0];
        if first & 0x01 != 0 {
            return Err(invalid_address(
                "local address must be an individual address",
            ));
        }
        if first & 0x02 == 0 {
            return Err(invalid_address(
                "local address must be locally administered",
            ));
        }

        let lap = u32::from_be_bytes([0, octets[3], octets[4], octets[5]]);
        if (RESERVED_INQUIRY_LAP_MIN..=RESERVED_INQUIRY_LAP_MAX).contains(&lap) {
            return Err(invalid_address(
                "local address must not use a reserved inquiry LAP",
            ));
        }

        Ok(Self(octets))
    }
}

impl fmt::Debug for LocalAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalAddress(<redacted>)")
    }
}

/// Identity persisted in a newly created pairing profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProfileIdentity {
    /// Keep the Bluetooth adapter's current local address.
    AdapterDefault,
    /// Use an explicit locally administered address.
    ///
    /// The supported and hardware-verified path is a CSR8510 A10 USB adapter
    /// using WinUSB on Windows. The address is written to volatile controller
    /// storage and remains active until the adapter is physically
    /// power-cycled. Manage each profile, address, and adapter as one set. If
    /// verification after a write fails, operations return
    /// [`ErrorKind::AdapterIdentityRecoveryRequired`] and must not be retried
    /// until a physical power cycle restores the adapter's original identity.
    LocalAddress(LocalAddress),
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn invalid_address(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}
