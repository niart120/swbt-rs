use std::{collections::HashMap, sync::Arc, time::Duration};

use swbt_bumble_backend as backend;

use crate::{adapter::AdapterSelector, profile::ProfileIdentity};

use super::profile_key_store::ProfileKeyStoreFactory;
use super::{
    ActivityNotifier, HidChannel, SendAcceptance, TransportCapabilities, TransportConfig,
    TransportError, TransportErrorKind, TransportEvent, TransportPort, TransportResult,
};

pub(super) trait BackendSessionPort: Send {
    fn capabilities(&self) -> TransportResult<TransportCapabilities>;
    fn start_pairing(&mut self) -> TransportResult<()>;
    fn start_reconnect(&mut self) -> TransportResult<()>;
    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>>;
    fn interrupt_send_capacity_available(&self) -> bool;
    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<()>;
    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()>;
    fn disconnect(&mut self) -> TransportResult<()>;
    fn close(&mut self) -> TransportResult<()>;
}

struct BackendSessionAdapter(backend::Session);

impl BackendSessionPort for BackendSessionAdapter {
    fn capabilities(&self) -> TransportResult<TransportCapabilities> {
        capabilities_from_backend(self.0.capabilities())
    }

    fn start_pairing(&mut self) -> TransportResult<()> {
        self.0.start_pairing().map_err(map_backend_error)
    }

    fn start_reconnect(&mut self) -> TransportResult<()> {
        self.0.start_reconnect().map_err(map_backend_error)
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.0
            .poll(timeout)
            .map_err(map_backend_error)?
            .into_iter()
            .map(map_backend_event)
            .collect()
    }

    fn interrupt_send_capacity_available(&self) -> bool {
        self.0.interrupt_send_capacity_available()
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<()> {
        self.0.send_interrupt(payload).map_err(map_backend_error)
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        self.0.drain_interrupt(timeout).map_err(map_backend_error)
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        self.0.disconnect().map_err(map_backend_error)
    }

    fn close(&mut self) -> TransportResult<()> {
        self.0.close().map_err(map_backend_error)
    }
}

pub(super) trait BackendOpener: Send {
    fn open(
        &mut self,
        options: backend::OpenOptions,
        bonds: Box<dyn backend::BondStore>,
    ) -> TransportResult<Box<dyn BackendSessionPort>>;
}

struct SystemBackendOpener;

impl BackendOpener for SystemBackendOpener {
    fn open(
        &mut self,
        options: backend::OpenOptions,
        bonds: Box<dyn backend::BondStore>,
    ) -> TransportResult<Box<dyn BackendSessionPort>> {
        backend::Session::open(options, bonds)
            .map(|session| Box::new(BackendSessionAdapter(session)) as Box<dyn BackendSessionPort>)
            .map_err(map_backend_error)
    }
}

/// Controller-worker transport backed by one owned `swbt-bumble-backend` session.
pub(crate) struct BumbleTransportPort {
    selector: AdapterSelector,
    config: TransportConfig,
    identity: ProfileIdentity,
    profile_key_store: Option<ProfileKeyStoreFactory>,
    opener: Box<dyn BackendOpener>,
    session: Option<Box<dyn BackendSessionPort>>,
}

impl BumbleTransportPort {
    pub(crate) fn with_profile_key_store(
        selector: AdapterSelector,
        config: TransportConfig,
        identity: ProfileIdentity,
        profile_key_store: Option<ProfileKeyStoreFactory>,
    ) -> Self {
        Self {
            selector,
            config,
            identity,
            profile_key_store,
            opener: Box::new(SystemBackendOpener),
            session: None,
        }
    }

    #[cfg(test)]
    pub(super) fn with_opener_for_test(
        selector: AdapterSelector,
        config: TransportConfig,
        identity: ProfileIdentity,
        profile_key_store: Option<ProfileKeyStoreFactory>,
        opener: Box<dyn BackendOpener>,
    ) -> Self {
        Self {
            selector,
            config,
            identity,
            profile_key_store,
            opener,
            session: None,
        }
    }
}

impl TransportPort for BumbleTransportPort {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        if let Some(session) = self.session.as_ref() {
            return session.capabilities();
        }

        let options = backend_open_options(&self.selector, &self.config, self.identity, activity)?;
        let bonds: Box<dyn backend::BondStore> = self.profile_key_store.as_ref().map_or_else(
            || Box::new(VolatileBondStore::default()) as _,
            |factory| Box::new(factory.create()) as _,
        );
        let session = self.opener.open(options, bonds)?;
        let capabilities = session.capabilities()?;
        self.session = Some(session);
        Ok(capabilities)
    }

    fn start_pairing(&mut self) -> TransportResult<()> {
        self.session_mut()?.start_pairing()
    }

    fn start_reconnect(&mut self) -> TransportResult<()> {
        self.session_mut()?.start_reconnect()
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.session_mut()?.poll(timeout)
    }

    fn interrupt_send_capacity_available(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|session| session.interrupt_send_capacity_available())
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.session_mut()?.send_interrupt(payload)?;
        Ok(SendAcceptance::ACCEPTED)
    }

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session.drain_interrupt(timeout)
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        let Some(session) = self.session.as_mut() else {
            return Ok(());
        };
        session.disconnect()
    }

    fn close(&mut self) -> TransportResult<()> {
        let Some(mut session) = self.session.take() else {
            return Ok(());
        };
        session.close()
    }
}

impl BumbleTransportPort {
    fn session_mut(&mut self) -> TransportResult<&mut (dyn BackendSessionPort + '_)> {
        match self.session.as_mut() {
            Some(session) => Ok(session.as_mut()),
            None => Err(TransportError::new(TransportErrorKind::Closed)),
        }
    }
}

