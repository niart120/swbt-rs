use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

use bumble::Address;
use bumble_host::{Device, DeviceEvent, HostTransport};
use bumble_l2cap::{ClassicChannelSpec, ClassicChannelState};

use super::config::{HidServiceConfig, TransportConfig};
use super::hidp::{HidpBridge, HidpBridgeError, HidpBridgeEvent};
use super::sdp::HidSdpChannel;
use super::{
    ActivityNotifier, HidChannel, SendAcceptance, TransportError, TransportErrorKind,
    TransportEvent, TransportResult,
};

pub(super) const SDP_PSM: u32 = 0x0001;
pub(super) const HID_CONTROL_PSM: u32 = 0x0011;
pub(super) const HID_INTERRUPT_PSM: u32 = 0x0013;

const SERVER_MTU: u16 = 672;
const MAX_SDP_SDUS_PER_POLL: usize = 16;
const EVENT_QUEUE_CAPACITY: usize = 64;

pub(super) struct ClassicDeviceSession {
    configuration: HidServiceConfig,
    activity: ActivityNotifier,
    servers_registered: bool,
    current: Option<ConnectionSession>,
    events: VecDeque<TransportEvent>,
    terminal: Option<TransportError>,
}

struct ConnectionSession {
    handle: u16,
    peer_address: Address,
    sdp_channels: BTreeMap<u16, SdpChannel>,
    control: Option<Channel>,
    interrupt: Option<Channel>,
    hidp: HidpBridge,
}

struct SdpChannel {
    runtime: HidSdpChannel,
    pending: VecDeque<Vec<u8>>,
}

#[derive(Clone, Copy)]
struct Channel {
    cid: u16,
}

impl ClassicDeviceSession {
    pub(super) fn new(configuration: &TransportConfig, activity: ActivityNotifier) -> Self {
        Self {
            configuration: configuration.hid_service.clone(),
            activity,
            servers_registered: false,
            current: None,
            events: VecDeque::with_capacity(EVENT_QUEUE_CAPACITY),
            terminal: None,
        }
    }

    pub(super) fn register_servers(&mut self, device: &mut Device) -> TransportResult<()> {
        if self.servers_registered {
            return Ok(());
        }

        let mut registered = Vec::new();
        for psm in [SDP_PSM, HID_CONTROL_PSM, HID_INTERRUPT_PSM] {
            match device
                .register_classic_channel_server(Some(psm), ClassicChannelSpec { mtu: SERVER_MTU })
            {
                Ok(actual) if actual == psm => registered.push(actual),
                Ok(actual) => {
                    device.unregister_classic_channel_server(actual);
                    for registered_psm in registered {
                        device.unregister_classic_channel_server(registered_psm);
                    }
                    return Err(TransportError::new(TransportErrorKind::OpenFailed));
                }
                Err(error) => {
                    for registered_psm in registered {
                        device.unregister_classic_channel_server(registered_psm);
                    }
                    return Err(TransportError::with_source(
                        TransportErrorKind::OpenFailed,
                        Arc::new(error),
                    ));
                }
            }
        }
        self.servers_registered = true;
        Ok(())
    }

