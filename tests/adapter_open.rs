#![cfg(feature = "adapter-tests")]

use std::{
    env,
    process::Command,
    sync::{Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use swbt::{AdapterInfo, AdapterSelector, ErrorKind, LifecycleState, ProController, list_adapters};

const TARGET_VENDOR_ID: u16 = 0x0a12;
const TARGET_PRODUCT_ID: u16 = 0x0001;
const PROCESS_CHILD_ENV: &str = "SWBT_ADAPTER_PROCESS_CHILD";
const SERIAL_ENV: &str = "SWBT_ADAPTER_SERIAL";
const UNPLUG_ENV: &str = "SWBT_RUN_UNPLUG_TEST";
const UNPLUG_TIMEOUT: Duration = Duration::from_secs(60);
const UNPLUG_POLL_INTERVAL: Duration = Duration::from_millis(20);
static HARDWARE_LOCK: Mutex<()> = Mutex::new(());

#[test]
#[ignore = "requires the CSR8510 A10 target adapter"]
fn no_open_discovery_finds_the_stable_target_adapter() {
    let _guard = hardware_lock();
    let first = target_adapter();
    let second = target_adapter();

    assert_eq!(first.info, second.info);
    assert_eq!(first.info.vendor_id(), TARGET_VENDOR_ID);
    assert_eq!(first.info.product_id(), TARGET_PRODUCT_ID);
}

#[test]
#[ignore = "claims and initializes the CSR8510 A10 target adapter"]
fn selector_aliases_open_initialize_and_close_the_target_adapter() {
    let _guard = hardware_lock();
    let target = target_adapter();
    let mut selectors = vec![
        ("candidate index", target.info.selector().clone()),
        (
            "VID/PID",
            AdapterSelector::from(format!(
                "usb:{TARGET_VENDOR_ID:04x}:{TARGET_PRODUCT_ID:04x}"
            )),
        ),
        (
            "VID/PID occurrence",
            AdapterSelector::from(format!(
                "usb:{TARGET_VENDOR_ID:04x}:{TARGET_PRODUCT_ID:04x}#{}",
                target.occurrence
            )),
        ),
    ];

    let ports = target
        .info
        .port_numbers()
        .expect("target adapter exposes a bus/port path");
    assert!(!ports.is_empty(), "target adapter port path is not empty");
    let ports = ports
        .iter()
        .map(u8::to_string)
        .collect::<Vec<_>>()
        .join(".");
    selectors.push((
        "bus/port path",
        AdapterSelector::from(format!("usb:{}-{ports}", target.info.bus_number())),
    ));
    match env::var(SERIAL_ENV) {
        Ok(serial) => selectors.push((
            "serial",
            AdapterSelector::from(format!(
                "usb:{TARGET_VENDOR_ID:04x}:{TARGET_PRODUCT_ID:04x}/{serial}"
            )),
        )),
        Err(env::VarError::NotPresent) if !target.info.has_serial_number() => {}
        Err(env::VarError::NotPresent) => {
            eprintln!("{SERIAL_ENV} is unset; the serial alias was not exercised");
        }
        Err(env::VarError::NotUnicode(_)) => panic!("{SERIAL_ENV} is not valid Unicode"),
    }

    for (label, selector) in selectors {
        open_initialize_and_close(label, selector);
    }
}

#[test]
#[ignore = "claims and reopens the CSR8510 A10 target adapter 100 times"]
fn adapter_reopens_after_one_hundred_complete_lifecycles() {
    let _guard = hardware_lock();
    let selector = target_adapter().info.selector().clone();

    for iteration in 1..=100 {
        open_initialize_and_close(&format!("iteration {iteration}"), selector.clone());
    }
}

#[test]
#[ignore = "starts two child processes that claim and release the CSR8510 A10 target adapter"]
fn adapter_reopens_after_previous_process_exit() {
    let _guard = hardware_lock();
    if env::var_os(PROCESS_CHILD_ENV).is_some() {
        open_initialize_and_close("child process", target_adapter().info.selector().clone());
        return;
    }

    let test_binary = env::current_exe().expect("resolve adapter_open test binary");
    for attempt in 1..=2 {
        let output = Command::new(&test_binary)
            .args([
                "--exact",
                "adapter_reopens_after_previous_process_exit",
                "--ignored",
                "--nocapture",
            ])
            .env(PROCESS_CHILD_ENV, "1")
            .output()
            .expect("start adapter lifecycle child process");
        assert!(
            output.status.success(),
            "adapter child process {attempt} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
#[ignore = "requires a person to unplug the CSR8510 A10 after open"]
fn unplug_terminates_the_worker_and_still_allows_cleanup() {
    let _guard = hardware_lock();
    if env::var_os(UNPLUG_ENV).is_none() {
        eprintln!("{UNPLUG_ENV} is unset; manual unplug test was not run");
        return;
    }

    let mut controller = ProController::builder(target_adapter().info.selector().clone())
        .build()
        .expect("build target adapter controller");
    controller
        .open()
        .expect("open and initialize target adapter before unplug");
    eprintln!("unplug the CSR8510 A10 now");

    let deadline = Instant::now() + UNPLUG_TIMEOUT;
    while controller.status().lifecycle != LifecycleState::Failed {
        assert!(
            Instant::now() < deadline,
            "worker did not observe adapter unplug within {UNPLUG_TIMEOUT:?}"
        );
        thread::sleep(UNPLUG_POLL_INTERVAL);
    }

    let error = controller
        .close_without_neutral()
        .expect_err("terminal reader failure remains the worker outcome");
    assert_eq!(error.kind(), ErrorKind::WorkerFailed);
    assert_eq!(controller.status().lifecycle, LifecycleState::Failed);
}

struct TargetAdapter {
    info: AdapterInfo,
    occurrence: usize,
}

fn target_adapter() -> TargetAdapter {
    let adapters = list_adapters().expect("enumerate USB Bluetooth HCI adapters");
    let mut occurrence = 0;
    for info in adapters {
        if info.vendor_id() == TARGET_VENDOR_ID && info.product_id() == TARGET_PRODUCT_ID {
            return TargetAdapter { info, occurrence };
        }
        if info.vendor_id() == TARGET_VENDOR_ID {
            occurrence += usize::from(info.product_id() == TARGET_PRODUCT_ID);
        }
    }
    panic!("CSR8510 A10 0A12:0001 is not present");
}

fn open_initialize_and_close(label: &str, selector: AdapterSelector) {
    let mut controller = ProController::builder(selector)
        .build()
        .unwrap_or_else(|error| panic!("{label}: build failed: {error}"));
    controller
        .open()
        .unwrap_or_else(|error| panic!("{label}: open/initialize failed: {error}"));
    let open = controller.status();
    assert_eq!(open.lifecycle, LifecycleState::Open, "{label}");
    assert!(!open.connected, "{label}");

    controller
        .close_without_neutral()
        .unwrap_or_else(|error| panic!("{label}: close/join failed: {error}"));
    assert_eq!(
        controller.status().lifecycle,
        LifecycleState::Closed,
        "{label}"
    );
}

fn hardware_lock() -> MutexGuard<'static, ()> {
    HARDWARE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