impl Drop for BumbleTransportPort {
    fn drop(&mut self) {
        if let Some(mut session) = self.session.take() {
            let _ = session.close();
        }
    }
}

fn backend_open_options(
    selector: &AdapterSelector,
    config: &TransportConfig,
    identity: ProfileIdentity,
    activity: ActivityNotifier,
) -> TransportResult<backend::OpenOptions> {
    let source_policy = &config.hid_service.sdp_policy;
    let sdp_policy = backend::HidSdpPolicy {
        service_name: source_policy.service_name.clone(),
        service_description: source_policy.service_description.clone(),
        provider_name: source_policy.provider_name.clone(),
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
    };
    let hid_service =
        backend::HidServiceConfig::new(config.hid_service.report_descriptor.clone(), sdp_policy);
    let session_config =
        backend::SessionConfig::new(config.local_name(), config.class_of_device(), hid_service)
            .map_err(map_backend_error)?;
    let backend_activity = backend::ActivityNotifier::new(move || activity.notify());
    let options = backend::OpenOptions::new(
        backend::AdapterSelector::from(selector.as_str()),
        session_config,
        backend_activity,
    );
    Ok(options.with_local_identity(map_profile_identity(identity)))
}

fn map_profile_identity(identity: ProfileIdentity) -> backend::LocalIdentity {
    match identity {
        ProfileIdentity::AdapterDefault => backend::LocalIdentity::AdapterDefault,
        ProfileIdentity::LocalAddress(address) => {
            let display = address.octets();
            backend::LocalIdentity::Explicit(backend::BluetoothAddress::from_le_bytes(
                [
                    display[5], display[4], display[3], display[2], display[1], display[0],
                ],
                backend::AddressKind::Public,
            ))
        }
    }
}

fn capabilities_from_backend(
    capabilities: &backend::Capabilities,
) -> TransportResult<TransportCapabilities> {
    let address = capabilities.local_address();
    let bytes = address.as_le_bytes();
    TransportCapabilities::from_validated_classic_controller([
        bytes[5], bytes[4], bytes[3], bytes[2], bytes[1], bytes[0],
    ])
}

pub(super) fn map_backend_event(event: backend::Event) -> TransportResult<TransportEvent> {
    match event {
        backend::Event::Connected { .. } => Ok(TransportEvent::Connected),
        backend::Event::ChannelOpened { channel } => Ok(TransportEvent::HidChannelOpened {
            channel: map_backend_channel(channel),
        }),
        backend::Event::HidOutput { channel, payload } => Ok(TransportEvent::HidOutput {
            channel: map_backend_channel(channel),
            payload,
        }),
        backend::Event::Disconnected { reason } => Ok(TransportEvent::Disconnected { reason }),
        _ => Err(TransportError::new(TransportErrorKind::SourceTerminated)),
    }
}

const fn map_backend_channel(channel: backend::Channel) -> HidChannel {
    match channel {
        backend::Channel::Control => HidChannel::Control,
        backend::Channel::Interrupt => HidChannel::Interrupt,
    }
}

pub(super) const fn map_backend_error_kind(kind: backend::ErrorKind) -> TransportErrorKind {
    match kind {
        backend::ErrorKind::InvalidConfiguration | backend::ErrorKind::OpenFailed => {
            TransportErrorKind::OpenFailed
        }
        backend::ErrorKind::InvalidControllerIdentity => {
            TransportErrorKind::InvalidControllerIdentity
        }
        backend::ErrorKind::IdentityMismatch => TransportErrorKind::IdentityMismatch,
        backend::ErrorKind::AdapterIdentityRecoveryRequired => {
            TransportErrorKind::AdapterIdentityRecoveryRequired
        }
        backend::ErrorKind::UnsupportedController => TransportErrorKind::UnsupportedController,
        backend::ErrorKind::InvalidBondStore => TransportErrorKind::InvalidKeyStore,
        backend::ErrorKind::NoBond => TransportErrorKind::NoBond,
        backend::ErrorKind::Closed => TransportErrorKind::Closed,
        backend::ErrorKind::SendRejected => TransportErrorKind::SendRejected,
        backend::ErrorKind::DrainTimedOut => TransportErrorKind::DrainTimedOut,
        backend::ErrorKind::EventQueueOverflow => TransportErrorKind::EventQueueOverflow,
        backend::ErrorKind::SourceTerminated | backend::ErrorKind::ProtocolViolation => {
            TransportErrorKind::SourceTerminated
        }
        backend::ErrorKind::CloseFailed => TransportErrorKind::CloseFailed,
        _ => TransportErrorKind::SourceTerminated,
    }
}

fn map_backend_error(error: backend::Error) -> TransportError {
    TransportError::with_source(map_backend_error_kind(error.kind()), Arc::new(error))
}

#[derive(Default)]
struct VolatileBondStore {
    bonds: HashMap<backend::BluetoothAddress, backend::ClassicBond>,
}

impl backend::BondStore for VolatileBondStore {
    fn load(
        &self,
        peer: backend::BluetoothAddress,
    ) -> Result<Option<backend::ClassicBond>, backend::BondStoreError> {
        Ok(self.bonds.get(&peer).cloned())
    }

    fn load_all(
        &self,
    ) -> Result<Vec<(backend::BluetoothAddress, backend::ClassicBond)>, backend::BondStoreError>
    {
        Ok(self
            .bonds
            .iter()
            .map(|(peer, bond)| (*peer, bond.clone()))
            .collect())
    }

    fn upsert(
        &mut self,
        peer: backend::BluetoothAddress,
        bond: backend::ClassicBond,
    ) -> Result<(), backend::BondStoreError> {
        self.bonds.clear();
        self.bonds.insert(peer, bond);
        Ok(())
    }
}
