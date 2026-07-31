use super::{
    ButtonKind, Command, ConnectionOperation, ConnectionRequest, ControllerKind, ControllerModel,
    ControllerSelection, ErrorKind, ImuRunEvidence, ProbeBackend, ProbeController, ReportingMode,
    ReportingSelection, SafeAdapter, connection_completed_record, execute, horizontal_yaw_frame,
    open_and_close,
};
use swbt::{Button, ImuFrame, ReportingKind};

#[test]
fn fake_adapter_listing_emits_only_safe_descriptor_fields() {
    let mut backend = FakeBackend {
        adapters: vec![SafeAdapter {
            vendor_id: 0x0a12,
            product_id: 0x0001,
        }],
        ..FakeBackend::default()
    };

    let record = execute(Command::Adapters, &mut backend).expect("list fake adapters");
    assert_eq!(record["event"], "adapters_listed");
    assert_eq!(record["adapter_count"], 1);
    assert_eq!(record["adapters"][0]["vendor_id"], 0x0a12);
    assert_eq!(record["adapters"][0]["product_id"], 0x0001);
    let text = record.to_string();
    for forbidden in ["selector", "serial", "bus", "port"] {
        assert!(
            !text.contains(forbidden),
            "record contains {forbidden}: {text}"
        );
    }
}

#[test]
fn fake_adapter_open_does_not_echo_the_selector() {
    let mut backend = FakeBackend::default();
    let selector = "usb:T06_SECRET_SELECTOR";

    let record =
        execute(Command::Open(selector.to_owned()), &mut backend).expect("open fake adapter");

    assert_eq!(backend.opened_selectors, [selector]);
    assert_eq!(record["event"], "adapter_opened");
    assert!(!record.to_string().contains(selector));
}

#[test]
fn adapter_open_success_requires_explicit_close_success() {
    let mut success = FakeController::default();
    assert_eq!(open_and_close(&mut success), Ok(()));
    assert_eq!(success.calls, ["open", "close"]);

    let mut close_failure = FakeController {
        close_result: Err(ErrorKind::WorkerFailed),
        ..FakeController::default()
    };
    assert_eq!(
        open_and_close(&mut close_failure),
        Err(ErrorKind::WorkerFailed)
    );
    assert_eq!(close_failure.calls, ["open", "close"]);
}

#[test]
fn typed_connection_dispatch_covers_all_models_and_reconnect_reporting_modes() {
    let mut backend = FakeBackend::default();
    for controller in [
        ControllerSelection::Pro,
        ControllerSelection::JoyConL,
        ControllerSelection::JoyConR,
    ] {
        execute(
            Command::Connection(connection_request(
                ConnectionOperation::Pair,
                controller,
                ReportingSelection::Periodic,
                None,
            )),
            &mut backend,
        )
        .expect("dispatch typed pair");
        for reporting in [ReportingSelection::Periodic, ReportingSelection::Direct] {
            execute(
                Command::Connection(connection_request(
                    ConnectionOperation::Reconnect,
                    controller,
                    reporting,
                    None,
                )),
                &mut backend,
            )
            .expect("dispatch typed reconnect");
        }
    }

    assert_eq!(
        backend.connections,
        [
            connection_call(
                ConnectionOperation::Pair,
                ControllerKind::Pro,
                ReportingKind::Periodic,
            ),
            connection_call(
                ConnectionOperation::Reconnect,
                ControllerKind::Pro,
                ReportingKind::Periodic,
            ),
            connection_call(
                ConnectionOperation::Reconnect,
                ControllerKind::Pro,
                ReportingKind::Direct,
            ),
            connection_call(
                ConnectionOperation::Pair,
                ControllerKind::JoyConL,
                ReportingKind::Periodic,
            ),
            connection_call(
                ConnectionOperation::Reconnect,
                ControllerKind::JoyConL,
                ReportingKind::Periodic,
            ),
            connection_call(
                ConnectionOperation::Reconnect,
                ControllerKind::JoyConL,
                ReportingKind::Direct,
            ),
            connection_call(
                ConnectionOperation::Pair,
                ControllerKind::JoyConR,
                ReportingKind::Periodic,
            ),
            connection_call(
                ConnectionOperation::Reconnect,
                ControllerKind::JoyConR,
                ReportingKind::Periodic,
            ),
            connection_call(
                ConnectionOperation::Reconnect,
                ControllerKind::JoyConR,
                ReportingKind::Direct,
            ),
        ]
    );
}

#[test]
fn fake_dynamic_button_rejects_model_mismatch_without_typed_fallback() {
    let mut backend = FakeBackend::default();
    let result = execute(
        Command::Connection(connection_request(
            ConnectionOperation::Pair,
            ControllerSelection::JoyConL,
            ReportingSelection::Periodic,
            Some(ButtonKind::A),
        )),
        &mut backend,
    );

    assert_eq!(result, Err(ErrorKind::UnsupportedInput));
    assert_eq!(
        backend.unsupported_buttons,
        [(ControllerKind::JoyConL, ButtonKind::A)]
    );
}

