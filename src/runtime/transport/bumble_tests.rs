use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::json;
use swbt_bumble_backend as backend;

use crate::{
    adapter::AdapterSelector,
    model::Pro,
    profile::{LocalAddress, ProfileIdentity},
};

use super::bumble::{
    BackendOpener, BackendSessionPort, BumbleTransportPort, map_backend_error_kind,
    map_backend_event,
};
use super::profile_key_store::ProfileKeyStoreFactory;
use super::{
    HidChannel, SendAcceptance, TransportCapabilities, TransportConfig, TransportErrorKind,
    TransportEvent, TransportPort, TransportResult, activity_channel,
};

const LOCAL_NAMESPACE: &str = "00:1B:DC:F9:9F:7D";
const PROFILE_PEER: &str = "11:22:33:44:55:66";

#[test]
fn backend_error_kinds_preserve_the_transport_contract() {
    let cases = [
        (
            backend::ErrorKind::InvalidConfiguration,
            TransportErrorKind::OpenFailed,
        ),
        (
            backend::ErrorKind::OpenFailed,
            TransportErrorKind::OpenFailed,
        ),
        (
            backend::ErrorKind::InvalidControllerIdentity,
            TransportErrorKind::InvalidControllerIdentity,
        ),
        (
            backend::ErrorKind::IdentityMismatch,
            TransportErrorKind::IdentityMismatch,
        ),
        (
            backend::ErrorKind::AdapterIdentityRecoveryRequired,
            TransportErrorKind::AdapterIdentityRecoveryRequired,
        ),
        (
            backend::ErrorKind::UnsupportedController,
            TransportErrorKind::UnsupportedController,
        ),
        (
            backend::ErrorKind::InvalidBondStore,
            TransportErrorKind::InvalidKeyStore,
        ),
        (backend::ErrorKind::NoBond, TransportErrorKind::NoBond),
        (backend::ErrorKind::Closed, TransportErrorKind::Closed),
        (
            backend::ErrorKind::SendRejected,
            TransportErrorKind::SendRejected,
        ),
        (
            backend::ErrorKind::DrainTimedOut,
            TransportErrorKind::DrainTimedOut,
        ),
        (
            backend::ErrorKind::EventQueueOverflow,
            TransportErrorKind::EventQueueOverflow,
        ),
        (
            backend::ErrorKind::SourceTerminated,
            TransportErrorKind::SourceTerminated,
        ),
        (
            backend::ErrorKind::CloseFailed,
            TransportErrorKind::CloseFailed,
        ),
        (
            backend::ErrorKind::ProtocolViolation,
            TransportErrorKind::SourceTerminated,
        ),
    ];

    for (backend, expected) in cases {
        assert_eq!(map_backend_error_kind(backend), expected);
    }
}

#[test]
fn backend_events_preserve_the_transport_contract() {
    let peer = backend_address(PROFILE_PEER);
    let cases = [
        (
            backend::Event::Connected { peer },
            TransportEvent::Connected,
        ),
        (
            backend::Event::ChannelOpened {
                channel: backend::Channel::Control,
            },
            TransportEvent::HidChannelOpened {
                channel: HidChannel::Control,
            },
        ),
        (
            backend::Event::ChannelOpened {
                channel: backend::Channel::Interrupt,
            },
            TransportEvent::HidChannelOpened {
                channel: HidChannel::Interrupt,
            },
        ),
        (
            backend::Event::HidOutput {
                channel: backend::Channel::Interrupt,
                payload: Box::new([0xA2, 0x01]),
            },
            TransportEvent::HidOutput {
                channel: HidChannel::Interrupt,
                payload: Box::new([0xA2, 0x01]),
            },
        ),
        (
            backend::Event::Disconnected { reason: Some(0x13) },
            TransportEvent::Disconnected { reason: Some(0x13) },
        ),
    ];

    for (backend, expected) in cases {
        assert_eq!(
            map_backend_event(backend).expect("map backend event"),
            expected
        );
    }
}

