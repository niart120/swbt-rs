use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use bumble::keys::{Key, KeyStore, MemoryKeyStore, PairingKeys};
use bumble::{Address, AddressType, Uuid};
use bumble_controller::{Controller as LinkController, LocalLink};
use bumble_host::{Device, DeviceConfiguration, pump};
use bumble_l2cap::{ClassicChannelSpec, ClassicChannelState};
use bumble_sdp::{DataElement, SdpPdu};

use crate::diagnostics::LifecycleState;
use crate::input::{InputState, ProButton};
use crate::model::{ControllerModel, JoyConL, JoyConR, Pro};
use crate::protocol::SwitchHidProtocol;
use crate::reporting::{Direct, Periodic, ReportingMode};
use crate::runtime::cleanup::CloseMode;
use crate::runtime::status::{StatusReader, status_projection};
use crate::runtime::worker::{
    CommandSource, CommonCommand, MonotonicClock, PeriodicCommand, PriorityShutdown,
    RuntimeCommand, ShutdownRequest, WorkerBudget, WorkerCommandProgress, WorkerCore,
    WorkerReporting, WorkerStep,
};

use super::classic::{ClassicDeviceSession, HID_CONTROL_PSM, HID_INTERRUPT_PSM, SDP_PSM};
use super::{
    ActivityNotifier, HidChannel, SendAcceptance, TransportCapabilities, TransportConfig,
    TransportError, TransportErrorKind, TransportEvent, TransportPort, TransportResult,
    activity_channel,
};

const PEER_ADDRESS: &str = "11:11:11:11:11:11";
const SWBT_ADDRESS: &str = "22:22:22:22:22:22";
const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
const REPORT_PERIOD: Duration = Duration::from_millis(8);
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);
const STEP: Duration = Duration::from_millis(10);
const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];

#[test]
fn pro_periodic_reaches_ready_and_emits_typed_then_neutral_input() {
    let (transport, trace) = VirtualClassicTransport::new::<Pro>();
    let (status, reader) = status_projection();
    let protocol = SwitchHidProtocol::<Pro>::new(None, DEVICE_INFO_ADDRESS);
    let mut worker = WorkerCore::new_periodic_with_status(
        protocol,
        Box::new(transport),
        REPORT_PERIOD,
        WorkerBudget::new(2, 4),
        Box::new(|_| {}),
        status,
    )
    .expect("valid Periodic worker");
    let mut clock = ManualClock::default();
    let mut shutdown = ShutdownLatch::default();
    let mut commands = QueuedCommands::from([RuntimeCommand::<Pro, Periodic>::Pair {
        timeout: CONNECTION_TIMEOUT,
    }]);
    let mut pair_completed = false;

    for _ in 0..100 {
        let WorkerStep::Continue(mut progress) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("virtual pairing must keep the worker running");
        };
        for completion in progress.take_command_results() {
            match completion {
                WorkerCommandProgress::Pending => {}
                WorkerCommandProgress::Complete(Ok(())) => pair_completed = true,
                WorkerCommandProgress::Complete(Err(error)) => {
                    panic!("virtual pairing command failed: {error:?}")
                }
            }
        }
        if reader.status::<Periodic>().lifecycle == LifecycleState::Ready {
            break;
        }
        clock.advance(STEP);
    }

    let status = reader.status::<Periodic>();
    assert_eq!(status.lifecycle, LifecycleState::Ready);
    assert!(status.connected);
    assert_eq!(status.report_mode, Some(0x30));
    assert!(pair_completed);
    {
        let trace = lock(&trace);
        assert!(trace.stored_key_pairing_complete);
        assert!(trace.sdp_record_complete);
        assert!(trace.sdp_rounds > 1, "small SDP MTU must use continuation");
        assert!(
            trace
                .input_reports
                .iter()
                .any(|report| report.first() == Some(&0x30)),
            "peer receives bootstrap input through HIDP"
        );
        assert!(
            trace
                .input_reports
                .iter()
                .filter(|report| report.first() == Some(&0x21))
                .count()
                >= 2,
            "peer receives report-mode and player-light replies"
        );
    }

    let typed_start = lock(&trace).input_reports.len();
    commands.push(RuntimeCommand::Input(PeriodicCommand::Common(
        CommonCommand::Press(vec![ProButton::A]),
    )));
    for _ in 0..10 {
        clock.advance(REPORT_PERIOD);
        let WorkerStep::Continue(mut progress) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("typed input must keep the worker running");
        };
        assert_command_successes(&mut progress);
        if input_reports_after(&trace, typed_start)
            .iter()
            .any(|report| is_report_with_buttons(report, pro_a_buttons()))
        {
            break;
        }
    }
    assert!(
        input_reports_after(&trace, typed_start)
            .iter()
            .any(|report| is_report_with_buttons(report, pro_a_buttons())),
        "peer receives a Periodic 0x30 report with A pressed"
    );

    shutdown.request = Some(ShutdownRequest::explicit(CloseMode::WithNeutral));
    let WorkerStep::Closed {
        completion,
        interrupted,
        ..
    } = worker.step_runtime(&clock, &mut shutdown, &mut commands)
    else {
        panic!("explicit shutdown must close the virtual runtime");
    };
    assert!(completion.performed());
    assert!(interrupted.is_none());
    assert_eq!(
        reader.status::<Periodic>().lifecycle,
        LifecycleState::Closed
    );

    let trace = lock(&trace);
    let last_input = trace
        .input_reports
        .iter()
        .rev()
        .find(|report| report.first() == Some(&0x30))
        .expect("cleanup emits a final 0x30 report");
    assert!(is_report_with_buttons(last_input, neutral_buttons()));
    assert!(trace.disconnected);
    assert!(trace.closed);
}

