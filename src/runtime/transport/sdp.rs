use bumble::Uuid;
use bumble_sdp::service::{SdpRequestHandler, SdpServer};
use bumble_sdp::{DataElement, SdpPdu, ServiceAttribute, error_code};

use super::config::HidServiceConfig;

pub(super) const HID_SERVICE_RECORD_HANDLE: u32 = 0x0001_0001;

const HID_CONTROL_PSM: u16 = 0x0011;
const HID_INTERRUPT_PSM: u16 = 0x0013;
const HID_SERVICE_CLASS_UUID: u16 = 0x1124;
const L2CAP_PROTOCOL_UUID: u16 = 0x0100;
const HIDP_PROTOCOL_UUID: u16 = 0x0011;
const PUBLIC_BROWSE_ROOT_UUID: u16 = 0x1002;

#[derive(Debug)]
pub(super) struct HidSdpChannel {
    server: SdpServer,
}

impl HidSdpChannel {
    pub(super) fn new(configuration: &HidServiceConfig, peer_mtu: u16) -> Self {
        let mut server = SdpServer::new(peer_mtu);
        server.add_service(
            HID_SERVICE_RECORD_HANDLE,
            build_hid_service_record(configuration),
        );
        Self { server }
    }

    pub(super) fn handle_sdu(&mut self, sdu: &[u8]) -> Option<Vec<u8>> {
        let transaction_id = sdu
            .get(1..3)
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))?;
        let response = match parse_complete_pdu(sdu) {
            Ok(request) => self.server.handle_request(&request),
            Err(()) => SdpPdu::ErrorResponse {
                transaction_id,
                error_code: error_code::INVALID_REQUEST_SYNTAX,
            },
        };
        response.to_bytes().ok()
    }
}

fn parse_complete_pdu(sdu: &[u8]) -> Result<SdpPdu, ()> {
    let length_bytes = sdu.get(3..5).ok_or(())?;
    let parameter_length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    if sdu.len() != 5 + parameter_length {
        return Err(());
    }
    SdpPdu::from_bytes(sdu).map_err(|_| ())
}

fn build_hid_service_record(configuration: &HidServiceConfig) -> Vec<ServiceAttribute> {
    let policy = &configuration.sdp_policy;
    let mut attributes = vec![
        attribute(
            0x0000,
            DataElement::unsigned_integer_32(HID_SERVICE_RECORD_HANDLE),
        ),
        attribute(
            0x0001,
            DataElement::sequence([DataElement::uuid(uuid16(HID_SERVICE_CLASS_UUID))]),
        ),
        attribute(
            0x0004,
            DataElement::sequence([
                DataElement::sequence([
                    DataElement::uuid(uuid16(L2CAP_PROTOCOL_UUID)),
                    DataElement::unsigned_integer_16(HID_CONTROL_PSM),
                ]),
                DataElement::sequence([DataElement::uuid(uuid16(HIDP_PROTOCOL_UUID))]),
            ]),
        ),
        attribute(
            0x0005,
            DataElement::sequence([DataElement::uuid(uuid16(PUBLIC_BROWSE_ROOT_UUID))]),
        ),
        attribute(
            0x0006,
            DataElement::sequence([
                DataElement::unsigned_integer_16(0x656E),
                DataElement::unsigned_integer_16(0x006A),
                DataElement::unsigned_integer_16(0x0100),
            ]),
        ),
        attribute(
            0x0100,
            DataElement::text_string(policy.service_name.as_bytes()),
        ),
    ];
    if let Some(description) = &policy.service_description {
        attributes.push(attribute(
            0x0101,
            DataElement::text_string(description.as_bytes()),
        ));
    }
    if let Some(provider) = &policy.provider_name {
        attributes.push(attribute(
            0x0102,
            DataElement::text_string(provider.as_bytes()),
        ));
    }
    attributes.extend([
        attribute(
            0x0009,
            DataElement::sequence([DataElement::sequence([
                DataElement::uuid(uuid16(HID_SERVICE_CLASS_UUID)),
                DataElement::unsigned_integer_16(policy.bluetooth_profile_version),
            ])]),
        ),
        attribute(
            0x000D,
            DataElement::sequence([DataElement::sequence([
                DataElement::sequence([
                    DataElement::uuid(uuid16(L2CAP_PROTOCOL_UUID)),
                    DataElement::unsigned_integer_16(HID_INTERRUPT_PSM),
                ]),
                DataElement::sequence([DataElement::uuid(uuid16(HIDP_PROTOCOL_UUID))]),
            ])]),
        ),
    ]);
    if let Some(device_release_number) = policy.device_release_number {
        attributes.push(attribute(
            0x0200,
            DataElement::unsigned_integer_16(device_release_number),
        ));
    }
    attributes.extend([
        attribute(
            0x0201,
            DataElement::unsigned_integer_16(policy.parser_version),
        ),
        attribute(
            0x0202,
            DataElement::unsigned_integer_8(policy.device_subclass),
        ),
        attribute(0x0203, DataElement::unsigned_integer_8(policy.country_code)),
        attribute(0x0204, DataElement::boolean(policy.virtual_cable)),
        attribute(0x0205, DataElement::boolean(policy.reconnect_initiate)),
        attribute(
            0x0206,
            DataElement::sequence([DataElement::sequence([
                DataElement::unsigned_integer_8(0x22),
                DataElement::text_string(configuration.report_descriptor.as_ref()),
            ])]),
        ),
        attribute(
            0x0207,
            DataElement::sequence([DataElement::sequence([
                DataElement::unsigned_integer_16(0x0409),
                DataElement::unsigned_integer_16(0x0100),
            ])]),
        ),
    ]);
    if let Some(remote_wake) = policy.remote_wake {
        attributes.push(attribute(0x020A, DataElement::boolean(remote_wake)));
    }
    attributes.extend([
        attribute(
            0x020B,
            DataElement::unsigned_integer_16(policy.profile_version),
        ),
        attribute(
            0x020C,
            DataElement::unsigned_integer_16(policy.supervision_timeout),
        ),
        attribute(0x020D, DataElement::boolean(policy.normally_connectable)),
        attribute(0x020E, DataElement::boolean(policy.boot_device)),
        attribute(
            0x020F,
            DataElement::unsigned_integer_16(policy.ssr_host_max_latency),
        ),
        attribute(
            0x0210,
            DataElement::unsigned_integer_16(policy.ssr_host_min_timeout),
        ),
    ]);
    attributes
}