#[test]
fn imu_connection_completion_reports_only_safe_machine_evidence() {
    let request = connection_request(
        ConnectionOperation::Reconnect,
        ControllerSelection::Pro,
        ReportingSelection::Periodic,
        None,
    );
    let record = connection_completed_record(
        &request,
        super::ConnectionEvidence {
            imu: Some(ImuRunEvidence {
                duration_seconds: 60,
                apply_command_latency_ns: 12_345,
                non_neutral_reports_accepted: 7_500,
                neutral_reports_accepted: 1,
            }),
            shutdown_latency_ns: Some(98_765),
            neutral_close: true,
            profile_unchanged: Some(true),
            adapter_reopened: Some(true),
        },
    );

    assert_eq!(record["imu_run_seconds"], 60);
    assert_eq!(record["imu_apply_command_latency_ns"], 12_345);
    assert_eq!(record["imu_non_neutral_reports_accepted"], 7_500);
    assert_eq!(record["neutral_reports_accepted"], 1);
    assert_eq!(record["shutdown_latency_ns"], 98_765);
    assert_eq!(record["neutral_close"], true);
    assert_eq!(record["profile_unchanged"], true);
    assert_eq!(record["adapter_reopened"], true);
    let text = record.to_string();
    assert!(!text.contains("T07_SECRET_PROFILE"));
    assert!(!text.contains("T07_SECRET_TRACE"));
}

#[test]
fn hardware_imu_run_uses_project_demi_horizontal_yaw_pattern() {
    let frame = horizontal_yaw_frame().expect("horizontal yaw fixture");

    assert_eq!(frame.to_accel_g(), [0.0, 0.0, 1.0]);
    let [x, y, z] = frame.to_gyro_rate();
    assert_eq!([x, y], [0.0, 0.0]);
    assert!((z - 1.0).abs() <= ImuFrame::GYRO_DPS_PER_RAW.to_radians() / 2.0);
}

fn connection_request(
    operation: ConnectionOperation,
    controller: ControllerSelection,
    reporting: ReportingSelection,
    button: Option<ButtonKind>,
) -> ConnectionRequest {
    ConnectionRequest {
        operation,
        controller,
        reporting,
        profile: "T07_SECRET_PROFILE".into(),
        trace: "T07_SECRET_TRACE".into(),
        button,
        imu_duration: None,
    }
}

const fn connection_call(
    operation: ConnectionOperation,
    controller: ControllerKind,
    reporting: ReportingKind,
) -> ConnectionCall {
    ConnectionCall {
        operation,
        controller,
        reporting,
    }
}

#[derive(Default)]
struct FakeBackend {
    adapters: Vec<SafeAdapter>,
    opened_selectors: Vec<String>,
    connections: Vec<ConnectionCall>,
    unsupported_buttons: Vec<(ControllerKind, ButtonKind)>,
}

impl ProbeBackend for FakeBackend {
    fn list_adapters(&mut self) -> Result<Vec<SafeAdapter>, ErrorKind> {
        Ok(std::mem::take(&mut self.adapters))
    }

    fn open_adapter(&mut self, selector: &str) -> Result<(), ErrorKind> {
        self.opened_selectors.push(selector.to_owned());
        Ok(())
    }

    fn pair<M: ControllerModel>(
        &mut self,
        request: &ConnectionRequest,
    ) -> Result<super::ConnectionEvidence, ErrorKind> {
        self.record_connection::<M, swbt::reporting::Periodic>(
            ConnectionOperation::Pair,
            request.button,
        )?;
        Ok(super::ConnectionEvidence::default())
    }

    fn reconnect<M: ControllerModel, R: super::ProbeReporting<M>>(
        &mut self,
        request: &ConnectionRequest,
    ) -> Result<super::ConnectionEvidence, ErrorKind> {
        self.record_connection::<M, R>(ConnectionOperation::Reconnect, request.button)?;
        Ok(super::ConnectionEvidence::default())
    }
}

impl FakeBackend {
    fn record_connection<M: ControllerModel, R: ReportingMode>(
        &mut self,
        operation: ConnectionOperation,
        button: Option<ButtonKind>,
    ) -> Result<(), ErrorKind> {
        self.connections.push(ConnectionCall {
            operation,
            controller: M::KIND,
            reporting: R::KIND,
        });
        let Some(kind) = button else {
            return Ok(());
        };
        Button::<M>::try_from(kind).map(|_| ()).map_err(|error| {
            self.unsupported_buttons.push((M::KIND, kind));
            error.kind()
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ConnectionCall {
    operation: ConnectionOperation,
    controller: ControllerKind,
    reporting: ReportingKind,
}

struct FakeController {
    calls: Vec<&'static str>,
    open_result: Result<(), ErrorKind>,
    close_result: Result<(), ErrorKind>,
}

impl Default for FakeController {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            open_result: Ok(()),
            close_result: Ok(()),
        }
    }
}

impl ProbeController for FakeController {
    fn open(&mut self) -> Result<(), ErrorKind> {
        self.calls.push("open");
        self.open_result
    }

    fn close(&mut self) -> Result<(), ErrorKind> {
        self.calls.push("close");
        self.close_result
    }
}
