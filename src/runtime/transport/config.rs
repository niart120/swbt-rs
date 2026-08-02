use crate::model::ControllerModel;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HidServiceConfig {
    pub(super) report_descriptor: Box<[u8]>,
    pub(super) sdp_policy: HidSdpPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HidSdpPolicy {
    pub(super) service_name: Box<str>,
    pub(super) service_description: Option<Box<str>>,
    pub(super) provider_name: Option<Box<str>>,
    pub(super) device_release_number: Option<u16>,
    pub(super) bluetooth_profile_version: u16,
    pub(super) parser_version: u16,
    pub(super) device_subclass: u8,
    pub(super) country_code: u8,
    pub(super) virtual_cable: bool,
    pub(super) reconnect_initiate: bool,
    pub(super) remote_wake: Option<bool>,
    pub(super) profile_version: u16,
    pub(super) supervision_timeout: u16,
    pub(super) normally_connectable: bool,
    pub(super) boot_device: bool,
    pub(super) ssr_host_max_latency: u16,
    pub(super) ssr_host_min_timeout: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TransportConfig {
    local_name: Box<str>,
    class_of_device: u32,
    pub(super) hid_service: HidServiceConfig,
}

impl TransportConfig {
    pub(crate) fn for_model<M: ControllerModel>() -> Self {
        let protocol = M::SPEC.protocol;
        let source_policy = protocol.hid_sdp_policy;

        Self {
            local_name: protocol.local_name.into(),
            class_of_device: protocol.class_of_device,
            hid_service: HidServiceConfig {
                report_descriptor: protocol.hid_report_descriptor.into(),
                sdp_policy: HidSdpPolicy {
                    service_name: source_policy
                        .service_name
                        .unwrap_or(protocol.local_name)
                        .into(),
                    service_description: source_policy.service_description.map(Into::into),
                    provider_name: source_policy.provider_name.map(Into::into),
                    device_release_number: source_policy.device_release_number,
                    bluetooth_profile_version: source_policy.bluetooth_profile_version,
                    parser_version: source_policy.parser_version,
                    device_subclass: source_policy.device_subclass,
                    country_code: source_policy.country_code,
                    virtual_cable: source_policy.virtual_cable,
                    reconnect_initiate: source_policy.reconnect_initiate,
                    remote_wake: source_policy.remote_wake,
                    profile_version: source_policy.profile_version,
                    supervision_timeout: source_policy.supervision_timeout,
                    normally_connectable: source_policy.normally_connectable,
                    boot_device: source_policy.boot_device,
                    ssr_host_max_latency: source_policy.ssr_host_max_latency,
                    ssr_host_min_timeout: source_policy.ssr_host_min_timeout,
                },
            },
        }
    }

    pub(crate) fn local_name(&self) -> &str {
        &self.local_name
    }

    pub(crate) const fn class_of_device(&self) -> u32 {
        self.class_of_device
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use crate::model::{JoyConL, JoyConR, Pro};

    use super::TransportConfig;

    const FIXTURE: &str =
        include_str!("../../../tests/fixtures/python-v0.6.0/hid/hid-service-fixtures.json");

    #[test]
    fn model_hid_service_projection_matches_the_pinned_python_fixture() {
        let fixture: Value =
            serde_json::from_str(FIXTURE).expect("valid generated HID service fixture");
        let descriptor = decode_hex(
            fixture["descriptor"]["hex"]
                .as_str()
                .expect("descriptor hex"),
        );
        let configurations = [
            ("pro", TransportConfig::for_model::<Pro>()),
            ("joycon_l", TransportConfig::for_model::<JoyConL>()),
            ("joycon_r", TransportConfig::for_model::<JoyConR>()),
        ];

        for (model, configuration) in configurations {
            let expected = &fixture["models"][model]["sdp_policy"];
            let service = &configuration.hid_service;
            let policy = &service.sdp_policy;

            assert_eq!(service.report_descriptor.as_ref(), descriptor);
            assert_eq!(
                policy.service_name.as_ref(),
                expected["service_name"].as_str().expect("service name")
            );
            assert_eq!(
                policy.service_description.as_deref(),
                expected["service_description"].as_str()
            );
            assert_eq!(
                policy.provider_name.as_deref(),
                expected["provider_name"].as_str()
            );
            assert_eq!(
                policy.device_release_number,
                expected["device_release_number"]
                    .as_u64()
                    .map(|value| value as u16)
            );
            assert_eq!(
                u64::from(policy.bluetooth_profile_version),
                expected["bluetooth_profile_version"]
            );
            assert_eq!(u64::from(policy.parser_version), expected["parser_version"]);
            assert_eq!(
                u64::from(policy.device_subclass),
                expected["device_subclass"]
            );
            assert_eq!(u64::from(policy.country_code), expected["country_code"]);
            assert_eq!(policy.virtual_cable, expected["virtual_cable"]);
            assert_eq!(policy.reconnect_initiate, expected["reconnect_initiate"]);
            assert_eq!(policy.remote_wake, expected["remote_wake"].as_bool());
            assert_eq!(
                u64::from(policy.profile_version),
                expected["profile_version"]
            );
            assert_eq!(
                u64::from(policy.supervision_timeout),
                expected["supervision_timeout"]
            );
            assert_eq!(
                policy.normally_connectable,
                expected["normally_connectable"]
            );
            assert_eq!(policy.boot_device, expected["boot_device"]);
            assert_eq!(
                u64::from(policy.ssr_host_max_latency),
                expected["ssr_host_max_latency"]
            );
            assert_eq!(
                u64::from(policy.ssr_host_min_timeout),
                expected["ssr_host_min_timeout"]
            );
        }
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "fixture hex must contain byte pairs");
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