    pub(super) fn poll(
        &mut self,
        device: &mut Device,
        link: &mut (dyn HostTransport + 'static),
    ) -> TransportResult<Vec<TransportEvent>> {
        if let Some(terminal) = &self.terminal {
            return Err(terminal.clone());
        }

        self.process_device_events(device.take_device_events());
        if self.current.is_some() {
            self.accept_channels(device);
            self.process_sdp(device, link)?;
            self.process_hid(device, link)?;
        }
        self.take_events()
    }

    pub(super) fn send_interrupt(
        &mut self,
        device: &mut Device,
        link: &mut (dyn HostTransport + 'static),
        payload: &[u8],
    ) -> TransportResult<SendAcceptance> {
        if let Some(terminal) = &self.terminal {
            return Err(terminal.clone());
        }
        let Some(current) = self.current.as_ref() else {
            return Err(TransportError::new(TransportErrorKind::SendRejected));
        };
        let Some(channel) = current.interrupt else {
            return Err(TransportError::new(TransportErrorKind::SendRejected));
        };
        let is_open = device
            .classic_channel(current.handle, channel.cid)
            .is_some_and(|candidate| {
                candidate.state == ClassicChannelState::Open && candidate.psm == HID_INTERRUPT_PSM
            });
        if !is_open {
            return Err(TransportError::new(TransportErrorKind::SendRejected));
        }
        let encoded = current
            .hidp
            .encode_input(payload)
            .map_err(|_| TransportError::new(TransportErrorKind::SendRejected))?;
        device
            .send_classic_channel_sdu(link, current.handle, channel.cid, &encoded)
            .map_err(|error| {
                TransportError::with_source(TransportErrorKind::SendRejected, Arc::new(error))
            })?;
        Ok(SendAcceptance::ACCEPTED)
    }

    fn process_device_events(&mut self, events: impl IntoIterator<Item = DeviceEvent>) {
        for event in events {
            match event {
                DeviceEvent::ClassicConnectionEstablished(connection) => {
                    let is_current = self.current.as_ref().is_some_and(|current| {
                        current.handle == connection.connection_handle
                            && current.peer_address == connection.peer_address
                    });
                    if self.current.is_none() {
                        self.current = Some(ConnectionSession {
                            handle: connection.connection_handle,
                            peer_address: connection.peer_address,
                            sdp_channels: BTreeMap::new(),
                            control: None,
                            interrupt: None,
                            hidp: HidpBridge::new(0, 0),
                        });
                        self.enqueue(TransportEvent::Connected);
                    } else if is_current {
                        continue;
                    }
                }
                DeviceEvent::Disconnected {
                    connection_handle,
                    reason,
                } if self
                    .current
                    .as_ref()
                    .is_some_and(|current| current.handle == connection_handle) =>
                {
                    self.current = None;
                    self.enqueue(TransportEvent::Disconnected {
                        reason: Some(reason),
                    });
                }
                _ => {}
            }
        }
    }

    fn accept_channels(&mut self, device: &mut Device) {
        let Some(handle) = self.current.as_ref().map(|current| current.handle) else {
            return;
        };
        for cid in device.take_accepted_classic_channels(handle) {
            let Some(channel) = device.classic_channel(handle, cid) else {
                continue;
            };
            if channel.state != ClassicChannelState::Open {
                continue;
            }
            let psm = channel.psm;
            let peer_mtu = channel.peer_mtu;
            let mut opened = None;
            if let Some(current) = self.current.as_mut() {
                match psm {
                    SDP_PSM => {
                        current
                            .sdp_channels
                            .entry(cid)
                            .or_insert_with(|| SdpChannel {
                                runtime: HidSdpChannel::new(&self.configuration, peer_mtu),
                                pending: VecDeque::new(),
                            });
                    }
                    HID_CONTROL_PSM if current.control.is_none() => {
                        current.control = Some(Channel { cid });
                        current
                            .hidp
                            .set_peer_mtu(HidChannel::Control, usize::from(peer_mtu));
                        opened = Some(HidChannel::Control);
                    }
                    HID_INTERRUPT_PSM if current.interrupt.is_none() => {
                        current.interrupt = Some(Channel { cid });
                        current
                            .hidp
                            .set_peer_mtu(HidChannel::Interrupt, usize::from(peer_mtu));
                        opened = Some(HidChannel::Interrupt);
                    }
                    _ => {}
                }
            }
            if let Some(channel) = opened {
                self.enqueue(TransportEvent::HidChannelOpened { channel });
            }
        }
    }

    fn process_sdp(
        &mut self,
        device: &mut Device,
        link: &mut (dyn HostTransport + 'static),
    ) -> TransportResult<()> {
        let Some(handle) = self.current.as_ref().map(|current| current.handle) else {
            return Ok(());
        };
        let cids = self
            .current
            .as_ref()
            .map(|current| current.sdp_channels.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for cid in &cids {
            let received = device.take_classic_channel_sdus(handle, *cid);
            if let Some(channel) = self
                .current
                .as_mut()
                .and_then(|current| current.sdp_channels.get_mut(cid))
            {
                channel.pending.extend(received);
            }
        }

        let mut remaining_budget = MAX_SDP_SDUS_PER_POLL;
        let mut responses = Vec::new();
        if let Some(current) = self.current.as_mut() {
            for (cid, channel) in &mut current.sdp_channels {
                while remaining_budget != 0 {
                    let Some(request) = channel.pending.pop_front() else {
                        break;
                    };
                    if let Some(response) = channel.runtime.handle_sdu(&request) {
                        responses.push((*cid, response));
                    }
                    remaining_budget -= 1;
                }
            }
        }
        for (cid, response) in responses {
            device
                .send_classic_channel_sdu(link, handle, cid, &response)
                .map_err(map_source_terminated)?;
        }
        let work_remains = self.current.as_ref().is_some_and(|current| {
            current
                .sdp_channels
                .values()
                .any(|channel| !channel.pending.is_empty())
        });
        if work_remains {
            self.activity.notify();
        }
        Ok(())
    }

    fn process_hid(
        &mut self,
        device: &mut Device,
        link: &mut (dyn HostTransport + 'static),
    ) -> TransportResult<()> {
        let Some(current) = self.current.as_ref() else {
            return Ok(());
        };
        let handle = current.handle;
        let channels = [
            (HidChannel::Control, current.control),
            (HidChannel::Interrupt, current.interrupt),
        ];
        for (channel_kind, channel) in channels {
            let Some(channel) = channel else { continue };
            for sdu in device.take_classic_channel_sdus(handle, channel.cid) {
                let Some(current) = self.current.as_mut() else {
                    return Ok(());
                };
                let result = current.hidp.handle(channel_kind, &sdu);
                match result {
                    Ok(events) => {
                        self.apply_hidp_events(device, link, handle, events)?;
                    }
                    Err(HidpBridgeError::Malformed {
                        channel: HidChannel::Control,
                    }) => {
                        let response = self
                            .current
                            .as_ref()
                            .and_then(|current| current.hidp.invalid_parameter_response().ok());
                        if let Some(response) = response {
                            device
                                .send_classic_channel_sdu(link, handle, channel.cid, &response)
                                .map_err(map_source_terminated)?;
                        }
                    }
                    Err(_) => {}
                }
                if self.terminal.is_some() {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    fn apply_hidp_events(
        &mut self,
        device: &mut Device,
        link: &mut (dyn HostTransport + 'static),
        handle: u16,
        events: Vec<HidpBridgeEvent>,
    ) -> TransportResult<()> {
        for event in events {
            match event {
                HidpBridgeEvent::Output { channel, payload } => {
                    self.enqueue(TransportEvent::HidOutput { channel, payload });
                }
                HidpBridgeEvent::ControlResponse(response) => {
                    if let Some(control) = self.current.as_ref().and_then(|current| current.control)
                    {
                        device
                            .send_classic_channel_sdu(link, handle, control.cid, &response)
                            .map_err(map_source_terminated)?;
                    }
                }
                HidpBridgeEvent::Suspend
                | HidpBridgeEvent::Resume
                | HidpBridgeEvent::VirtualCableUnplug
                | HidpBridgeEvent::Unsupported { .. } => {}
            }
        }
        Ok(())
    }

    fn enqueue(&mut self, event: TransportEvent) {
        if self.terminal.is_some() {
            return;
        }
        if self.events.len() == EVENT_QUEUE_CAPACITY {
            self.terminal = Some(TransportError::new(TransportErrorKind::EventQueueOverflow));
            return;
        }
        self.events.push_back(event);
    }

    fn take_events(&mut self) -> TransportResult<Vec<TransportEvent>> {
        if let Some(terminal) = &self.terminal {
            return Err(terminal.clone());
        }
        Ok(self.events.drain(..).collect())
    }
}

fn map_source_terminated(error: bumble_l2cap::Error) -> TransportError {
    TransportError::with_source(TransportErrorKind::SourceTerminated, Arc::new(error))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::TryRecvError;

    use bumble::{Address, AddressType};
    use bumble_controller::{Controller, LocalLink};
    use bumble_host::{Device, DeviceEvent, pump};
    use bumble_l2cap::ClassicChannelSpec;
    use bumble_sdp::{DataElement, SdpPdu};

    use crate::model::Pro;

    use super::{ClassicDeviceSession, HID_CONTROL_PSM, HID_INTERRUPT_PSM, SDP_PSM};
    use crate::runtime::transport::{
        HidChannel, SendAcceptance, TransportConfig, TransportErrorKind, TransportEvent,
        activity_channel,
    };

    const INITIATOR_ADDRESS: &str = "11:11:11:11:11:11";
    const RESPONDER_ADDRESS: &str = "22:22:22:22:22:22";

    #[test]
    fn registers_three_servers_once_and_accepts_reverse_hid_order_once() {
        let mut fixture = Fixture::connected();

        fixture
            .session
            .register_servers(&mut fixture.devices[1])
            .expect("repeated server registration is idempotent");
        assert_eq!(
            fixture.poll(),
            [TransportEvent::Connected],
            "the established ACL produces one event"
        );
        assert!(fixture.poll().is_empty());

        fixture.connect_channel(HID_INTERRUPT_PSM, 96);
        assert_eq!(
            fixture.poll(),
            [TransportEvent::HidChannelOpened {
                channel: HidChannel::Interrupt,
            }]
        );
        assert!(fixture.poll().is_empty());

        fixture.connect_channel(SDP_PSM, 48);
        assert!(fixture.poll().is_empty(), "SDP stays transport-internal");

        fixture.connect_channel(HID_CONTROL_PSM, 80);
        assert_eq!(
            fixture.poll(),
            [TransportEvent::HidChannelOpened {
                channel: HidChannel::Control,
            }]
        );
        assert!(fixture.poll().is_empty());
    }

    #[test]
    fn control_and_interrupt_outputs_route_then_disconnect_discards_the_old_session() {
        let mut fixture = Fixture::connected();
        assert_eq!(fixture.poll(), [TransportEvent::Connected]);
        let control_cid = fixture.connect_channel(HID_CONTROL_PSM, 80);
        assert_eq!(
            fixture.poll(),
            [TransportEvent::HidChannelOpened {
                channel: HidChannel::Control,
            }]
        );
        let interrupt_cid = fixture.connect_channel(HID_INTERRUPT_PSM, 96);
        assert_eq!(
            fixture.poll(),
            [TransportEvent::HidChannelOpened {
                channel: HidChannel::Interrupt,
            }]
        );

        fixture.send_peer_sdu(control_cid, &[0xA2, 0x01, 0x00]);
        fixture.send_peer_sdu(interrupt_cid, &[0xA2, 0x10, 0x2A]);
        assert_eq!(
            fixture.poll(),
            [
                TransportEvent::HidOutput {
                    channel: HidChannel::Control,
                    payload: Box::from([0x01, 0x00]),
                },
                TransportEvent::HidOutput {
                    channel: HidChannel::Interrupt,
                    payload: Box::from([0x10, 0x2A]),
                },
            ]
        );

        assert!(fixture.devices[0].disconnect_handle(
            &mut fixture.link,
            fixture.initiator_handle,
            0x13,
        ));
        pump(&mut fixture.link, &mut fixture.devices);
        assert_eq!(
            fixture.poll(),
            [TransportEvent::Disconnected { reason: Some(0x13) }]
        );
        assert_eq!(
            fixture
                .session
                .send_interrupt(&mut fixture.devices[1], &mut fixture.link, &[0x30, 0x00],)
                .expect_err("old interrupt CID is discarded")
                .kind(),
            TransportErrorKind::SendRejected
        );

        fixture
            .session
            .process_device_events([DeviceEvent::Disconnected {
                connection_handle: fixture.responder_handle,
                reason: 0x16,
            }]);
        assert!(
            fixture
                .session
                .take_events()
                .expect("duplicate old disconnect is harmless")
                .is_empty()
        );
    }

    #[test]
    fn interrupt_send_adds_hidp_header_and_requires_an_open_current_channel() {
        let mut fixture = Fixture::connected();
        fixture.poll();
        assert_eq!(
            fixture
                .session
                .send_interrupt(&mut fixture.devices[1], &mut fixture.link, &[0x30; 49],)
                .expect_err("interrupt channel is not open")
                .kind(),
            TransportErrorKind::SendRejected
        );

        let interrupt_cid = fixture.connect_channel(HID_INTERRUPT_PSM, 50);
        fixture.poll();
        assert_eq!(
            fixture
                .session
                .send_interrupt(&mut fixture.devices[1], &mut fixture.link, &[0x30; 49],)
                .expect("header plus payload fits exactly"),
            SendAcceptance::ACCEPTED
        );
        pump(&mut fixture.link, &mut fixture.devices);
        let received =
            fixture.devices[0].take_classic_channel_sdus(fixture.initiator_handle, interrupt_cid);
        assert_eq!(received, [input_pdu(&[0x30; 49])]);

        assert_eq!(
            fixture
                .session
                .send_interrupt(&mut fixture.devices[1], &mut fixture.link, &[0x30; 50],)
                .expect_err("encoded packet exceeds peer MTU")
                .kind(),
            TransportErrorKind::SendRejected
        );
    }

    #[test]
    fn malformed_control_gets_invalid_parameter_while_interrupt_noise_is_dropped() {
        let mut fixture = Fixture::connected();
        fixture.poll();
        let control_cid = fixture.connect_channel(HID_CONTROL_PSM, 80);
        fixture.poll();
        let interrupt_cid = fixture.connect_channel(HID_INTERRUPT_PSM, 80);
        fixture.poll();

        fixture.send_peer_sdu(control_cid, &[0x41]);
        assert!(fixture.poll().is_empty());
        pump(&mut fixture.link, &mut fixture.devices);
        assert_eq!(
            fixture.devices[0].take_classic_channel_sdus(fixture.initiator_handle, control_cid),
            [vec![0x04]]
        );

        fixture.send_peer_sdu(interrupt_cid, &[0x41]);
        fixture.send_peer_sdu(interrupt_cid, &[0x13]);
        assert!(fixture.poll().is_empty());
        pump(&mut fixture.link, &mut fixture.devices);
        assert!(
            fixture.devices[0]
                .take_classic_channel_sdus(fixture.initiator_handle, interrupt_cid)
                .is_empty()
        );
    }

    #[test]
    fn one_poll_processes_sixteen_sdp_requests_and_renotifies_for_the_remainder() {
        let mut fixture = Fixture::connected();
        fixture.poll();
        let sdp_cid = fixture.connect_channel(SDP_PSM, 672);
        fixture.poll();

        for transaction_id in 1..=17 {
            fixture.send_peer_sdu(sdp_cid, &sdp_request(transaction_id));
        }
        assert!(fixture.poll().is_empty());
        assert_eq!(
            fixture.wakes.try_recv(),
            Ok(()),
            "remaining SDP work must wake the worker again"
        );
        pump(&mut fixture.link, &mut fixture.devices);
        assert_eq!(
            fixture.devices[0]
                .take_classic_channel_sdus(fixture.initiator_handle, sdp_cid)
                .len(),
            16
        );

        assert!(fixture.poll().is_empty());
        pump(&mut fixture.link, &mut fixture.devices);
        assert_eq!(
            fixture.devices[0]
                .take_classic_channel_sdus(fixture.initiator_handle, sdp_cid)
                .len(),
            1
        );
        assert!(matches!(
            fixture.wakes.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected)
        ));
    }

    #[test]
    fn sixty_fifth_worker_event_becomes_a_sticky_overflow() {
        let mut fixture = Fixture::connected();
        fixture.poll();
        let interrupt_cid = fixture.connect_channel(HID_INTERRUPT_PSM, 96);
        fixture.poll();

        for value in 0..65u8 {
            fixture.send_peer_sdu(interrupt_cid, &[0xA2, 0x10, value]);
        }
        assert_eq!(
            fixture
                .session
                .poll(&mut fixture.devices[1], &mut fixture.link)
                .expect_err("the 65th worker event is terminal")
                .kind(),
            TransportErrorKind::EventQueueOverflow
        );
        assert_eq!(
            fixture
                .session
                .poll(&mut fixture.devices[1], &mut fixture.link)
                .expect_err("overflow remains sticky")
                .kind(),
            TransportErrorKind::EventQueueOverflow
        );
    }

    struct Fixture {
        link: LocalLink,
        devices: [Device; 2],
        session: ClassicDeviceSession,
        wakes: std::sync::mpsc::Receiver<()>,
        initiator_handle: u16,
        responder_handle: u16,
    }

    impl Fixture {
        fn connected() -> Self {
            let initiator_address = address(INITIATOR_ADDRESS);
            let responder_address = address(RESPONDER_ADDRESS);
            let mut link = LocalLink::new();
            let initiator_id =
                link.add_controller(Controller::new("initiator", initiator_address.clone()));
            let responder_id =
                link.add_controller(Controller::new("swbt", responder_address.clone()));
            let mut devices = [Device::new(initiator_id), Device::new(responder_id)];
            let (activity, wakes) = activity_channel();
            let mut session =
                ClassicDeviceSession::new(&TransportConfig::for_model::<Pro>(), activity);
            session
                .register_servers(&mut devices[1])
                .expect("register Classic HID servers");

            devices[0].connect_classic(&mut link, responder_address);
            devices[0].poll(&mut link);
            link.pump_classic();
            devices[1].poll(&mut link);
            devices[1].accept_classic(&mut link, initiator_address);
            devices[1].poll(&mut link);
            link.pump_classic();
            devices[0].poll(&mut link);

            let initiator_handle = devices[0]
                .classic_connection_handle()
                .expect("initiator Classic handle");
            let responder_handle = devices[1]
                .classic_connection_handle()
                .expect("responder Classic handle");
            Self {
                link,
                devices,
                session,
                wakes,
                initiator_handle,
                responder_handle,
            }
        }

        fn connect_channel(&mut self, psm: u32, peer_mtu: u16) -> u16 {
            let cid = self.devices[0]
                .connect_classic_channel(
                    &mut self.link,
                    self.initiator_handle,
                    psm,
                    ClassicChannelSpec { mtu: peer_mtu },
                )
                .expect("connect Classic channel");
            pump(&mut self.link, &mut self.devices);
            cid
        }

        fn send_peer_sdu(&mut self, cid: u16, bytes: &[u8]) {
            self.devices[0]
                .send_classic_channel_sdu(&mut self.link, self.initiator_handle, cid, bytes)
                .expect("send peer SDU");
            pump(&mut self.link, &mut self.devices);
        }

        fn poll(&mut self) -> Vec<TransportEvent> {
            self.session
                .poll(&mut self.devices[1], &mut self.link)
                .expect("poll Classic device session")
        }
    }

    fn address(value: &str) -> Address {
        Address::parse(value, AddressType::PUBLIC_DEVICE).expect("valid test address")
    }

    fn input_pdu(payload: &[u8]) -> Vec<u8> {
        let mut encoded = vec![0xA1];
        encoded.extend_from_slice(payload);
        encoded
    }

    fn sdp_request(transaction_id: u16) -> Vec<u8> {
        SdpPdu::ServiceSearchAttributeRequest {
            transaction_id,
            service_search_pattern: DataElement::sequence([DataElement::uuid(
                bumble::Uuid::from_16_bits(0x1124),
            )]),
            maximum_attribute_byte_count: u16::MAX,
            attribute_id_list: DataElement::sequence([DataElement::unsigned_integer_32(
                0x0000_FFFF,
            )]),
            continuation_state: vec![0],
        }
        .to_bytes()
        .expect("valid SDP request")
    }
}