fn attribute(id: u16, value: DataElement) -> ServiceAttribute {
    ServiceAttribute::new(id, value)
}

fn uuid16(value: u16) -> Uuid {
    Uuid::from_16_bits(value)
}

#[cfg(test)]
mod tests {
    use bumble::Uuid;
    use bumble_sdp::{DataElement, SdpPdu, ServiceAttribute, error_code};
    use serde_json::Value;

    use crate::model::{JoyConL, JoyConR, Pro};

    use super::{HID_SERVICE_RECORD_HANDLE, HidSdpChannel};
    use crate::runtime::transport::config::TransportConfig;

    const FIXTURE: &str =
        include_str!("../../../tests/fixtures/python-v0.6.0/hid/hid-service-fixtures.json");

    #[test]
    fn hid_service_record_matches_the_pinned_python_fixture_for_each_model() {
        for (model, configuration) in [
            ("pro", TransportConfig::for_model::<Pro>()),
            ("joycon_l", TransportConfig::for_model::<JoyConL>()),
            ("joycon_r", TransportConfig::for_model::<JoyConR>()),
        ] {
            let mut channel = HidSdpChannel::new(&configuration.hid_service, 1024);
            let (record, rounds) = read_complete_record(&mut channel, u16::MAX);

            assert_eq!(rounds, 1, "{model} should fit the large peer MTU");
            assert_eq!(record, expected_record(model));
        }
    }

    #[test]
    fn small_peer_mtu_continuation_is_owned_by_each_sdp_channel() {
        let configuration = TransportConfig::for_model::<Pro>();
        let mut baseline = HidSdpChannel::new(&configuration.hid_service, 48);
        let first_request = request_bytes(1, u16::MAX, vec![0]);
        let first_response = round_trip(&mut baseline, &first_request);
        let continuation_state = continuation_state(&first_response);
        assert_ne!(continuation_state, [0], "the response must be chunked");
        let expected_second = round_trip(
            &mut baseline,
            &request_bytes(2, u16::MAX, continuation_state.clone()),
        );

        let mut first_channel = HidSdpChannel::new(&configuration.hid_service, 48);
        let mut second_channel = HidSdpChannel::new(&configuration.hid_service, 48);
        assert_eq!(
            round_trip(&mut first_channel, &first_request),
            first_response
        );
        assert_eq!(
            round_trip(&mut second_channel, &first_request),
            first_response
        );

        // A fresh request resets only the first channel's continuation cursor.
        round_trip(&mut first_channel, &request_bytes(3, u16::MAX, vec![0]));
        assert_eq!(
            round_trip(
                &mut second_channel,
                &request_bytes(2, u16::MAX, continuation_state),
            ),
            expected_second
        );

        let mut complete = HidSdpChannel::new(&configuration.hid_service, 48);
        let (record, rounds) = read_complete_record(&mut complete, 19);
        assert!(rounds > 1, "small peer MTU must exercise continuation");
        assert_eq!(record, expected_record("pro"));
    }

