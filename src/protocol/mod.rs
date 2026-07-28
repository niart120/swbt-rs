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
mod spi;

#[cfg(test)]
mod tests;
