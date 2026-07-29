use bumble_hid::{
    DeviceDelegate, DeviceEvent, DeviceRuntime, Handshake, Message, ReportType, device_data,
};

use super::HidChannel;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum HidpBridgeEvent {
    Output {
        channel: HidChannel,
        payload: Box<[u8]>,
    },
    ControlResponse(Box<[u8]>),
    Suspend,
    Resume,
    VirtualCableUnplug,
    Unsupported {
        channel: HidChannel,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HidpBridgeError {
    Malformed {
        channel: HidChannel,
    },
    EncodeFailed {
        channel: HidChannel,
    },
    PeerMtuExceeded {
        channel: HidChannel,
        encoded_len: usize,
        peer_mtu: usize,
    },
}

#[derive(Debug, Default)]
struct UnsupportedDelegate;

impl DeviceDelegate for UnsupportedDelegate {}

pub(super) struct HidpBridge {
    runtime: DeviceRuntime<UnsupportedDelegate>,
    control_peer_mtu: usize,
    interrupt_peer_mtu: usize,
}

impl HidpBridge {
    pub(super) fn new(control_peer_mtu: usize, interrupt_peer_mtu: usize) -> Self {
        Self {
            runtime: DeviceRuntime::new(UnsupportedDelegate, control_peer_mtu),
            control_peer_mtu,
            interrupt_peer_mtu,
        }
    }

    pub(super) fn handle(
        &mut self,
        channel: HidChannel,
        bytes: &[u8],
    ) -> Result<Vec<HidpBridgeEvent>, HidpBridgeError> {
        let device_events = match channel {
            HidChannel::Control => self
                .runtime
                .handle_control(bytes)
                .map_err(|_| HidpBridgeError::Malformed { channel })?,
            HidChannel::Interrupt => vec![
                self.runtime
                    .handle_interrupt(bytes)
                    .map_err(|_| HidpBridgeError::Malformed { channel })?,
            ],
        };

        let mut events = Vec::with_capacity(device_events.len());
        for event in device_events {
            self.translate_event(channel, event, &mut events)?;
        }
        Ok(events)
    }

    pub(super) fn encode_input(&self, payload: &[u8]) -> Result<Box<[u8]>, HidpBridgeError> {
        let encoded = device_data(payload.to_vec()).to_bytes().map_err(|_| {
            HidpBridgeError::EncodeFailed {
                channel: HidChannel::Interrupt,
            }
        })?;
        check_peer_mtu(HidChannel::Interrupt, encoded, self.interrupt_peer_mtu)
    }

    pub(super) fn set_peer_mtu(&mut self, channel: HidChannel, peer_mtu: usize) {
        match channel {
            HidChannel::Control => self.control_peer_mtu = peer_mtu,
            HidChannel::Interrupt => self.interrupt_peer_mtu = peer_mtu,
        }
    }

    pub(super) fn invalid_parameter_response(&self) -> Result<Box<[u8]>, HidpBridgeError> {
        self.encode_control(Message::Handshake(Handshake::ERR_INVALID_PARAMETER))
    }

    fn translate_event(
        &self,
        channel: HidChannel,
        event: DeviceEvent,
        output: &mut Vec<HidpBridgeEvent>,
    ) -> Result<(), HidpBridgeError> {
        match event {
            DeviceEvent::SendControl(message) => {
                output.push(HidpBridgeEvent::ControlResponse(
                    self.encode_control(message)?,
                ));
            }
            DeviceEvent::ControlData { report_type, data }
            | DeviceEvent::InterruptData { report_type, data } => {
                output.push(if report_type == ReportType::OUTPUT_REPORT {
                    HidpBridgeEvent::Output {
                        channel,
                        payload: data.into_boxed_slice(),
                    }
                } else {
                    HidpBridgeEvent::Unsupported { channel }
                });
            }
            DeviceEvent::Suspend => output.push(HidpBridgeEvent::Suspend),
            DeviceEvent::ExitSuspend => output.push(HidpBridgeEvent::Resume),
            DeviceEvent::VirtualCableUnplug => output.push(HidpBridgeEvent::VirtualCableUnplug),
            DeviceEvent::Unsupported(_) => {
                output.push(HidpBridgeEvent::Unsupported { channel });
            }
        }
        Ok(())
    }

    fn encode_control(&self, message: Message) -> Result<Box<[u8]>, HidpBridgeError> {
        let encoded = message
            .to_bytes()
            .map_err(|_| HidpBridgeError::EncodeFailed {
                channel: HidChannel::Control,
            })?;
        check_peer_mtu(HidChannel::Control, encoded, self.control_peer_mtu)
    }
}

fn check_peer_mtu(
    channel: HidChannel,
    encoded: Vec<u8>,
    peer_mtu: usize,
) -> Result<Box<[u8]>, HidpBridgeError> {
    if encoded.len() > peer_mtu {
        return Err(HidpBridgeError::PeerMtuExceeded {
            channel,
            encoded_len: encoded.len(),
            peer_mtu,
        });
    }
    Ok(encoded.into_boxed_slice())
}

#[cfg(test)]
mod tests {
    use super::{HidpBridge, HidpBridgeError, HidpBridgeEvent};
    use crate::runtime::transport::HidChannel;

    #[test]
    fn output_data_from_both_channels_removes_the_hidp_header() {
        let mut bridge = HidpBridge::new(48, 48);

        for channel in [HidChannel::Control, HidChannel::Interrupt] {
            assert_eq!(
                bridge.handle(channel, &[0xA2, 0x01, 0x00, 0x03]),
                Ok(vec![HidpBridgeEvent::Output {
                    channel,
                    payload: Box::from([0x01, 0x00, 0x03]),
                }])
            );
        }
    }

    #[test]
    fn control_requests_produce_responses_and_unsupported_events() {
        let mut bridge = HidpBridge::new(48, 48);

        assert_eq!(
            bridge.handle(HidChannel::Control, &[0x42, 0x30]),
            Ok(vec![HidpBridgeEvent::ControlResponse(Box::from([0x03]))])
        );
        assert_eq!(
            bridge.handle(HidChannel::Control, &[0xF0]),
            Ok(vec![
                HidpBridgeEvent::Unsupported {
                    channel: HidChannel::Control,
                },
                HidpBridgeEvent::ControlResponse(Box::from([0x03])),
            ])
        );
        assert_eq!(
            bridge.handle(HidChannel::Interrupt, &[0x13]),
            Ok(vec![HidpBridgeEvent::Unsupported {
                channel: HidChannel::Interrupt,
            }])
        );
    }

    #[test]
    fn malformed_control_and_interrupt_messages_return_typed_errors() {
        let mut bridge = HidpBridge::new(48, 48);

        for (channel, bytes) in [
            (HidChannel::Control, Vec::new()),
            (HidChannel::Control, vec![0x41]),
            (HidChannel::Control, vec![0x60, 0x00]),
            (HidChannel::Interrupt, Vec::new()),
        ] {
            assert_eq!(
                bridge.handle(channel, &bytes),
                Err(HidpBridgeError::Malformed { channel })
            );
        }
    }

    #[test]
    fn input_data_adds_a1_and_rejects_messages_above_the_interrupt_peer_mtu() {
        let bridge = HidpBridge::new(48, 50);
        let maximum_payload = vec![0x30; 49];
        let mut expected = vec![0xA1];
        expected.extend_from_slice(&maximum_payload);

        assert_eq!(
            bridge.encode_input(&maximum_payload),
            Ok(expected.into_boxed_slice())
        );
        assert_eq!(
            bridge.encode_input(&[0x30; 50]),
            Err(HidpBridgeError::PeerMtuExceeded {
                channel: HidChannel::Interrupt,
                encoded_len: 51,
                peer_mtu: 50,
            })
        );
    }

    #[test]
    fn a_control_response_above_the_peer_mtu_is_rejected() {
        let mut bridge = HidpBridge::new(0, 48);

        assert_eq!(
            bridge.handle(HidChannel::Control, &[0x42, 0x30]),
            Err(HidpBridgeError::PeerMtuExceeded {
                channel: HidChannel::Control,
                encoded_len: 1,
                peer_mtu: 0,
            })
        );
    }
}
