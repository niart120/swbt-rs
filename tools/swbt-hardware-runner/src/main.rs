#![forbid(unsafe_code)]

use std::process::ExitCode;

fn main() -> ExitCode {
    ExitCode::from(swbt_hardware_runner::run(std::env::args().skip(1)))
}