#[test]
fn all_model_reporting_combinations_reach_ready_over_the_virtual_packet_path() {
    run_periodic_case::<Pro>();
    run_direct_case::<Pro>();
    run_periodic_case::<JoyConL>();
    run_direct_case::<JoyConL>();
    run_periodic_case::<JoyConR>();
    run_direct_case::<JoyConR>();
}

#[test]
fn reverse_channels_malformed_packets_and_stored_key_reconnect_remain_isolated() {
    run_resilience_case(VirtualScenario::resilient());
}

fn run_periodic_case<M: ControllerModel>() {
    let (transport, trace) = VirtualClassicTransport::new::<M>();
    let (status, reader) = status_projection();
    let mut worker = WorkerCore::new_periodic_with_status(
        SwitchHidProtocol::<M>::new(None, DEVICE_INFO_ADDRESS),
        Box::new(transport),
        REPORT_PERIOD,
        WorkerBudget::new(2, 4),
        Box::new(|_| {}),
        status,
    )
    .expect("valid Periodic worker");
    let mut commands = QueuedCommands::from([RuntimeCommand::<M, Periodic>::Pair {
        timeout: CONNECTION_TIMEOUT,
    }]);
    let mut clock = ManualClock::default();
    let mut shutdown = ShutdownLatch::default();
    drive_until_ready(
        &mut worker,
        &reader,
        &mut clock,
        &mut shutdown,
        &mut commands,
    );
    assert_common_virtual_trace(&trace, 1);
    close_ready_worker(&mut worker, &reader, &clock, &mut shutdown, &mut commands);
}

fn run_direct_case<M: ControllerModel>() {
    let (transport, trace) = VirtualClassicTransport::new::<M>();
    let (status, reader) = status_projection();
    let mut worker = WorkerCore::new_direct_with_status(
        SwitchHidProtocol::<M>::new(None, DEVICE_INFO_ADDRESS),
        Box::new(transport),
        WorkerBudget::new(2, 4),
        Box::new(|_| {}),
        status,
    );
    let mut commands = QueuedCommands::from([RuntimeCommand::<M, Direct>::Pair {
        timeout: CONNECTION_TIMEOUT,
    }]);
    let mut clock = ManualClock::default();
    let mut shutdown = ShutdownLatch::default();
    drive_until_ready(
        &mut worker,
        &reader,
        &mut clock,
        &mut shutdown,
        &mut commands,
    );
    assert_common_virtual_trace(&trace, 1);
    close_ready_worker(&mut worker, &reader, &clock, &mut shutdown, &mut commands);
}

