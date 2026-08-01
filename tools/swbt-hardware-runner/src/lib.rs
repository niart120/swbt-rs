#![forbid(unsafe_code)]
//! Internal support for the workspace-only hardware evidence runner.

use std::io;

mod scenarios {
    pub(super) mod joycon_profile;
    pub(super) mod pro_periodic;
    pub(super) mod pro_profile;
}

const HELP: &str = "\
Usage:
  swbt-hardware-runner pro-periodic --adapter <selector> --profile <new-path> --pair-timeout-secs <1..600> --run <1..20>
  swbt-hardware-runner pro-profile --adapter <selector> --profile <path> --mode <periodic|direct> --setup <normal|post-power-cycle|stale-bond> --connect-timeout-secs <1..600> --run <1..99> [--stale-source-profile <existing-path>]
  swbt-hardware-runner joycon-profile --adapter <selector> --profile <path> --model <left|right> --mode <periodic|direct> --connection <pair|reconnect> --timeout-secs <1..600> [--pre-input-idle-ms <0..10000>] --run <1..99>
  swbt-hardware-runner help
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scenario {
    ProPeriodic,
    ProProfile,
    JoyConProfile,
}

#[derive(Debug, PartialEq, Eq)]
enum Command {
    Help,
    Scenario {
        scenario: Scenario,
        arguments: Vec<String>,
    },
}

fn parse_command<I, S>(arguments: I) -> Result<Command, ()>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut arguments = arguments.into_iter().map(Into::into);
    let command = arguments.next().ok_or(())?;
    let remaining = arguments.collect::<Vec<_>>();
    match command.as_str() {
        "help" | "--help" | "-h" if remaining.is_empty() => Ok(Command::Help),
        "pro-periodic" => Ok(Command::Scenario {
            scenario: Scenario::ProPeriodic,
            arguments: remaining,
        }),
        "pro-profile" => Ok(Command::Scenario {
            scenario: Scenario::ProProfile,
            arguments: remaining,
        }),
        "joycon-profile" => Ok(Command::Scenario {
            scenario: Scenario::JoyConProfile,
            arguments: remaining,
        }),
        _ => Err(()),
    }
}

/// Runs the workspace-only hardware evidence command and returns its exit code.
pub fn run<I, S>(arguments: I) -> u8
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    match parse_command(arguments) {
        Ok(Command::Help) => write(io::stdout().lock(), HELP, 0),
        Ok(Command::Scenario {
            scenario: Scenario::ProPeriodic,
            arguments,
        }) => scenarios::pro_periodic::run(arguments),
        Ok(Command::Scenario {
            scenario: Scenario::ProProfile,
            arguments,
        }) => scenarios::pro_profile::run(arguments),
        Ok(Command::Scenario {
            scenario: Scenario::JoyConProfile,
            arguments,
        }) => scenarios::joycon_profile::run(arguments),
        Err(()) => write(io::stderr().lock(), HELP, 2),
    }
}

fn write(mut output: impl io::Write, text: &str, success_code: u8) -> u8 {
    if output.write_all(text.as_bytes()).is_ok() {
        success_code
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, Scenario, parse_command};

    #[test]
    fn entry_parser_selects_each_scenario_and_preserves_its_arguments() {
        for (name, expected) in [
            ("pro-periodic", Scenario::ProPeriodic),
            ("pro-profile", Scenario::ProProfile),
            ("joycon-profile", Scenario::JoyConProfile),
        ] {
            let command = parse_command([name, "--adapter", "usb:0"]).expect("valid scenario");
            assert_eq!(
                command,
                Command::Scenario {
                    scenario: expected,
                    arguments: vec!["--adapter".to_owned(), "usb:0".to_owned()],
                }
            );
        }
    }

    #[test]
    fn entry_parser_keeps_help_explicit_and_rejects_unknown_or_missing_scenarios() {
        for help in ["help", "--help", "-h"] {
            assert_eq!(parse_command([help]), Ok(Command::Help));
        }
        assert!(parse_command(std::iter::empty::<&str>()).is_err());
        assert!(parse_command(["unknown"]).is_err());
        assert!(parse_command(["help", "extra"]).is_err());
    }
}