#[test]
fn open_projects_config_identity_and_profile_store_into_the_backend() {
    let temp = TempDirectory::new("open-profile");
    let profile_path = temp.path().join("pro.json");
    fs::write(&profile_path, profile_bytes()).expect("write test profile");
    let state = Arc::new(Mutex::new(FakeState::default()));
    let explicit = LocalAddress::parse("02:12:34:56:78:9A").expect("valid local address");
    let config = TransportConfig::for_model::<Pro>();
    let expected_config = config.clone();
    let mut transport = BumbleTransportPort::with_opener_for_test(
        AdapterSelector::from("usb:0a12:0001"),
        config,
        ProfileIdentity::LocalAddress(explicit),
        Some(ProfileKeyStoreFactory::for_model::<Pro>(profile_path)),
        Box::new(FakeOpener {
            state: Arc::clone(&state),
            expected_config,
        }),
    );

    let capabilities = transport
        .open(activity_channel().0)
        .expect("open fake backend session");
    let repeated = transport
        .open(activity_channel().0)
        .expect("reuse fake backend session");

    assert_eq!(capabilities, TransportCapabilities::test_default());
    assert_eq!(repeated, capabilities);
    let state = state.lock().expect("read fake state");
    assert_eq!(state.open_count, 1);
    assert!(state.selector_matches);
    assert!(state.config_matches);
    assert_eq!(
        state.explicit_identity_le,
        Some([0x9A, 0x78, 0x56, 0x34, 0x12, 0x02])
    );
    assert_eq!(
        state.loaded_bond,
        Some(backend::ClassicBond::new([0xA1; 16], 4, true))
    );
}

#[test]
fn session_operations_delegate_once_and_close_is_idempotent() {
    let state = Arc::new(Mutex::new(FakeState::default()));
    let mut transport = BumbleTransportPort::with_opener_for_test(
        AdapterSelector::from("usb:0"),
        TransportConfig::for_model::<Pro>(),
        ProfileIdentity::AdapterDefault,
        None,
        Box::new(FakeOpener {
            state: Arc::clone(&state),
            expected_config: TransportConfig::for_model::<Pro>(),
        }),
    );

    assert_eq!(
        transport.start_pairing().expect_err("not open").kind(),
        TransportErrorKind::Closed
    );
    assert_eq!(
        transport
            .send_interrupt(&[0x01])
            .expect_err("not open")
            .kind(),
        TransportErrorKind::Closed
    );
    assert!(!transport.interrupt_send_capacity_available());
    transport
        .drain_interrupt(Duration::ZERO)
        .expect("pre-open drain is settled");
    transport
        .disconnect()
        .expect("pre-open disconnect is settled");
    transport.close().expect("pre-open close is settled");

    transport
        .open(activity_channel().0)
        .expect("open fake backend session");
    assert!(transport.interrupt_send_capacity_available());
    transport.start_pairing().expect("start pairing");
    transport.start_reconnect().expect("start reconnect");
    assert_eq!(
        transport.poll(Duration::from_millis(5)).expect("poll"),
        [TransportEvent::Connected]
    );
    assert_eq!(
        transport.send_interrupt(&[0x31, 0x02]).expect("send"),
        SendAcceptance::ACCEPTED
    );
    transport
        .drain_interrupt(Duration::from_millis(7))
        .expect("drain");
    transport.disconnect().expect("disconnect");
    transport.close().expect("close");
    transport.close().expect("repeated close");

    let state = state.lock().expect("read fake state");
    assert_eq!(state.start_pairing_count, 1);
    assert_eq!(state.start_reconnect_count, 1);
    assert_eq!(state.poll_timeouts, [Duration::from_millis(5)]);
    assert_eq!(state.sent_payloads, [vec![0x31, 0x02]]);
    assert_eq!(state.drain_timeouts, [Duration::from_millis(7)]);
    assert_eq!(state.disconnect_count, 1);
    assert_eq!(state.close_count, 1);
}

struct FakeOpener {
    state: Arc<Mutex<FakeState>>,
    expected_config: TransportConfig,
}