fn run_resilience_case(scenario: VirtualScenario) {
    let (transport, trace) = VirtualClassicTransport::new_with_scenario::<Pro>(scenario);
    let (status, reader) = status_projection();
    let mut worker = WorkerCore::new_direct_with_status(
        SwitchHidProtocol::<Pro>::new(None, DEVICE_INFO_ADDRESS),
        Box::new(transport),
        WorkerBudget::new(2, 4),
        Box::new(|_| {}),
        status,
    );
    let mut commands = QueuedCommands::from([RuntimeCommand::<Pro, Direct>::Pair {
        timeout: CONNECTION_TIMEOUT,
    }]);
    let mut clock = ManualClock::default();
    let mut shutdown = ShutdownLatch::default();
    drive_until_ready(
        &mut worker,
        &reader,
        &mut clock,
        &mut shutdown,
        &mut commands,
    );
    {
        let trace = lock(&trace);
        assert_eq!(
            trace.hid_open_order.as_slice(),
            [HidChannel::Interrupt, HidChannel::Control]
        );
        assert!(trace.malformed_sdp_rejected);
        assert!(trace.malformed_hidp_rejected);
    }

    lock(&trace).request_peer_disconnect = true;
    for _ in 0..20 {
        let WorkerStep::Continue(mut progress) =
            worker.step_runtime(&clock, &mut shutdown, &mut commands)
        else {
            panic!("peer disconnect remains a recoverable worker event");
        };
        assert_command_successes(&mut progress);
        if reader.status::<Direct>().lifecycle == LifecycleState::Open {
            break;
        }
        clock.advance(STEP);
    }
    assert_eq!(reader.status::<Direct>().lifecycle, LifecycleState::Open);
    assert!(!reader.status::<Direct>().connected);

    commands.push(RuntimeCommand::Pair {
        timeout: CONNECTION_TIMEOUT,
    });
    drive_until_ready(
        &mut worker,
        &reader,
        &mut clock,
        &mut shutdown,
        &mut commands,
    );
    assert_common_virtual_trace(&trace, 2);
    {
        let trace = lock(&trace);
        assert_eq!(trace.peer_disconnects, 1);
        assert!(trace.sdp_completions >= 2);
        assert_eq!(
            trace.hid_open_order.as_slice(),
            [
                HidChannel::Interrupt,
                HidChannel::Control,
                HidChannel::Interrupt,
                HidChannel::Control,
            ]
        );
    }
    close_ready_worker(&mut worker, &reader, &clock, &mut shutdown, &mut commands);
}

fn drive_until_ready<M, R>(
    worker: &mut WorkerCore<M, R>,
    reader: &StatusReader<M>,
    clock: &mut ManualClock,
    shutdown: &mut ShutdownLatch,
    commands: &mut QueuedCommands<RuntimeCommand<M, R>>,
) where
    M: ControllerModel,
    R: ReportingMode + WorkerReporting<M>,
{
    let mut pair_completed = false;
    for _ in 0..100 {
        let WorkerStep::Continue(mut progress) = worker.step_runtime(clock, shutdown, commands)
        else {
            panic!("virtual pairing must keep the worker running");
        };
        for completion in progress.take_command_results() {
            match completion {
                WorkerCommandProgress::Pending => {}
                WorkerCommandProgress::Complete(Ok(())) => pair_completed = true,
                WorkerCommandProgress::Complete(Err(error)) => {
                    panic!("virtual pairing command failed: {error:?}")
                }
            }
        }
        if reader.status::<R>().lifecycle == LifecycleState::Ready {
            break;
        }
        clock.advance(STEP);
    }
    assert_eq!(reader.status::<R>().lifecycle, LifecycleState::Ready);
    assert!(reader.status::<R>().connected);
    assert_eq!(reader.status::<R>().report_mode, Some(0x30));
    assert!(pair_completed);
}

fn close_ready_worker<M, R>(
    worker: &mut WorkerCore<M, R>,
    reader: &StatusReader<M>,
    clock: &ManualClock,
    shutdown: &mut ShutdownLatch,
    commands: &mut QueuedCommands<RuntimeCommand<M, R>>,
) where
    M: ControllerModel,
    R: ReportingMode + WorkerReporting<M>,
{
    shutdown.request = Some(ShutdownRequest::explicit(CloseMode::WithNeutral));
    let WorkerStep::Closed {
        completion,
        interrupted,
        ..
    } = worker.step_runtime(clock, shutdown, commands)
    else {
        panic!("explicit shutdown must close the virtual worker");
    };
    assert!(completion.performed());
    assert!(interrupted.is_none());
    assert_eq!(reader.status::<R>().lifecycle, LifecycleState::Closed);
}

