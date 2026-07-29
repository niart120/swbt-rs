use std::collections::{BTreeMap, VecDeque};
use std::error::Error as _;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use bumble_hci::{Address, AddressType, Command, Event, HciPacket, ReturnParameters};
use bumble_host::{HOST_EVENT_MASK, HOST_LE_EVENT_MASK};
use bumble_transport::{
    Error as BumbleError, PacketSink, PacketSource, PacketSourceShutdown, Result as BumbleResult,
    SplitOpenedTransport,
};

use crate::adapter::AdapterSelector;
use crate::model::Pro;

use super::bumble::{
    BumbleSession, BumbleTransportPort, SplitTransportOpener, initialize_bumble_session_with,
};
use super::{TransportConfig, TransportErrorKind, TransportPort, activity_channel};

const DISPLAY_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];
const HCI_ADDRESS: [u8; 6] = [0x7d, 0x9f, 0xf9, 0xdc, 0x1b, 0x00];

#[cfg(feature = "adapter-tests")]
#[test]
#[ignore = "claims and initializes the CSR8510 A10 target adapter"]
fn target_adapter_reports_initialized_identity_version_and_classic_capability() {
    let config = TransportConfig::for_model::<Pro>();
    let mut transport = BumbleTransportPort::new(AdapterSelector::from("usb:0a12:0001"), config);

    let capabilities = transport
        .open(activity_channel().0)
        .expect("open and initialize target adapter");

    let local_address = capabilities.local_address();
    let version = capabilities
        .local_version()
        .expect("target adapter reports HCI/LMP version metadata");
    eprintln!(
        "local_address={:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X} \
         hci_version=0x{:02X} hci_subversion=0x{:04X} \
         lmp_version=0x{:02X} company_identifier=0x{:04X} lmp_subversion=0x{:04X}",
        local_address[0],
        local_address[1],
        local_address[2],
        local_address[3],
        local_address[4],
        local_address[5],
        version.hci_version(),
        version.hci_subversion(),
        version.lmp_version(),
        version.company_identifier(),
        version.lmp_subversion(),
    );

    assert_ne!(local_address, [0; 6]);
    assert!(capabilities.classic_capable());
    assert_eq!(capabilities.usb().vendor_id(), 0x0a12);
    assert_eq!(capabilities.usb().product_id(), 0x0001);
    transport
        .close()
        .expect("stop reader and release target adapter");
}

#[test]
fn bumble_initialization_uses_configured_device_and_exact_hci_order() {
    let config = TransportConfig::for_model::<Pro>();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let (transport, _drops) = scripted_transport(
        successful_initialization_responses(&config),
        Arc::clone(&commands),
        None,
    );
    let selectors = Arc::new(Mutex::new(Vec::new()));
    let mut opener = ScriptedOpener::new(transport, Arc::clone(&selectors));

    let session = initialize_bumble_session_with(
        &mut opener,
        &AdapterSelector::from("usb:0A12:0001"),
        &config,
        activity_channel().0,
    )
    .expect("scripted Bumble initialization");

    assert_eq!(lock(&selectors).as_slice(), ["usb:0A12:0001"]);
    let expected = expected_commands(&config);
    assert_eq!(lock(&commands).as_slice(), expected.as_slice());
    let device = session.device_configuration();
    assert_eq!(device.name, config.local_name());
    assert_eq!(device.class_of_device, config.class_of_device());
    assert_eq!(device.advertising_data, config.complete_local_name_ad());
    assert_eq!(device.classic_enabled, config.classic_enabled());
    assert_eq!(device.classic_accept_any, config.classic_accept_any());
    assert_eq!(device.connectable, config.connectable());
    assert_eq!(device.discoverable, config.discoverable());
    assert_eq!(device.classic_sc_enabled, config.classic_sc_enabled());
    assert_eq!(device.classic_ssp_enabled, config.classic_ssp_enabled());
    assert_eq!(device.le_enabled, config.le_enabled());
    assert_eq!(
        device.le_simultaneous_enabled,
        config.le_simultaneous_enabled()
    );

    let capabilities = session.capabilities();
    assert_eq!(capabilities.local_address(), DISPLAY_ADDRESS);
    assert!(capabilities.classic_capable());
    let version = capabilities.local_version().expect("version metadata");
    assert_eq!(version.hci_version(), 0x09);
    assert_eq!(version.hci_subversion(), 0x1234);
    assert_eq!(version.lmp_version(), 0x09);
    assert_eq!(version.company_identifier(), 0x000a);
    assert_eq!(version.lmp_subversion(), 0x5678);
    let usb = capabilities.usb();
    assert_eq!(usb.vendor_id(), 0x0a12);
    assert_eq!(usb.product_id(), 0x0001);
    assert_eq!(usb.bus_number(), 1);
    assert_eq!(usb.device_address(), 7);
}

