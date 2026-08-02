//! Pure Nintendo Switch HID protocol transformations.

mod error;
mod facade;
mod imu;
mod input_report;
mod output_report;
mod session;
mod spi;
mod subcommand;

pub use error::ProtocolError;
pub use facade::{InputPreparation, PreparedOutputAction, SwitchHidProtocol};
pub use imu::ImuEncodingState;
pub use output_report::{OutputReport, RawRumble, SubcommandRequest, parse_output_report};
pub use session::ProtocolSession;
pub use subcommand::{PreparedSessionReply, PreparedSubcommandReply};

#[cfg(test)]
mod tests;