fn assert_common_virtual_trace(trace: &Arc<Mutex<VirtualTrace>>, expected_sessions: usize) {
    let trace = lock(trace);
    assert!(trace.stored_key_pairing_complete);
    assert!(trace.pairing_completions >= expected_sessions);
    assert!(trace.sdp_record_complete);
    assert!(trace.sdp_rounds > expected_sessions);
    assert!(
        trace
            .input_reports
            .iter()
            .filter(|report| report.first() == Some(&0x30))
            .count()
            >= expected_sessions
    );
    assert!(
        trace
            .input_reports
            .iter()
            .filter(|report| report.first() == Some(&0x21))
            .count()
            >= expected_sessions * 2
    );
}

#[derive(Clone, Copy, Default)]
struct VirtualScenario {
    reverse_hid_channels: bool,
    malformed_sdp: bool,
    malformed_hidp: bool,
}

impl VirtualScenario {
    const fn resilient() -> Self {
        Self {
            reverse_hid_channels: true,
            malformed_sdp: true,
            malformed_hidp: true,
        }
    }
}

#[derive(Default)]
struct VirtualTrace {
    stored_key_pairing_complete: bool,
    pairing_completions: usize,
    sdp_rounds: usize,
    sdp_completions: usize,
    sdp_record_complete: bool,
    hid_open_order: Vec<HidChannel>,
    malformed_sdp_rejected: bool,
    malformed_hidp_rejected: bool,
    input_reports: Vec<Vec<u8>>,
    request_peer_disconnect: bool,
    peer_disconnects: usize,
    disconnected: bool,
    closed: bool,
}

struct VirtualClassicTransport {
    link: LocalLink,
    devices: [Device; 2],
    session: ClassicDeviceSession,
    peer_address: Address,
    swbt_address: Address,
    scenario: VirtualScenario,
    pairing_started: bool,
    pairing_recorded: bool,
    connection_requested: bool,
    encryption_started: bool,
    ctkd_started: bool,
    sdp_cid: Option<u16>,
    sdp_continuation: Vec<u8>,
    sdp_request_in_flight: bool,
    malformed_sdp_sent: bool,
    sdp_attributes: Vec<u8>,
    sdp_complete: bool,
    control_cid: Option<u16>,
    interrupt_cid: Option<u16>,
    peer_outputs_sent: bool,
    trace: Arc<Mutex<VirtualTrace>>,
    closed: bool,
}

impl VirtualClassicTransport {
    fn new<M: ControllerModel>() -> (Self, Arc<Mutex<VirtualTrace>>) {
        Self::new_with_scenario::<M>(VirtualScenario::default())
    }

    fn new_with_scenario<M: ControllerModel>(
        scenario: VirtualScenario,
    ) -> (Self, Arc<Mutex<VirtualTrace>>) {
        let peer_address = public_address(PEER_ADDRESS);
        let swbt_address = public_address(SWBT_ADDRESS);
        let mut link = LocalLink::new();
        let peer_id = link.add_controller(LinkController::new("peer", peer_address.clone()));
        let swbt_id = link.add_controller(LinkController::new("swbt", swbt_address.clone()));
        let peer_config = DeviceConfiguration {
            classic_enabled: true,
            classic_smp_enabled: true,
            classic_accept_any: true,
            ..DeviceConfiguration::default()
        };
        let swbt_config = DeviceConfiguration {
            classic_enabled: true,
            classic_smp_enabled: true,
            classic_accept_any: false,
            connectable: false,
            discoverable: false,
            ..DeviceConfiguration::default()
        };
        let mut devices = [
            Device::from_config(peer_id, peer_config).expect("configured virtual peer"),
            Device::from_config(swbt_id, swbt_config).expect("configured virtual swbt device"),
        ];
        let link_key = [0xC7; 16];
        devices[0].set_key_store(stored_link_key(&swbt_address, link_key));
        devices[1].set_key_store(stored_link_key(&peer_address, link_key));
        devices[0].power_on(&mut link).expect("power virtual peer");
        devices[1]
            .power_on(&mut link)
            .expect("power virtual swbt device");
        pump(&mut link, &mut devices);

        let config = TransportConfig::for_model::<M>();
        let (activity, _wakes) = activity_channel();
        let mut session = ClassicDeviceSession::new(&config, activity);
        session
            .register_servers(&mut devices[1])
            .expect("register virtual SDP and HID servers");
        let trace = Arc::new(Mutex::new(VirtualTrace::default()));
        (
            Self {
                link,
                devices,
                session,
                peer_address,
                swbt_address,
                scenario,
                pairing_started: false,
                pairing_recorded: false,
                connection_requested: false,
                encryption_started: false,
                ctkd_started: false,
                sdp_cid: None,
                sdp_continuation: vec![0],
                sdp_request_in_flight: false,
                malformed_sdp_sent: false,
                sdp_attributes: Vec::new(),
                sdp_complete: false,
                control_cid: None,
                interrupt_cid: None,
                peer_outputs_sent: false,
                trace: Arc::clone(&trace),
                closed: false,
            },
            trace,
        )
    }