#[test]
fn bumble_initialization_rejects_incomplete_identity_response() {
    let config = TransportConfig::for_model::<Pro>();
    for identity_response in [
        HciPacket::Event(Event::CommandStatus {
            status: 0,
            num_hci_command_packets: 1,
            command_opcode: Command::ReadBdAddr.op_code(),
        }),
        command_complete(Command::ReadBdAddr, ReturnParameters::Raw { data: vec![0] }),
        command_complete(
            Command::ReadBdAddr,
            ReturnParameters::ReadBdAddr {
                status: 1,
                bd_addr: Address::from_bytes(HCI_ADDRESS, AddressType::PUBLIC_DEVICE),
            },
        ),
    ] {
        let mut responses = controller_initialization_responses();
        responses.push(identity_response);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let (transport, _drops) = scripted_transport(responses, commands, None);
        let selectors = Arc::new(Mutex::new(Vec::new()));
        let mut opener = ScriptedOpener::new(transport, selectors);

        let error = match initialize_bumble_session_with(
            &mut opener,
            &AdapterSelector::from("usb:0"),
            &config,
            activity_channel().0,
        ) {
            Ok(_) => panic!("ReadBdAddr requires exact successful Command Complete"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), TransportErrorKind::OpenFailed);
        assert!(
            error.source().is_some(),
            "ReadBdAddr failure retains a typed initialization source"
        );
        assert!(!error.to_string().contains("ReadBdAddr"));
        assert!(!format!("{error:?}").contains("ReadBdAddr"));
    }
}

#[test]
fn bumble_initialization_rejects_failed_reset_with_typed_source() {
    let config = TransportConfig::for_model::<Pro>();
    let mut responses = controller_initialization_responses();
    responses[0] = command_complete(Command::Reset, ReturnParameters::Status { status: 1 });
    let commands = Arc::new(Mutex::new(Vec::new()));
    let (transport, _drops) = scripted_transport(responses, commands, None);
    let selectors = Arc::new(Mutex::new(Vec::new()));
    let mut opener = ScriptedOpener::new(transport, selectors);

    let error = match initialize_bumble_session_with(
        &mut opener,
        &AdapterSelector::from("usb:0"),
        &config,
        activity_channel().0,
    ) {
        Ok(_) => panic!("failed HCI Reset must stop initialization"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), TransportErrorKind::OpenFailed);
    assert!(
        matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<BumbleError>()),
            Some(BumbleError::Remote(_))
        ),
        "failed Reset retains the typed Bumble source"
    );
    assert!(!error.to_string().contains("Reset"));
    assert!(!format!("{error:?}").contains("Reset"));
}

#[test]
fn bumble_initialization_requires_successful_complete_identity_writes() {
    let config = TransportConfig::for_model::<Pro>();
    let command = identity_commands(&config)[0].clone();
    for response in [
        HciPacket::Event(Event::CommandStatus {
            status: 0,
            num_hci_command_packets: 1,
            command_opcode: command.op_code(),
        }),
        command_complete(command.clone(), ReturnParameters::Raw { data: vec![1] }),
    ] {
        let mut responses = successful_initialization_responses(&config);
        responses[8] = response;
        let commands = Arc::new(Mutex::new(Vec::new()));
        let (transport, _drops) = scripted_transport(responses, commands, None);
        let selectors = Arc::new(Mutex::new(Vec::new()));
        let mut opener = ScriptedOpener::new(transport, selectors);

        let error = match initialize_bumble_session_with(
            &mut opener,
            &AdapterSelector::from("usb:0"),
            &config,
            activity_channel().0,
        ) {
            Ok(_) => panic!("identity writes require successful Command Complete"),
            Err(error) => error,
        };

        assert_eq!(error.kind(), TransportErrorKind::OpenFailed);
    }
}

