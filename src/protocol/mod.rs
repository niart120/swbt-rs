//! Pure Nintendo Switch HID protocol transformations.

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod error;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod facade;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod imu;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod input_report;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod output_report;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod session;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod spi;
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M1 builds this module before M2 runtime integration"
    )
)]
mod subcommand;

pub(crate) use error::ProtocolError;
pub(crate) use facade::{InputPreparation, PreparedOutputAction, SwitchHidProtocol};
pub(crate) use imu::ImuEncodingState;
pub(crate) use output_report::SubcommandRequest;
pub(crate) use session::ProtocolSession;
pub(crate) use subcommand::{PreparedSessionReply, PreparedSubcommandReply};

#[cfg(test)]
pub(crate) use output_report::{OutputReport, parse_output_report};
#[cfg(test)]
pub(crate) use subcommand::DeviceInfoBluetoothAddress;

#[cfg(test)]
mod tests;