    fn drive(&mut self) -> TransportResult<Vec<TransportEvent>> {
        let mut events = Vec::new();
        for _ in 0..64 {
            self.maybe_disconnect_peer();
            pump(&mut self.link, &mut self.devices);
            let polled = self.session.poll(&mut self.devices[1], &mut self.link)?;
            for event in &polled {
                match event {
                    TransportEvent::HidChannelOpened { channel } => {
                        lock(&self.trace).hid_open_order.push(*channel);
                    }
                    TransportEvent::Disconnected { .. } => self.reset_connection_state(),
                    TransportEvent::Connected | TransportEvent::HidOutput { .. } => {}
                }
            }
            events.extend(polled);
            pump(&mut self.link, &mut self.devices);
            self.advance_peer();
            self.collect_peer_input();
            self.collect_peer_control();
        }
        Ok(events)
    }

    fn maybe_disconnect_peer(&mut self) {
        let requested = {
            let mut trace = lock(&self.trace);
            let requested = trace.request_peer_disconnect;
            trace.request_peer_disconnect = false;
            requested
        };
        if !requested {
            return;
        }
        let Some(handle) = self.devices[0].classic_connection_handle() else {
            return;
        };
        assert!(self.devices[0].disconnect_handle(&mut self.link, handle, 0x13));
        lock(&self.trace).peer_disconnects += 1;
    }

    fn reset_connection_state(&mut self) {
        self.pairing_started = false;
        self.pairing_recorded = false;
        self.connection_requested = false;
        self.encryption_started = false;
        self.ctkd_started = false;
        self.sdp_cid = None;
        self.sdp_continuation = vec![0];
        self.sdp_request_in_flight = false;
        self.malformed_sdp_sent = false;
        self.sdp_attributes.clear();
        self.sdp_complete = false;
        self.control_cid = None;
        self.interrupt_cid = None;
        self.peer_outputs_sent = false;
    }