    #[test]
    fn malformed_or_truncated_request_is_rejected_without_panicking() {
        let configuration = TransportConfig::for_model::<Pro>();
        let mut channel = HidSdpChannel::new(&configuration.hid_service, 48);

        assert_eq!(channel.handle_sdu(&[]), None);
        assert_eq!(channel.handle_sdu(&[0x06, 0x12]), None);

        for malformed in [
            vec![0x06, 0x12, 0x34],
            vec![0x06, 0x12, 0x34, 0x00, 0x01],
            vec![0x06, 0x12, 0x34, 0x00, 0x00, 0x00],
            vec![0xFF, 0x12, 0x34, 0x00, 0x00],
        ] {
            let response = channel
                .handle_sdu(&malformed)
                .expect("a transaction id permits an SDP error response");
            assert_eq!(
                SdpPdu::from_bytes(&response).expect("valid SDP error response"),
                SdpPdu::ErrorResponse {
                    transaction_id: 0x1234,
                    error_code: error_code::INVALID_REQUEST_SYNTAX,
                }
            );
        }
    }

    fn read_complete_record(
        channel: &mut HidSdpChannel,
        maximum_attribute_byte_count: u16,
    ) -> (Vec<ServiceAttribute>, usize) {
        let mut continuation_state = vec![0];
        let mut serialized_records = Vec::new();
        let mut rounds = 0usize;

        loop {
            rounds += 1;
            assert!(rounds < 256, "continuation did not terminate");
            let response = round_trip(
                channel,
                &request_bytes(
                    rounds as u16,
                    maximum_attribute_byte_count,
                    continuation_state,
                ),
            );
            let SdpPdu::ServiceSearchAttributeResponse {
                attribute_lists,
                continuation_state: next,
                ..
            } = response
            else {
                panic!("expected ServiceSearchAttributeResponse");
            };
            serialized_records.extend_from_slice(&attribute_lists);
            continuation_state = next;
            if continuation_state == [0] {
                break;
            }
        }

        let DataElement::Sequence(mut records) =
            DataElement::from_bytes(&serialized_records).expect("complete SDP record sequence")
        else {
            panic!("expected outer SDP record sequence");
        };
        assert_eq!(records.len(), 1, "one HID service must match");
        let DataElement::Sequence(attributes) = records.remove(0) else {
            panic!("expected HID service attribute sequence");
        };
        (
            ServiceAttribute::list_from_data_elements(&attributes),
            rounds,
        )
    }

    fn request_bytes(
        transaction_id: u16,
        maximum_attribute_byte_count: u16,
        continuation_state: Vec<u8>,
    ) -> Vec<u8> {
        SdpPdu::ServiceSearchAttributeRequest {
            transaction_id,
            service_search_pattern: DataElement::sequence([DataElement::uuid(uuid16(0x1124))]),
            maximum_attribute_byte_count,
            attribute_id_list: DataElement::sequence([DataElement::unsigned_integer_32(
                0x0000_FFFF,
            )]),
            continuation_state,
        }
        .to_bytes()
        .expect("valid service search attribute request")
    }

    fn round_trip(channel: &mut HidSdpChannel, request: &[u8]) -> SdpPdu {
        let response = channel
            .handle_sdu(request)
            .expect("well-formed request must have a response");
        SdpPdu::from_bytes(&response).expect("well-formed response")
    }

    fn continuation_state(response: &SdpPdu) -> Vec<u8> {
        let SdpPdu::ServiceSearchAttributeResponse {
            continuation_state, ..
        } = response
        else {
            panic!("expected ServiceSearchAttributeResponse");
        };
        continuation_state.clone()
    }