#[test]
fn bumble_initialization_failure_preserves_source_and_releases_scripted_resources() {
    let config = TransportConfig::for_model::<Pro>();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let fail_opcode = Command::WriteExtendedInquiryResponse {
        fec_required: 0,
        extended_inquiry_response: [0; 240],
    }
    .op_code();
    let (transport, drops) = scripted_transport(
        successful_initialization_responses(&config),
        commands,
        Some(fail_opcode),
    );
    let selectors = Arc::new(Mutex::new(Vec::new()));
    let mut opener = ScriptedOpener::new(transport, selectors);

    let error = match initialize_bumble_session_with(
        &mut opener,
        &AdapterSelector::from("usb:0"),
        &config,
        activity_channel().0,
    ) {
        Ok(_) => panic!("scripted identity write failure"),
        Err(error) => error,
    };

    assert_eq!(error.kind(), TransportErrorKind::OpenFailed);
    assert!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<BumbleError>())
            .is_some()
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!format!("{error:?}").contains("secret"));
    let mut released = [
        drops
            .recv_timeout(Duration::from_secs(1))
            .expect("scripted source or sink is released"),
        drops
            .recv_timeout(Duration::from_secs(1))
            .expect("both scripted transport halves are released"),
    ];
    released.sort_unstable();
    assert_eq!(released, ["sink", "source"]);
}

#[test]
fn bumble_reader_enqueues_before_wake_and_coalesces_activity() {
    let (mut session, source, wakes, _drops) = controlled_session();

    wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("initialization activity");
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));

    source.push(Ok(Some(command_complete(
        Command::Reset,
        ReturnParameters::Status { status: 0 },
    ))));
    wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("reader activity after enqueue");
    session
        .poll(Duration::ZERO)
        .expect("zero-time poll observes enqueued packet");
    assert_eq!(wakes.try_recv(), Err(TryRecvError::Empty));

    session.close().expect("reader cleanup");
}

#[test]
fn bumble_reader_end_and_failure_are_single_wake_and_sticky() {
    let (mut ended, end_source, end_wakes, _drops) = controlled_session();
    end_wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("initialization activity");
    end_source.finish();
    end_wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("clean end activity");
    assert!(matches!(
        end_wakes.try_recv(),
        Err(TryRecvError::Empty | TryRecvError::Disconnected)
    ));

    let first = ended
        .poll(Duration::ZERO)
        .expect_err("clean end is terminal");
    assert_eq!(first.kind(), TransportErrorKind::SourceTerminated);
    let repeated = ended
        .poll(Duration::ZERO)
        .expect_err("clean end remains terminal");
    assert_eq!(repeated.kind(), TransportErrorKind::SourceTerminated);
    ended.close().expect("clean end remains closable");

    let (failed, fail_source, fail_wakes, _drops) = controlled_session();
    let mut failed = BumbleTransportPort::from_session_for_test(failed);
    fail_wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("initialization activity");
    fail_source.push(Err(std::io::Error::other(
        "PAIRING-KEY reader failure sentinel",
    )
    .into()));
    fail_wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("failure activity");
    assert!(matches!(
        fail_wakes.try_recv(),
        Err(TryRecvError::Empty | TryRecvError::Disconnected)
    ));

    let first = failed
        .poll(Duration::ZERO)
        .expect_err("reader failure is terminal");
    assert_eq!(first.kind(), TransportErrorKind::SourceTerminated);
    let source = first.source().expect("typed Bumble source");
    let bumble = source
        .downcast_ref::<BumbleError>()
        .expect("Bumble error remains in the source chain");
    assert!(matches!(
        bumble,
        BumbleError::ExternalHostFailure(error)
            if matches!(error.as_ref(), BumbleError::Io(_))
    ));
    assert!(!first.to_string().contains("PAIRING-KEY"));
    assert!(!format!("{first:?}").contains("PAIRING-KEY"));

    let repeated = failed
        .poll(Duration::ZERO)
        .expect_err("reader failure remains terminal");
    assert_eq!(repeated.kind(), TransportErrorKind::SourceTerminated);
    assert!(std::ptr::eq(
        first.source().expect("first source"),
        repeated.source().expect("sticky source"),
    ));
    for terminal in [
        failed
            .send_interrupt(&[])
            .expect_err("send retains the reader terminal"),
        failed
            .drain_interrupt(Duration::ZERO)
            .expect_err("drain retains the reader terminal"),
        failed
            .disconnect()
            .expect_err("disconnect retains the reader terminal"),
    ] {
        assert_eq!(terminal.kind(), TransportErrorKind::SourceTerminated);
        assert!(std::ptr::eq(
            first.source().expect("first source"),
            terminal.source().expect("port preserves the sticky source"),
        ));
    }
    failed.close().expect("failed reader remains closable");
}