    fn advance_peer(&mut self) {
        if self.pairing_started && !self.connection_requested {
            self.devices[0].connect_classic(&mut self.link, self.swbt_address.clone());
            self.connection_requested = true;
            return;
        }
        let Some(peer_handle) = self.devices[0].classic_connection_handle() else {
            return;
        };
        if self.devices[1].classic_connection_handle().is_none() {
            return;
        }
        if !self.encryption_started {
            assert!(self.devices[0].set_classic_encryption(&mut self.link, true));
            self.encryption_started = true;
            return;
        }
        if !self.devices[0].is_classic_encrypted() || !self.devices[1].is_classic_encrypted() {
            return;
        }
        if !self.ctkd_started {
            self.devices[0]
                .pair_classic(&mut self.link)
                .expect("start virtual BR/EDR CTKD");
            self.ctkd_started = true;
            return;
        }
        if !self.stored_key_pairing_complete() {
            return;
        }
        if !self.pairing_recorded {
            let mut trace = lock(&self.trace);
            trace.stored_key_pairing_complete = true;
            trace.pairing_completions += 1;
            self.pairing_recorded = true;
        }

        if self.sdp_cid.is_none() {
            self.sdp_cid = Some(
                self.devices[0]
                    .connect_classic_channel(
                        &mut self.link,
                        peer_handle,
                        SDP_PSM,
                        ClassicChannelSpec { mtu: 48 },
                    )
                    .expect("connect virtual SDP channel"),
            );
            return;
        }
        self.advance_sdp(peer_handle);
        if !self.sdp_complete {
            return;
        }
        let order = if self.scenario.reverse_hid_channels {
            [HID_INTERRUPT_PSM, HID_CONTROL_PSM]
        } else {
            [HID_CONTROL_PSM, HID_INTERRUPT_PSM]
        };
        for psm in order {
            let missing = match psm {
                HID_CONTROL_PSM => self.control_cid.is_none(),
                HID_INTERRUPT_PSM => self.interrupt_cid.is_none(),
                _ => false,
            };
            if !missing {
                continue;
            }
            let cid = self.devices[0]
                .connect_classic_channel(
                    &mut self.link,
                    peer_handle,
                    psm,
                    ClassicChannelSpec { mtu: 96 },
                )
                .expect("connect virtual HID channel");
            match psm {
                HID_CONTROL_PSM => self.control_cid = Some(cid),
                HID_INTERRUPT_PSM => self.interrupt_cid = Some(cid),
                _ => unreachable!("virtual HID order contains known PSMs"),
            }
            return;
        }
    }

    fn stored_key_pairing_complete(&mut self) -> bool {
        let (Some(peer_handle), Some(swbt_handle)) = (
            self.devices[0].classic_connection_handle(),
            self.devices[1].classic_connection_handle(),
        ) else {
            return false;
        };
        let current_pairing_complete = [
            self.devices[0].pairing_keys(peer_handle),
            self.devices[1].pairing_keys(swbt_handle),
        ]
        .into_iter()
        .all(|keys| keys.is_some_and(|keys| keys.link_key.is_some() && keys.ltk.is_some()));
        if !current_pairing_complete {
            return false;
        }
        let peer_bond = self.devices[0]
            .bond(&self.swbt_address)
            .expect("read peer bond");
        let swbt_bond = self.devices[1]
            .bond(&self.peer_address)
            .expect("read swbt bond");
        [peer_bond, swbt_bond]
            .into_iter()
            .all(|bond| bond.is_some_and(|keys| keys.link_key.is_some() && keys.ltk.is_some()))
    }

    fn advance_sdp(&mut self, peer_handle: u16) {
        let sdp_cid = self.sdp_cid.expect("SDP CID exists");
        let channel_open = self.devices[0]
            .classic_channel(peer_handle, sdp_cid)
            .is_some_and(|channel| channel.state == ClassicChannelState::Open);
        if !channel_open {
            return;
        }
        let responses = self.devices[0].take_classic_channel_sdus(peer_handle, sdp_cid);
        for response in responses {
            self.sdp_request_in_flight = false;
            match SdpPdu::from_bytes(&response).expect("parse virtual SDP response") {
                SdpPdu::ErrorResponse { .. }
                    if self.scenario.malformed_sdp && self.malformed_sdp_sent =>
                {
                    lock(&self.trace).malformed_sdp_rejected = true;
                }
                SdpPdu::ServiceSearchAttributeResponse {
                    attribute_lists,
                    continuation_state,
                    ..
                } => {
                    self.sdp_attributes.extend_from_slice(&attribute_lists);
                    self.sdp_continuation = continuation_state;
                    lock(&self.trace).sdp_rounds += 1;
                    if self.sdp_continuation == [0] {
                        self.sdp_complete = true;
                        let complete_record = matches!(
                            DataElement::from_bytes(&self.sdp_attributes),
                            Ok(DataElement::Sequence(records)) if !records.is_empty()
                        );
                        let mut trace = lock(&self.trace);
                        trace.sdp_record_complete = complete_record;
                        trace.sdp_completions += 1;
                    }
                }
                _ => panic!("virtual peer received an unexpected SDP response"),
            }
        }
        if !self.sdp_complete && !self.sdp_request_in_flight {
            if self.scenario.malformed_sdp && !self.malformed_sdp_sent {
                self.devices[0]
                    .send_classic_channel_sdu(
                        &mut self.link,
                        peer_handle,
                        sdp_cid,
                        &[0x06, 0x00, 0x01, 0x00, 0x02, 0x00],
                    )
                    .expect("send malformed virtual SDP request");
                self.malformed_sdp_sent = true;
                self.sdp_request_in_flight = true;
                return;
            }
            let transaction_id =
                u16::try_from(lock(&self.trace).sdp_rounds + 1).expect("SDP rounds fit u16");
            let request = SdpPdu::ServiceSearchAttributeRequest {
                transaction_id,
                service_search_pattern: DataElement::sequence([DataElement::uuid(
                    Uuid::from_16_bits(0x1124),
                )]),
                maximum_attribute_byte_count: 32,
                attribute_id_list: DataElement::sequence([DataElement::unsigned_integer_32(
                    0x0000_FFFF,
                )]),
                continuation_state: self.sdp_continuation.clone(),
            }
            .to_bytes()
            .expect("encode virtual SDP request");
            self.devices[0]
                .send_classic_channel_sdu(&mut self.link, peer_handle, sdp_cid, &request)
                .expect("send virtual SDP request");
            self.sdp_request_in_flight = true;
        }
    }