    fn expected_record(model: &str) -> Vec<ServiceAttribute> {
        let fixture: Value = serde_json::from_str(FIXTURE).expect("valid HID fixture");
        let descriptor = decode_hex(
            fixture["descriptor"]["hex"]
                .as_str()
                .expect("descriptor hex"),
        );
        let policy = &fixture["models"][model]["sdp_policy"];

        let mut attributes = vec![
            attribute(
                0x0000,
                DataElement::unsigned_integer_32(HID_SERVICE_RECORD_HANDLE),
            ),
            attribute(
                0x0001,
                DataElement::sequence([DataElement::uuid(uuid16(0x1124))]),
            ),
            attribute(
                0x0004,
                DataElement::sequence([
                    DataElement::sequence([
                        DataElement::uuid(uuid16(0x0100)),
                        DataElement::unsigned_integer_16(0x0011),
                    ]),
                    DataElement::sequence([DataElement::uuid(uuid16(0x0011))]),
                ]),
            ),
            attribute(
                0x0005,
                DataElement::sequence([DataElement::uuid(uuid16(0x1002))]),
            ),
            attribute(
                0x0006,
                DataElement::sequence([
                    DataElement::unsigned_integer_16(0x656E),
                    DataElement::unsigned_integer_16(0x006A),
                    DataElement::unsigned_integer_16(0x0100),
                ]),
            ),
            attribute(
                0x0009,
                DataElement::sequence([DataElement::sequence([
                    DataElement::uuid(uuid16(0x1124)),
                    DataElement::unsigned_integer_16(json_u16(policy, "bluetooth_profile_version")),
                ])]),
            ),
            attribute(
                0x000D,
                DataElement::sequence([DataElement::sequence([
                    DataElement::sequence([
                        DataElement::uuid(uuid16(0x0100)),
                        DataElement::unsigned_integer_16(0x0013),
                    ]),
                    DataElement::sequence([DataElement::uuid(uuid16(0x0011))]),
                ])]),
            ),
            attribute(
                0x0100,
                DataElement::text_string(json_string(policy, "service_name")),
            ),
            attribute(
                0x0201,
                DataElement::unsigned_integer_16(json_u16(policy, "parser_version")),
            ),
            attribute(
                0x0202,
                DataElement::unsigned_integer_8(json_u8(policy, "device_subclass")),
            ),
            attribute(
                0x0203,
                DataElement::unsigned_integer_8(json_u8(policy, "country_code")),
            ),
            attribute(
                0x0204,
                DataElement::boolean(json_bool(policy, "virtual_cable")),
            ),
            attribute(
                0x0205,
                DataElement::boolean(json_bool(policy, "reconnect_initiate")),
            ),
            attribute(
                0x0206,
                DataElement::sequence([DataElement::sequence([
                    DataElement::unsigned_integer_8(0x22),
                    DataElement::text_string(descriptor),
                ])]),
            ),
            attribute(
                0x0207,
                DataElement::sequence([DataElement::sequence([
                    DataElement::unsigned_integer_16(0x0409),
                    DataElement::unsigned_integer_16(0x0100),
                ])]),
            ),
            attribute(
                0x020B,
                DataElement::unsigned_integer_16(json_u16(policy, "profile_version")),
            ),
            attribute(
                0x020C,
                DataElement::unsigned_integer_16(json_u16(policy, "supervision_timeout")),
            ),
            attribute(
                0x020D,
                DataElement::boolean(json_bool(policy, "normally_connectable")),
            ),
            attribute(
                0x020E,
                DataElement::boolean(json_bool(policy, "boot_device")),
            ),
            attribute(
                0x020F,
                DataElement::unsigned_integer_16(json_u16(policy, "ssr_host_max_latency")),
            ),
            attribute(
                0x0210,
                DataElement::unsigned_integer_16(json_u16(policy, "ssr_host_min_timeout")),
            ),
        ];
        for (id, field) in [(0x0101, "service_description"), (0x0102, "provider_name")] {
            if let Some(value) = policy[field].as_str() {
                attributes.push(attribute(id, DataElement::text_string(value)));
            }
        }
        if let Some(value) = policy["device_release_number"].as_u64() {
            attributes.push(attribute(
                0x0200,
                DataElement::unsigned_integer_16(value as u16),
            ));
        }
        if let Some(value) = policy["remote_wake"].as_bool() {
            attributes.push(attribute(0x020A, DataElement::boolean(value)));
        }
        attributes.sort_by_key(|attribute| attribute.id);
        attributes
    }

    fn attribute(id: u16, value: DataElement) -> ServiceAttribute {
        ServiceAttribute::new(id, value)
    }

    fn uuid16(value: u16) -> Uuid {
        Uuid::from_16_bits(value)
    }

    fn json_string<'a>(value: &'a Value, field: &str) -> &'a str {
        value[field].as_str().expect("fixture string")
    }

    fn json_u16(value: &Value, field: &str) -> u16 {
        value[field].as_u64().expect("fixture u16") as u16
    }

    fn json_u8(value: &Value, field: &str) -> u8 {
        value[field].as_u64().expect("fixture u8") as u8
    }

    fn json_bool(value: &Value, field: &str) -> bool {
        value[field].as_bool().expect("fixture boolean")
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).expect("fixture hex is ASCII");
                u8::from_str_radix(pair, 16).expect("fixture hex byte")
            })
            .collect()
    }
}