#[test]
fn bumble_close_cancels_reader_waits_for_completion_and_joins_once() {
    let (mut session, _source, wakes, drops) = controlled_session();
    wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("initialization activity");

    session.close().expect("first close");
    wakes
        .recv_timeout(Duration::from_secs(1))
        .expect("shutdown terminal activity");
    assert!(matches!(
        wakes.try_recv(),
        Err(TryRecvError::Empty | TryRecvError::Disconnected)
    ));
    let mut released = [
        drops
            .recv_timeout(Duration::from_secs(1))
            .expect("source released after reader join"),
        drops
            .recv_timeout(Duration::from_secs(1))
            .expect("sink released after reader join"),
    ];
    released.sort_unstable();
    assert_eq!(released, ["sink", "source"]);
    assert_eq!(
        session
            .poll(Duration::ZERO)
            .expect_err("closed session rejects poll")
            .kind(),
        TransportErrorKind::Closed
    );

    session.close().expect("repeated close");
    assert!(matches!(
        drops.try_recv(),
        Err(TryRecvError::Empty | TryRecvError::Disconnected)
    ));
}

struct ScriptedOpener {
    transport: Option<SplitOpenedTransport>,
    selectors: Arc<Mutex<Vec<String>>>,
}

impl ScriptedOpener {
    fn new(transport: SplitOpenedTransport, selectors: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            transport: Some(transport),
            selectors,
        }
    }
}

impl SplitTransportOpener for ScriptedOpener {
    fn open_split(&mut self, selector: &str) -> BumbleResult<SplitOpenedTransport> {
        lock(&self.selectors).push(selector.to_owned());
        self.transport
            .take()
            .ok_or_else(|| BumbleError::Remote("scripted transport already consumed".into()))
    }
}

struct ScriptedSource {
    responses: VecDeque<BumbleResult<Option<HciPacket>>>,
    dropped: Sender<&'static str>,
}

impl PacketSource for ScriptedSource {
    fn read_packet(&mut self) -> BumbleResult<Option<HciPacket>> {
        self.responses.pop_front().unwrap_or(Ok(None))
    }
}

impl Drop for ScriptedSource {
    fn drop(&mut self) {
        let _ = self.dropped.send("source");
    }
}

#[derive(Clone)]
struct ControlledSourceControl {
    shared: Arc<ControlledSourceShared>,
}

struct ControlledSourceShared {
    state: Mutex<ControlledSourceState>,
    wake: Condvar,
}

struct ControlledSourceState {
    queued: VecDeque<BumbleResult<Option<HciPacket>>>,
    shutdown_requested: bool,
}

impl ControlledSourceControl {
    fn push(&self, result: BumbleResult<Option<HciPacket>>) {
        lock(&self.shared.state).queued.push_back(result);
        self.shared.wake.notify_all();
    }

    fn finish(&self) {
        self.push(Ok(None));
    }
}

impl PacketSourceShutdown for ControlledSourceControl {
    fn request_shutdown(&self) {
        lock(&self.shared.state).shutdown_requested = true;
        self.shared.wake.notify_all();
    }
}

