use std::fmt;

use crate::model::ButtonKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtocolError {
    SpiReadTooLarge {
        size: usize,
        maximum: usize,
    },
    SpiAddressOutOfRange {
        address: u32,
        size: usize,
    },
    OutputReportEmpty,
    UnsupportedOutputReport {
        report_id: u8,
    },
    TruncatedOutputReport {
        report_id: u8,
        minimum: usize,
        actual: usize,
    },
    SubcommandReplyDataTooLarge {
        size: usize,
        maximum: usize,
    },
    UnsupportedTriggerElapsedButton {
        button: ButtonKind,
    },
    TruncatedSpiReadRequest {
        minimum: usize,
        actual: usize,
    },
    MissingSubcommandArgument {
        subcommand_id: u8,
    },
    UnsupportedImuMode {
        requested: u8,
        accepted: &'static [u8],
    },
    InvalidVibrationValue {
        requested: u8,
    },
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
            Self::OutputReportEmpty => formatter.write_str("output report is empty"),
            Self::UnsupportedOutputReport { report_id } => {
                write!(formatter, "unsupported output report id: 0x{report_id:02x}")
            }
            Self::TruncatedOutputReport {
                report_id: 0x01, ..
            } => formatter
                .write_str("0x01 output report must include packet, rumble, and subcommand"),
            Self::TruncatedOutputReport {
                report_id: 0x10, ..
            } => formatter.write_str("0x10 output report must include packet and rumble"),
            Self::TruncatedOutputReport {
                report_id,
                minimum,
                actual,
            } => write!(
                formatter,
                "output report 0x{report_id:02x} needs {minimum} bytes, got {actual}"
            ),
            Self::SubcommandReplyDataTooLarge { size, maximum } => write!(
                formatter,
                "subcommand reply data must be {maximum} bytes or less: {size}"
            ),
            Self::UnsupportedTriggerElapsedButton { button } => {
                write!(formatter, "unsupported trigger elapsed button: {button:?}")
            }
            Self::TruncatedSpiReadRequest { .. } => {
                formatter.write_str("SPI read subcommand must include address and size")
            }
            Self::MissingSubcommandArgument {
                subcommand_id: 0x03,
            } => formatter
                .write_str("set input report mode subcommand must include one argument byte"),
            Self::MissingSubcommandArgument {
                subcommand_id: 0x30,
            } => formatter.write_str("set player lights subcommand must include one argument byte"),
            Self::MissingSubcommandArgument {
                subcommand_id: 0x40,
            } => formatter.write_str("enable IMU subcommand must include one argument byte"),
            Self::MissingSubcommandArgument {
                subcommand_id: 0x48,
            } => formatter.write_str("enable vibration subcommand must include one argument byte"),
            Self::MissingSubcommandArgument { subcommand_id } => write!(
                formatter,
                "subcommand 0x{subcommand_id:02x} must include one argument byte"
            ),
            Self::UnsupportedImuMode {
                requested: _,
                accepted,
            } => {
                formatter.write_str("enable IMU subcommand argument must be one of: ")?;
                for (index, mode) in accepted.iter().enumerate() {
                    if index != 0 {
                        formatter.write_str(", ")?;
                    }
                    write!(formatter, "0x{mode:02X}")?;
                }
                Ok(())
            }
            Self::InvalidVibrationValue { .. } => {
                formatter.write_str("enable vibration subcommand argument must be 0x00 or 0x01")
            }
        }
    }
}

impl std::error::Error for ProtocolError {}
