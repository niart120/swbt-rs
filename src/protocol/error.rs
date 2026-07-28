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
        }
    }
}

impl std::error::Error for ProtocolError {}