struct ControlledSource {
    control: ControlledSourceControl,
    dropped: Sender<&'static str>,
}

impl PacketSource for ControlledSource {
    fn read_packet(&mut self) -> BumbleResult<Option<HciPacket>> {
        let mut state = lock(&self.control.shared.state);
        loop {
            if let Some(result) = state.queued.pop_front() {
                return result;
            }
            if state.shutdown_requested {
                return Ok(None);
            }
            state = self
                .control
                .shared
                .wake
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn shutdown_handle(&self) -> Option<Arc<dyn PacketSourceShutdown>> {
        Some(Arc::new(self.control.clone()))
    }
}

impl Drop for ControlledSource {
    fn drop(&mut self) {
        let _ = self.dropped.send("source");
    }
}

struct RecordingSink {
    commands: Arc<Mutex<Vec<Command>>>,
    fail_opcode: Option<u16>,
    dropped: Sender<&'static str>,
}

impl PacketSink for RecordingSink {
    fn write_packet(&mut self, packet: &HciPacket) -> BumbleResult<()> {
        let HciPacket::Command(command) = packet else {
            return Err(BumbleError::Remote(
                "scripted sink accepts HCI commands only".into(),
            ));
        };
        lock(&self.commands).push(command.clone());
        if self.fail_opcode == Some(command.op_code()) {
            return Err(BumbleError::Remote(
                "secret scripted identity write failure".into(),
            ));
        }
        Ok(())
    }
}

impl Drop for RecordingSink {
    fn drop(&mut self) {
        let _ = self.dropped.send("sink");
    }
}

fn scripted_transport(
    responses: Vec<HciPacket>,
    commands: Arc<Mutex<Vec<Command>>>,
    fail_opcode: Option<u16>,
) -> (SplitOpenedTransport, Receiver<&'static str>) {
    let (dropped, drops) = channel();
    let mut responses = responses
        .into_iter()
        .map(|packet| Ok(Some(packet)))
        .collect::<VecDeque<_>>();
    responses.push_back(Ok(None));
    let metadata = BTreeMap::from([
        ("vendor_id".into(), "0a12".into()),
        ("product_id".into(), "0001".into()),
        ("bus".into(), "1".into()),
        ("address".into(), "7".into()),
    ]);
    (
        SplitOpenedTransport {
            source: Box::new(ScriptedSource {
                responses,
                dropped: dropped.clone(),
            }),
            sink: Box::new(RecordingSink {
                commands,
                fail_opcode,
                dropped,
            }),
            metadata,
        },
        drops,
    )
}

fn controlled_session() -> (
    BumbleSession,
    ControlledSourceControl,
    Receiver<()>,
    Receiver<&'static str>,
) {
    let config = TransportConfig::for_model::<Pro>();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let (transport, source, drops) =
        controlled_transport(successful_initialization_responses(&config), commands);
    let selectors = Arc::new(Mutex::new(Vec::new()));
    let mut opener = ScriptedOpener::new(transport, selectors);
    let (activity, wakes) = activity_channel();
    let session = initialize_bumble_session_with(
        &mut opener,
        &AdapterSelector::from("usb:0A12:0001"),
        &config,
        activity,
    )
    .expect("controlled Bumble initialization");
    (session, source, wakes, drops)
}

fn controlled_transport(
    responses: Vec<HciPacket>,
    commands: Arc<Mutex<Vec<Command>>>,
) -> (
    SplitOpenedTransport,
    ControlledSourceControl,
    Receiver<&'static str>,
) {
    let (dropped, drops) = channel();
    let shared = Arc::new(ControlledSourceShared {
        state: Mutex::new(ControlledSourceState {
            queued: responses
                .into_iter()
                .map(|packet| Ok(Some(packet)))
                .collect(),
            shutdown_requested: false,
        }),
        wake: Condvar::new(),
    });
    let control = ControlledSourceControl { shared };
    let metadata = BTreeMap::from([
        ("vendor_id".into(), "0a12".into()),
        ("product_id".into(), "0001".into()),
        ("bus".into(), "1".into()),
        ("address".into(), "7".into()),
    ]);
    (
        SplitOpenedTransport {
            source: Box::new(ControlledSource {
                control: control.clone(),
                dropped: dropped.clone(),
            }),
            sink: Box::new(RecordingSink {
                commands,
                fail_opcode: None,
                dropped,
            }),
            metadata,
        },
        control,
        drops,
    )
}

fn successful_initialization_responses(config: &TransportConfig) -> Vec<HciPacket> {
    let mut responses = controller_initialization_responses();
    responses.push(command_complete(
        Command::ReadBdAddr,
        ReturnParameters::ReadBdAddr {
            status: 0,
            bd_addr: Address::from_bytes(HCI_ADDRESS, AddressType::PUBLIC_DEVICE),
        },
    ));
    responses.extend(
        identity_commands(config)
            .into_iter()
            .map(|command| command_complete(command, ReturnParameters::Raw { data: vec![0] })),
    );
    responses
}

fn controller_initialization_responses() -> Vec<HciPacket> {
    let mut supported_commands = [0; 64];
    supported_commands[14] = 0xc8;
    vec![
        command_complete(Command::Reset, ReturnParameters::Status { status: 0 }),
        command_complete(
            Command::ReadLocalSupportedCommands,
            ReturnParameters::ReadLocalSupportedCommands {
                status: 0,
                supported_commands,
            },
        ),
        command_complete(
            Command::ReadLocalVersionInformation,
            ReturnParameters::ReadLocalVersionInformation {
                status: 0,
                hci_version: 0x09,
                hci_subversion: 0x1234,
                lmp_version: 0x09,
                company_identifier: 0x000a,
                lmp_subversion: 0x5678,
            },
        ),
        command_complete(
            Command::ReadLocalExtendedFeatures { page_number: 0 },
            ReturnParameters::ReadLocalExtendedFeatures {
                status: 0,
                page_number: 0,
                maximum_page_number: 0,
                extended_lmp_features: [0; 8],
            },
        ),
        command_complete(
            Command::SetEventMask {
                event_mask: HOST_EVENT_MASK,
            },
            ReturnParameters::Raw { data: vec![0] },
        ),
        command_complete(
            Command::LeSetEventMask {
                le_event_mask: HOST_LE_EVENT_MASK,
            },
            ReturnParameters::Raw { data: vec![0] },
        ),
        command_complete(
            Command::ReadBufferSize,
            ReturnParameters::ReadBufferSize {
                status: 0,
                hc_acl_data_packet_length: 1021,
                hc_synchronous_data_packet_length: 0,
                hc_total_num_acl_data_packets: 8,
                hc_total_num_synchronous_data_packets: 0,
            },
        ),
    ]
}

fn expected_commands(config: &TransportConfig) -> Vec<Command> {
    let mut commands = vec![
        Command::Reset,
        Command::ReadLocalSupportedCommands,
        Command::ReadLocalVersionInformation,
        Command::ReadLocalExtendedFeatures { page_number: 0 },
        Command::SetEventMask {
            event_mask: HOST_EVENT_MASK,
        },
        Command::LeSetEventMask {
            le_event_mask: HOST_LE_EVENT_MASK,
        },
        Command::ReadBufferSize,
        Command::ReadBdAddr,
    ];
    commands.extend(identity_commands(config));
    commands
}

fn identity_commands(config: &TransportConfig) -> [Command; 5] {
    let mut local_name = [0; 248];
    local_name[..config.local_name().len()].copy_from_slice(config.local_name().as_bytes());
    [
        Command::WriteLocalName { local_name },
        Command::WriteClassOfDevice {
            class_of_device: config.class_of_device(),
        },
        Command::WriteSimplePairingMode {
            simple_pairing_mode: 1,
        },
        Command::WriteExtendedInquiryResponse {
            fec_required: 0,
            extended_inquiry_response: *config.extended_inquiry_response(),
        },
        Command::WriteScanEnable { scan_enable: 0 },
    ]
}

fn command_complete(command: Command, return_parameters: ReturnParameters) -> HciPacket {
    HciPacket::Event(Event::CommandComplete {
        num_hci_command_packets: 1,
        command_opcode: command.op_code(),
        return_parameters,
    })
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
