//! Pure Nintendo Switch HID protocol transformations.

mod error;
mod facade;
mod imu;
mod input_report;
mod output_report;
mod session;
mod spi;
mod subcommand;

pub(crate) use error::ProtocolError;
pub(crate) use facade::{InputPreparation, PreparedOutputAction, SwitchHidProtocol};
pub(crate) use imu::ImuEncodingState;
pub(crate) use output_report::{OutputReport, RawRumble, SubcommandRequest, parse_output_report};
pub(crate) use session::ProtocolSession;
pub(crate) use subcommand::{PreparedSessionReply, PreparedSubcommandReply};

#[cfg(test)]
mod tests;