    fn collect_peer_input(&mut self) {
        let (Some(peer_handle), Some(interrupt_cid)) = (
            self.devices[0].classic_connection_handle(),
            self.interrupt_cid,
        ) else {
            return;
        };
        let reports = self.devices[0].take_classic_channel_sdus(peer_handle, interrupt_cid);
        let mut send_outputs = false;
        for report in reports {
            let Some((&0xA1, input)) = report.split_first() else {
                continue;
            };
            lock(&self.trace).input_reports.push(input.to_vec());
            if !self.peer_outputs_sent && input.first() == Some(&0x30) {
                send_outputs = true;
            }
        }
        if send_outputs {
            self.send_peer_outputs(peer_handle);
            self.peer_outputs_sent = true;
        }
    }

    fn collect_peer_control(&mut self) {
        let (Some(peer_handle), Some(control_cid)) = (
            self.devices[0].classic_connection_handle(),
            self.control_cid,
        ) else {
            return;
        };
        let responses = self.devices[0].take_classic_channel_sdus(peer_handle, control_cid);
        if responses
            .iter()
            .any(|response| response.as_slice() == [0x04])
        {
            lock(&self.trace).malformed_hidp_rejected = true;
        }
    }

    fn send_peer_outputs(&mut self, peer_handle: u16) {
        let control_cid = self.control_cid.expect("control channel is connected");
        let interrupt_cid = self.interrupt_cid.expect("interrupt channel is connected");
        if self.scenario.malformed_hidp {
            self.devices[0]
                .send_classic_channel_sdu(&mut self.link, peer_handle, control_cid, &[0x41])
                .expect("send malformed virtual HIDP control message");
        }
        let report_mode = hid_output(subcommand_report(0x03, &[0x30]));
        self.devices[0]
            .send_classic_channel_sdu(&mut self.link, peer_handle, control_cid, &report_mode)
            .expect("send report-mode output");
        let player_lights = hid_output(subcommand_report(0x30, &[0x01]));
        self.devices[0]
            .send_classic_channel_sdu(&mut self.link, peer_handle, interrupt_cid, &player_lights)
            .expect("send player-lights output");
    }
}