impl BackendOpener for FakeOpener {
    fn open(
        &mut self,
        options: backend::OpenOptions,
        mut bonds: Box<dyn backend::BondStore>,
    ) -> TransportResult<Box<dyn BackendSessionPort>> {
        let local_address = backend_address(LOCAL_NAMESPACE);
        bonds
            .select_local_address(local_address)
            .expect("select profile namespace as backend initialization does");
        let loaded_bond = bonds
            .load(backend_address(PROFILE_PEER))
            .expect("load profile bond");
        let expected = &self.expected_config;
        let actual = options.config();
        let actual_policy = actual.hid_service().sdp_policy();
        let expected_policy = &expected.hid_service.sdp_policy;
        let config_matches = actual.local_name() == expected.local_name()
            && actual.class_of_device() == expected.class_of_device()
            && actual.complete_local_name_eir() == expected.complete_local_name_ad()
            && actual.hid_service().report_descriptor()
                == expected.hid_service.report_descriptor.as_ref()
            && actual_policy.service_name == expected_policy.service_name
            && actual_policy.service_description == expected_policy.service_description
            && actual_policy.provider_name == expected_policy.provider_name
            && actual_policy.device_release_number == expected_policy.device_release_number
            && actual_policy.bluetooth_profile_version == expected_policy.bluetooth_profile_version
            && actual_policy.parser_version == expected_policy.parser_version
            && actual_policy.device_subclass == expected_policy.device_subclass
            && actual_policy.country_code == expected_policy.country_code
            && actual_policy.virtual_cable == expected_policy.virtual_cable
            && actual_policy.reconnect_initiate == expected_policy.reconnect_initiate
            && actual_policy.remote_wake == expected_policy.remote_wake
            && actual_policy.profile_version == expected_policy.profile_version
            && actual_policy.supervision_timeout == expected_policy.supervision_timeout
            && actual_policy.normally_connectable == expected_policy.normally_connectable
            && actual_policy.boot_device == expected_policy.boot_device
            && actual_policy.ssr_host_max_latency == expected_policy.ssr_host_max_latency
            && actual_policy.ssr_host_min_timeout == expected_policy.ssr_host_min_timeout;
        let explicit_identity_le = match options.local_identity() {
            backend::LocalIdentity::AdapterDefault => None,
            backend::LocalIdentity::Explicit(address) => Some(*address.as_le_bytes()),
        };

        let mut state = self.state.lock().expect("update fake state");
        state.open_count += 1;
        state.selector_matches = options.adapter()
            == &backend::AdapterSelector::from("usb:0a12:0001")
            || options.adapter() == &backend::AdapterSelector::from("usb:0");
        state.config_matches = config_matches;
        state.explicit_identity_le = explicit_identity_le;
        state.loaded_bond = loaded_bond;
        drop(state);

        Ok(Box::new(FakeSession {
            state: Arc::clone(&self.state),
            capabilities: TransportCapabilities::test_default(),
        }))
    }
}

struct FakeSession {
    state: Arc<Mutex<FakeState>>,
    capabilities: TransportCapabilities,
}

impl BackendSessionPort for FakeSession {
    fn capabilities(&self) -> TransportResult<TransportCapabilities> {
        Ok(self.capabilities)
    }

    fn start_pairing(&mut self) -> TransportResult<()> {
        self.state
            .lock()
            .expect("update fake state")
            .start_pairing_count += 1;
        Ok(())
    }

    fn start_reconnect(&mut self) -> TransportResult<()> {
        self.state
            .lock()
            .expect("update fake state")
            .start_reconnect_count += 1;
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.state
            .lock()
            .expect("update fake state")
            .poll_timeouts
            .push(timeout);
        Ok(vec![TransportEvent::Connected])
    }

    fn interrupt_send_capacity_available(&self) -> bool {
        true
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<()> {
        self.state
            .lock()
            .expect("update fake state")
            .sent_payloads
            .push(payload.to_vec());
        Ok(())
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        self.state
            .lock()
            .expect("update fake state")
            .drain_timeouts
            .push(timeout);
        Ok(())
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        self.state
            .lock()
            .expect("update fake state")
            .disconnect_count += 1;
        Ok(())
    }

    fn close(&mut self) -> TransportResult<()> {
        self.state.lock().expect("update fake state").close_count += 1;
        Ok(())
    }
}

#[derive(Default)]
struct FakeState {
    open_count: usize,
    selector_matches: bool,
    config_matches: bool,
    explicit_identity_le: Option<[u8; 6]>,
    loaded_bond: Option<backend::ClassicBond>,
    start_pairing_count: usize,
    start_reconnect_count: usize,
    poll_timeouts: Vec<Duration>,
    sent_payloads: Vec<Vec<u8>>,
    drain_timeouts: Vec<Duration>,
    disconnect_count: usize,
    close_count: usize,
}

fn backend_address(value: &str) -> backend::BluetoothAddress {
    backend::BluetoothAddress::parse(value, backend::AddressKind::Public)
        .expect("valid test address")
}

fn profile_bytes() -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "format": "swbt.profile",
        "schema_version": 2,
        "controller_kind": "pro",
        "identity": {
            "kind": "exp-local-address",
            "address": "02:12:34:56:78:9A"
        },
        "key_store": {
            "namespaces": {
                LOCAL_NAMESPACE: {
                    PROFILE_PEER: {
                        "address_type": 0,
                        "link_key": {
                            "authenticated": true,
                            "value": "A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1A1"
                        },
                        "link_key_type": 4
                    }
                }
            }
        }
    }))
    .expect("serialize test profile")
}

struct TempDirectory {
    path: PathBuf,
}

impl TempDirectory {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "swbt-rs-bumble-backend-{label}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.path).expect("remove test directory");
    }
}
