use std::process::{Command, Output};

#[test]
fn help_exposes_one_binary_with_three_hardware_scenarios() {
    let output = run(["help"]);

    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
    let help = String::from_utf8(output.stdout).expect("help is UTF-8");
    for scenario in ["pro-periodic", "pro-profile", "joycon-profile"] {
        assert!(help.contains(&format!("swbt-hardware-runner {scenario}")));
    }
}

#[test]
fn invalid_or_incomplete_scenarios_stop_with_usage_before_hardware_open() {
    for arguments in [
        vec!["unknown"],
        vec!["pro-periodic"],
        vec!["pro-profile"],
        vec!["joycon-profile"],
    ] {
        let output = run(arguments);
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8(output.stderr)
                .expect("usage is UTF-8")
                .contains("swbt-hardware-runner")
        );
    }
}

fn run(arguments: impl IntoIterator<Item = impl AsRef<std::ffi::OsStr>>) -> Output {
    Command::new(env!("CARGO_BIN_EXE_swbt-hardware-runner"))
        .args(arguments)
        .output()
        .expect("run swbt-hardware-runner")
}