impl TransportPort for VirtualClassicTransport {
    fn open(&mut self, _activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        if self.closed {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        Ok(TransportCapabilities::test_default())
    }

    fn start_pairing(&mut self) -> TransportResult<()> {
        if self.closed {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        if !self.pairing_started {
            self.reset_connection_state();
        }
        self.session
            .start_pairing(&mut self.devices[1], &mut self.link)?;
        self.pairing_started = true;
        Ok(())
    }

    fn poll(&mut self, _timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        if self.closed {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        self.drive()
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        if self.closed {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        let acceptance =
            self.session
                .send_interrupt(&mut self.devices[1], &mut self.link, payload)?;
        pump(&mut self.link, &mut self.devices);
        self.collect_peer_input();
        pump(&mut self.link, &mut self.devices);
        self.collect_peer_control();
        Ok(acceptance)
    }

    fn drain_interrupt(&mut self, _timeout: Duration) -> TransportResult<()> {
        if self.closed {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        pump(&mut self.link, &mut self.devices);
        if self.session.interrupt_output_is_drained(&self.devices[1]) {
            Ok(())
        } else {
            Err(TransportError::new(TransportErrorKind::DrainTimedOut))
        }
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        if self.closed {
            return Ok(());
        }
        self.session
            .disconnect(&mut self.devices[1], &mut self.link)?;
        pump(&mut self.link, &mut self.devices);
        lock(&self.trace).disconnected = true;
        Ok(())
    }

    fn close(&mut self) -> TransportResult<()> {
        self.closed = true;
        lock(&self.trace).closed = true;
        Ok(())
    }
}

#[derive(Default)]
struct ManualClock {
    now: Duration,
}

impl ManualClock {
    fn advance(&mut self, amount: Duration) {
        self.now += amount;
    }
}

impl MonotonicClock for ManualClock {
    fn now(&self) -> Duration {
        self.now
    }
}

#[derive(Default)]
struct ShutdownLatch {
    request: Option<ShutdownRequest>,
}

impl PriorityShutdown for ShutdownLatch {
    fn take(&mut self) -> Option<ShutdownRequest> {
        self.request.take()
    }
}

struct QueuedCommands<C> {
    commands: VecDeque<C>,
}

impl<C> QueuedCommands<C> {
    fn from(commands: impl IntoIterator<Item = C>) -> Self {
        Self {
            commands: commands.into_iter().collect(),
        }
    }

    fn push(&mut self, command: C) {
        self.commands.push_back(command);
    }
}

impl<C> CommandSource<C> for QueuedCommands<C> {
    fn try_next(&mut self) -> Option<C> {
        self.commands.pop_front()
    }
}

fn stored_link_key(peer_address: &Address, value: [u8; 16]) -> MemoryKeyStore {
    let mut store = MemoryKeyStore::new();
    store
        .update(
            &peer_address.to_string(false),
            PairingKeys {
                link_key: Some(Key {
                    value: value.to_vec(),
                    authenticated: true,
                    ..Key::default()
                }),
                link_key_type: Some(0x08),
                ..PairingKeys::default()
            },
        )
        .expect("store virtual Classic link key");
    store
}

fn public_address(value: &str) -> Address {
    Address::parse(value, AddressType::PUBLIC_DEVICE).expect("valid virtual public address")
}

fn hid_output(report: Vec<u8>) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(report.len() + 1);
    encoded.push(0xA2);
    encoded.extend_from_slice(&report);
    encoded
}

fn subcommand_report(subcommand_id: u8, payload: &[u8]) -> Vec<u8> {
    let mut report = vec![0x01, 0];
    report.extend_from_slice(&NEUTRAL_RUMBLE);
    report.push(subcommand_id);
    report.extend_from_slice(payload);
    report
}

fn pro_a_buttons() -> [u8; 3] {
    let state = InputState::<Pro>::neutral().with_buttons([ProButton::A]);
    input_button_bytes(&state)
}

fn neutral_buttons() -> [u8; 3] {
    input_button_bytes(&InputState::<Pro>::neutral())
}

fn input_button_bytes(state: &InputState<Pro>) -> [u8; 3] {
    SwitchHidProtocol::<Pro>::new(None, DEVICE_INFO_ADDRESS)
        .prepare_input_report(state, 0, Default::default(), 0)
        .bytes()[3..6]
        .try_into()
        .expect("input report has three button bytes")
}

fn is_report_with_buttons(report: &[u8], buttons: [u8; 3]) -> bool {
    report.first() == Some(&0x30) && report.get(3..6) == Some(buttons.as_slice())
}

fn input_reports_after(trace: &Arc<Mutex<VirtualTrace>>, start: usize) -> Vec<Vec<u8>> {
    lock(trace).input_reports[start..].to_vec()
}

fn assert_command_successes(progress: &mut crate::runtime::worker::StepProgress) {
    for completion in progress.take_command_results() {
        match completion {
            WorkerCommandProgress::Complete(Ok(())) | WorkerCommandProgress::Pending => {}
            WorkerCommandProgress::Complete(Err(error)) => {
                panic!("virtual command failed: {error:?}")
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
