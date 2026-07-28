use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolError {
    SpiReadTooLarge { size: usize, maximum: usize },
    SpiAddressOutOfRange { address: u32, size: usize },
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SpiReadTooLarge { size, maximum } => {
                write!(
                    formatter,
                    "SPI read size must be {maximum} bytes or less: {size}"
                )
            }
            Self::SpiAddressOutOfRange { address, size } => {
                write!(
                    formatter,
                    "SPI read is outside address space: address=0x{address:x}, size={size}"
                )
            }
        }
    }
}

impl std::error::Error for ProtocolError {}
