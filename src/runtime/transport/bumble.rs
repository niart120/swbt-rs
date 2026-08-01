use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bumble::{Address, AddressType};
use bumble_hci::{Command, ReturnParameters};
use bumble_host::{Device, DeviceConfiguration};
use bumble_transport::{
    CommandResponse, Error as BumbleError, ExternalControllerInfo, ExternalHost,
    ExternalHostActivity, SplitOpenedTransport, open_split_transport,
};

use crate::{adapter::AdapterSelector, profile::ProfileIdentity};

use super::classic::ClassicDeviceSession;
use super::csr::{CsrVendorCommand, matches_csr_vendor_response};
use super::identity::{
    AdapterIdentityBackend, AdapterIdentityPreparation, AdapterIdentitySession,
    IdentityPreparationError, IdentityPreparationErrorKind, IdentityPreparationOptions,
    prepare_adapter_identity,
};
use super::profile_key_store::ProfileKeyStoreFactory;
use super::{
    ActivityNotifier, ClassicAclBufferInfo, ControllerVersionInfo, SendAcceptance,
    TransportCapabilities, TransportConfig, TransportError, TransportErrorKind, TransportEvent,
    TransportPort, TransportResult, UsbTransportMetadata,
};

const CONTROLLER_ID: usize = 0;
const HCI_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const IDENTITY_REENUMERATION_TIMEOUT: Duration = Duration::from_secs(10);
const IDENTITY_REENUMERATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOCAL_NAME_LENGTH: usize = 248;
// Preserve the reference host's role-switch and sniff-mode policy.
const DEFAULT_CLASSIC_LINK_POLICY_SETTINGS: u16 = 0x0005;

pub(super) trait SplitTransportOpener {
    fn open_split(&mut self, selector: &str) -> bumble_transport::Result<SplitOpenedTransport>;
}

struct SystemSplitTransportOpener;

impl SplitTransportOpener for SystemSplitTransportOpener {
    fn open_split(&mut self, selector: &str) -> bumble_transport::Result<SplitOpenedTransport> {
        open_split_transport(selector)
    }
}

#[cfg(all(test, feature = "adapter-tests"))]
pub(super) fn prepare_target_adapter_identity_for_test(
    selector: &AdapterSelector,
    target: [u8; 6],
) -> Result<AdapterIdentityPreparation, IdentityPreparationError> {
    let mut opener = SystemSplitTransportOpener;
    let mut backend = BumbleIdentityBackend::new(&mut opener, selector);
    prepare_adapter_identity(
        &mut backend,
        target,
        IdentityPreparationOptions {
            response_timeout: HCI_COMMAND_TIMEOUT,
            reenumeration_timeout: IDENTITY_REENUMERATION_TIMEOUT,
            reenumeration_poll_interval: IDENTITY_REENUMERATION_POLL_INTERVAL,
        },
    )
}

/// A synchronously initialized Bumble host/device pair with an owned reader.
///
/// The controller transport owns this value from HCI initialization through
/// reader shutdown and join.
pub(super) struct BumbleSession {
    runtime: Option<BumbleRuntime>,
    capabilities: TransportCapabilities,
    terminal: Option<TransportError>,
}

struct BumbleRuntime {
    host: ExternalHost,
    device: Device,
    classic: ClassicDeviceSession,
}

/// Controller-worker transport that opens and owns one Bumble HCI session.
pub(crate) struct BumbleTransportPort {
    selector: AdapterSelector,
    config: TransportConfig,
    identity: ProfileIdentity,
    profile_key_store: Option<ProfileKeyStoreFactory>,
    session: Option<BumbleSession>,
}

impl BumbleTransportPort {
    #[cfg(test)]
    pub(crate) const fn new(selector: AdapterSelector, config: TransportConfig) -> Self {
        Self {
            selector,
            config,
            identity: ProfileIdentity::AdapterDefault,
            profile_key_store: None,
            session: None,
        }
    }

    pub(crate) const fn with_profile_key_store(
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
            session: None,
        }
    }

    #[cfg(test)]
    pub(super) fn from_session_for_test(session: BumbleSession) -> Self {
        Self {
            selector: AdapterSelector::from("usb:0"),
            config: TransportConfig::for_model::<crate::model::Pro>(),
            identity: ProfileIdentity::AdapterDefault,
            profile_key_store: None,
            session: Some(session),
        }
    }
}

impl TransportPort for BumbleTransportPort {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities> {
        if let Some(session) = self.session.as_ref() {
            return Ok(session.capabilities());
        }
        let session = initialize_bumble_transport(
            &self.selector,
            &self.config,
            self.identity,
            activity,
            self.profile_key_store.as_ref(),
        )?;
        let capabilities = session.capabilities();
        self.session = Some(session);
        Ok(capabilities)
    }

    fn start_pairing(&mut self) -> TransportResult<()> {
        self.session
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?
            .start_pairing()
    }

    fn start_reconnect(&mut self) -> TransportResult<()> {
        self.session
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?
            .start_reconnect()
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        self.session
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?
            .poll(timeout)
    }

    fn interrupt_send_capacity_available(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(BumbleSession::interrupt_send_capacity_available)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        self.session
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?
            .send_interrupt(payload)
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

impl BumbleSession {
    pub(super) const fn capabilities(&self) -> TransportCapabilities {
        self.capabilities
    }

    fn terminal_error(&self) -> Option<TransportError> {
        self.terminal.clone()
    }

    pub(super) fn start_pairing(&mut self) -> TransportResult<()> {
        if let Some(terminal) = self.terminal_error() {
            return Err(terminal);
        }
        let runtime = self
            .runtime
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?;
        runtime
            .classic
            .start_pairing(&mut runtime.device, &mut runtime.host)
    }

    pub(super) fn start_reconnect(&mut self) -> TransportResult<()> {
        if let Some(terminal) = self.terminal_error() {
            return Err(terminal);
        }
        let runtime = self
            .runtime
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?;
        let bonds = runtime
            .device
            .bonds()
            .map_err(|_| TransportError::new(TransportErrorKind::InvalidKeyStore))?;
        let mut classic_peers = bonds.into_iter().filter_map(|(peer, keys)| {
            keys.link_key
                .as_ref()
                .is_some_and(|key| key.value.len() == 16)
                .then_some(peer)
        });
        let Some(peer) = classic_peers.next() else {
            return Err(TransportError::new(TransportErrorKind::NoBond));
        };
        if classic_peers.next().is_some() {
            return Err(TransportError::new(TransportErrorKind::InvalidKeyStore));
        }
        let peer_address = Address::parse(&peer, AddressType::PUBLIC_DEVICE)
            .map_err(|_| TransportError::new(TransportErrorKind::InvalidKeyStore))?;
        runtime
            .classic
            .start_reconnect(&mut runtime.device, &mut runtime.host, peer_address, true)
    }

    pub(super) fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        if let Some(terminal) = self.terminal_error() {
            return Err(terminal);
        }
        let runtime = self
            .runtime
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?;
        runtime
            .classic
            .send_interrupt(&mut runtime.device, &mut runtime.host, payload)
    }

    pub(super) fn interrupt_send_capacity_available(&self) -> bool {
        if self.terminal.is_some() {
            return false;
        }
        self.runtime.as_ref().is_some_and(|runtime| {
            runtime
                .classic
                .interrupt_send_capacity_available(&runtime.device)
        })
    }

    pub(super) fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()> {
        if let Some(terminal) = self.terminal_error() {
            return Err(terminal);
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .unwrap_or_else(Instant::now);
        loop {
            let runtime = self
                .runtime
                .as_mut()
                .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?;
            drive_runtime(runtime)?;
            if runtime.classic.interrupt_output_is_flushed(&runtime.device) {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::new(TransportErrorKind::DrainTimedOut));
            }
            match runtime
                .host
                .wait_for_device_activity(&mut runtime.device, remaining)
            {
                Ok(ExternalHostActivity::Packet) => {}
                Ok(ExternalHostActivity::Timeout) => {
                    return Err(TransportError::new(TransportErrorKind::DrainTimedOut));
                }
                Ok(ExternalHostActivity::Ended) => {
                    return Err(self.record_terminal(None));
                }
                Err(error) => {
                    return Err(self.record_terminal(Some(error)));
                }
            }
        }
    }

    pub(super) fn disconnect(&mut self) -> TransportResult<()> {
        if let Some(terminal) = self.terminal_error() {
            return Err(terminal);
        }
        let runtime = self
            .runtime
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?;
        runtime
            .classic
            .disconnect(&mut runtime.device, &mut runtime.host)
    }

    #[cfg(test)]
    pub(super) const fn device_configuration(&self) -> &DeviceConfiguration {
        &self
            .runtime
            .as_ref()
            .expect("open Bumble session owns its runtime")
            .device
            .config
    }

    pub(super) fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        if let Some(terminal) = self.terminal.as_ref() {
            return Err(terminal.clone());
        }
        let runtime = self
            .runtime
            .as_mut()
            .ok_or_else(|| TransportError::new(TransportErrorKind::Closed))?;
        let mut events = drive_runtime(runtime)?;
        if !events.is_empty() {
            return Ok(events);
        }

        let mut wait = timeout;
        let terminal = loop {
            match runtime
                .host
                .wait_for_device_activity(&mut runtime.device, wait)
            {
                Ok(ExternalHostActivity::Packet) => {
                    events.extend(drive_runtime(runtime)?);
                    if !events.is_empty() {
                        break None;
                    }
                    wait = Duration::ZERO;
                }
                Ok(ExternalHostActivity::Timeout) => break None,
                Ok(ExternalHostActivity::Ended) => break Some(None),
                Err(error) => break Some(Some(error)),
            }
        };

        match terminal {
            None => Ok(events),
            Some(source) => Err(self.record_terminal(source)),
        }
    }

    pub(super) fn close(&mut self) -> TransportResult<()> {
        let Some(mut runtime) = self.runtime.take() else {
            return Ok(());
        };
        let result = runtime
            .host
            .shutdown_reader(HCI_COMMAND_TIMEOUT)
            .map_err(map_close_source);
        drop(runtime);
        result
    }

    fn record_terminal(&mut self, source: Option<BumbleError>) -> TransportError {
        let terminal = match source {
            Some(source) => {
                TransportError::with_source(TransportErrorKind::SourceTerminated, Arc::new(source))
            }
            None => TransportError::new(TransportErrorKind::SourceTerminated),
        };
        self.terminal = Some(terminal.clone());
        terminal
    }
}

fn drive_runtime(runtime: &mut BumbleRuntime) -> TransportResult<Vec<TransportEvent>> {
    while runtime.device.poll(&mut runtime.host) {}
    runtime.classic.poll(&mut runtime.device, &mut runtime.host)
}

fn initialize_bumble_transport(
    selector: &AdapterSelector,
    config: &TransportConfig,
    identity: ProfileIdentity,
    activity: ActivityNotifier,
    profile_key_store: Option<&ProfileKeyStoreFactory>,
) -> TransportResult<BumbleSession> {
    initialize_bumble_transport_with(
        &mut SystemSplitTransportOpener,
        selector,
        config,
        identity,
        activity,
        profile_key_store,
    )
}

pub(super) fn initialize_bumble_transport_with<O>(
    opener: &mut O,
    selector: &AdapterSelector,
    config: &TransportConfig,
    identity: ProfileIdentity,
    activity: ActivityNotifier,
    profile_key_store: Option<&ProfileKeyStoreFactory>,
) -> TransportResult<BumbleSession>
where
    O: SplitTransportOpener,
{
    let preparation = match identity {
        ProfileIdentity::AdapterDefault => None,
        ProfileIdentity::LocalAddress(address) => {
            let mut backend = BumbleIdentityBackend::new(opener, selector);
            Some(
                prepare_adapter_identity(
                    &mut backend,
                    address.octets(),
                    IdentityPreparationOptions {
                        response_timeout: HCI_COMMAND_TIMEOUT,
                        reenumeration_timeout: IDENTITY_REENUMERATION_TIMEOUT,
                        reenumeration_poll_interval: IDENTITY_REENUMERATION_POLL_INTERVAL,
                    },
                )
                .map_err(map_identity_preparation_error)?,
            )
        }
    };
    let expected_address = match identity {
        ProfileIdentity::AdapterDefault => None,
        ProfileIdentity::LocalAddress(address) => Some(address.octets()),
    };
    match initialize_bumble_session_with_profile_and_expected(
        opener,
        selector,
        config,
        activity,
        profile_key_store,
        expected_address,
    ) {
        Err(source)
            if source.kind() == TransportErrorKind::IdentityMismatch
                && preparation == Some(AdapterIdentityPreparation::Rewritten) =>
        {
            Err(TransportError::with_source(
                TransportErrorKind::AdapterIdentityRecoveryRequired,
                Arc::new(source),
            ))
        }
        result => result,
    }
}

#[cfg(test)]
pub(super) fn initialize_bumble_session_with<O>(
    opener: &mut O,
    selector: &AdapterSelector,
    config: &TransportConfig,
    activity: ActivityNotifier,
) -> TransportResult<BumbleSession>
where
    O: SplitTransportOpener,
{
    initialize_bumble_session_with_profile(opener, selector, config, activity, None)
}

#[cfg(test)]
pub(super) fn initialize_bumble_session_with_profile<O>(
    opener: &mut O,
    selector: &AdapterSelector,
    config: &TransportConfig,
    activity: ActivityNotifier,
    profile_key_store: Option<&ProfileKeyStoreFactory>,
) -> TransportResult<BumbleSession>
where
    O: SplitTransportOpener,
{
    initialize_bumble_session_with_profile_and_expected(
        opener,
        selector,
        config,
        activity,
        profile_key_store,
        None,
    )
}

fn initialize_bumble_session_with_profile_and_expected<O>(
    opener: &mut O,
    selector: &AdapterSelector,
    config: &TransportConfig,
    activity: ActivityNotifier,
    profile_key_store: Option<&ProfileKeyStoreFactory>,
    expected_address: Option<[u8; 6]>,
) -> TransportResult<BumbleSession>
where
    O: SplitTransportOpener,
{
    selector
        .parse_usb()
        .map_err(|_| TransportError::new(TransportErrorKind::OpenFailed))?;
    let split = opener
        .open_split(selector.as_str())
        .map_err(map_bumble_error)?;
    let usb = parse_usb_metadata(&split.metadata)?;
    let classic_activity = activity.clone();
    let mut host = ExternalHost::new_with_activity_callback(split, move || activity.notify());
    let mut device = Device::from_config(CONTROLLER_ID, device_configuration(config))
        .map_err(map_open_source)?;
    let controller = host
        .initialize_device(&mut device, HCI_COMMAND_TIMEOUT)
        .map_err(map_bumble_error)?;
    let local_address = read_local_address(&mut host)?;
    if expected_address.is_some_and(|expected| expected != local_address) {
        return Err(TransportError::new(TransportErrorKind::IdentityMismatch));
    }
    let capabilities = controller_capabilities(&controller, &device, local_address, usb)?;
    if let Some(factory) = profile_key_store {
        device.set_key_store(factory.create(local_address));
    }

    for command in identity_commands(config) {
        send_successful_command_complete(&mut host, command)?;
    }
    let mut classic = ClassicDeviceSession::new(config, classic_activity);
    classic.register_servers(&mut device)?;
    trace_initialized_controller(capabilities);

    Ok(BumbleSession {
        runtime: Some(BumbleRuntime {
            host,
            device,
            classic,
        }),
        capabilities,
        terminal: None,
    })
}

fn trace_initialized_controller(capabilities: TransportCapabilities) {
    let version = capabilities.local_version();
    let usb = capabilities.usb();
    tracing::debug!(
        target: "swbt::transport::bumble",
        hci_version = ?version.map(ControllerVersionInfo::hci_version),
        hci_subversion = ?version.map(ControllerVersionInfo::hci_subversion),
        lmp_version = ?version.map(ControllerVersionInfo::lmp_version),
        company_identifier = ?version.map(ControllerVersionInfo::company_identifier),
        lmp_subversion = ?version.map(ControllerVersionInfo::lmp_subversion),
        classic_capable = capabilities.classic_capable(),
        usb_vendor_id = usb.vendor_id(),
        usb_product_id = usb.product_id(),
        usb_bus = usb.bus_number(),
        usb_address = usb.device_address(),
        "initialized Bumble HCI controller",
    );
}

fn device_configuration(config: &TransportConfig) -> DeviceConfiguration {
    DeviceConfiguration {
        name: config.local_name().to_owned(),
        class_of_device: config.class_of_device(),
        advertising_data: config.complete_local_name_ad().to_vec(),
        classic_enabled: config.classic_enabled(),
        classic_accept_any: config.classic_accept_any(),
        connectable: config.connectable(),
        discoverable: config.discoverable(),
        classic_sc_enabled: config.classic_sc_enabled(),
        classic_ssp_enabled: config.classic_ssp_enabled(),
        le_enabled: config.le_enabled(),
        le_simultaneous_enabled: config.le_simultaneous_enabled(),
        ..DeviceConfiguration::default()
    }
}

fn controller_capabilities(
    controller: &ExternalControllerInfo,
    device: &Device,
    local_address: [u8; 6],
    usb: UsbTransportMetadata,
) -> TransportResult<TransportCapabilities> {
    let local_version = controller.local_version.as_ref().map(|version| {
        ControllerVersionInfo::new(
            version.hci_version,
            version.hci_subversion,
            version.lmp_version,
            version.company_identifier,
            version.lmp_subversion,
        )
    });
    let classic_acl = device.classic_acl_buffer().map(|buffer| {
        ClassicAclBufferInfo::new(buffer.data_packet_length, buffer.total_num_data_packets)
    });

    TransportCapabilities::from_initialized_controller(
        local_address,
        local_version,
        controller.local_lmp_features.first().copied(),
        classic_acl,
        usb,
    )
}

fn read_local_address(host: &mut ExternalHost) -> TransportResult<[u8; 6]> {
    read_local_address_with(host, HCI_COMMAND_TIMEOUT)
}

fn read_local_address_with(
    host: &mut ExternalHost,
    response_timeout: Duration,
) -> TransportResult<[u8; 6]> {
    let response = host
        .send_command(Command::ReadBdAddr, response_timeout)
        .map_err(map_bumble_error)?;
    let CommandResponse::Complete {
        return_parameters: ReturnParameters::ReadBdAddr { status: 0, bd_addr },
        ..
    } = response
    else {
        return Err(map_initialization_error(
            InitializationError::InvalidIdentityResponse,
        ));
    };

    let mut local_address = *bd_addr.address_bytes();
    local_address.reverse();
    Ok(local_address)
}

struct BumbleIdentityBackend<'a, O> {
    opener: &'a mut O,
    selector: &'a AdapterSelector,
    origin: Instant,
}

impl<'a, O> BumbleIdentityBackend<'a, O> {
    fn new(opener: &'a mut O, selector: &'a AdapterSelector) -> Self {
        Self {
            opener,
            selector,
            origin: Instant::now(),
        }
    }
}

impl<O> AdapterIdentityBackend for BumbleIdentityBackend<'_, O>
where
    O: SplitTransportOpener,
{
    type Error = TransportError;
    type Session = BumbleIdentitySession;

    fn open(&mut self) -> Result<Self::Session, Self::Error> {
        self.selector
            .parse_usb()
            .map_err(|_| TransportError::new(TransportErrorKind::OpenFailed))?;
        let split = self
            .opener
            .open_split(self.selector.as_str())
            .map_err(map_bumble_error)?;
        Ok(BumbleIdentitySession {
            host: ExternalHost::new(split),
        })
    }

    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

struct BumbleIdentitySession {
    host: ExternalHost,
}

impl AdapterIdentitySession for BumbleIdentitySession {
    type Error = TransportError;

    fn initialize(&mut self, response_timeout: Duration) -> Result<u16, Self::Error> {
        let response = self
            .host
            .send_command(Command::ReadLocalVersionInformation, response_timeout)
            .map_err(map_bumble_error)?;
        let CommandResponse::Complete {
            return_parameters:
                ReturnParameters::ReadLocalVersionInformation {
                    status: 0,
                    company_identifier,
                    ..
                },
            ..
        } = response
        else {
            return Err(map_initialization_error(
                InitializationError::InvalidVersionResponse,
            ));
        };
        Ok(company_identifier)
    }

    fn read_address(&mut self, response_timeout: Duration) -> Result<[u8; 6], Self::Error> {
        read_local_address_with(&mut self.host, response_timeout)
    }

    fn send_vendor_command(
        &mut self,
        command: &CsrVendorCommand,
        response_timeout: Duration,
    ) -> Result<Box<[u8]>, Self::Error> {
        self.host
            .send_vendor_command(csr_command(command), response_timeout, |response| {
                matches_csr_vendor_response(command, response)
            })
            .map(Vec::into_boxed_slice)
            .map_err(map_bumble_error)
    }

    fn send_command_without_response(
        &mut self,
        command: &CsrVendorCommand,
    ) -> Result<(), Self::Error> {
        self.host
            .send_command_without_response(csr_command(command))
            .map_err(map_bumble_error)
    }

    fn close(&mut self) -> Result<(), Self::Error> {
        self.host
            .shutdown_reader(HCI_COMMAND_TIMEOUT)
            .map_err(map_close_source)
    }
}

fn csr_command(command: &CsrVendorCommand) -> Command {
    Command::Generic {
        op_code: command.op_code(),
        parameters: command.parameters().to_vec(),
    }
}

fn map_identity_preparation_error(error: IdentityPreparationError) -> TransportError {
    let kind = match error.kind() {
        IdentityPreparationErrorKind::UnsupportedController => {
            TransportErrorKind::UnsupportedController
        }
        IdentityPreparationErrorKind::FailedBeforeWrite => TransportErrorKind::OpenFailed,
        IdentityPreparationErrorKind::RecoveryRequired => {
            TransportErrorKind::AdapterIdentityRecoveryRequired
        }
    };
    TransportError::with_source(kind, Arc::new(error))
}

fn identity_commands(config: &TransportConfig) -> [Command; 6] {
    let mut local_name = [0; LOCAL_NAME_LENGTH];
    let local_name_bytes = config.local_name().as_bytes();
    local_name[..local_name_bytes.len()].copy_from_slice(local_name_bytes);
    let scan_enable = u8::from(config.discoverable()) | (u8::from(config.connectable()) << 1);

    [
        Command::WriteLocalName { local_name },
        Command::WriteClassOfDevice {
            class_of_device: config.class_of_device(),
        },
        Command::WriteSimplePairingMode {
            simple_pairing_mode: u8::from(config.classic_ssp_enabled()),
        },
        Command::WriteExtendedInquiryResponse {
            fec_required: 0,
            extended_inquiry_response: *config.extended_inquiry_response(),
        },
        Command::WriteDefaultLinkPolicySettings {
            default_link_policy_settings: DEFAULT_CLASSIC_LINK_POLICY_SETTINGS,
        },
        Command::WriteScanEnable { scan_enable },
    ]
}

fn send_successful_command_complete(
    host: &mut ExternalHost,
    command: Command,
) -> TransportResult<()> {
    let opcode = command.op_code();
    let response = host
        .send_command(command, HCI_COMMAND_TIMEOUT)
        .map_err(map_bumble_error)?;
    if response.status() != Some(0) || !matches!(response, CommandResponse::Complete { .. }) {
        return Err(map_initialization_error(
            InitializationError::CommandDidNotComplete { opcode },
        ));
    }
    Ok(())
}

fn parse_usb_metadata(
    metadata: &BTreeMap<String, String>,
) -> TransportResult<UsbTransportMetadata> {
    let vendor_id = parse_hex_u16(metadata, "vendor_id")?;
    let product_id = parse_hex_u16(metadata, "product_id")?;
    let bus_number = parse_decimal_u8(metadata, "bus")?;
    let device_address = parse_decimal_u8(metadata, "address")?;
    Ok(UsbTransportMetadata::new(
        vendor_id,
        product_id,
        bus_number,
        device_address,
    ))
}

fn parse_hex_u16(metadata: &BTreeMap<String, String>, key: &'static str) -> TransportResult<u16> {
    metadata
        .get(key)
        .and_then(|value| u16::from_str_radix(value, 16).ok())
        .ok_or_else(|| map_initialization_error(InitializationError::InvalidUsbMetadata { key }))
}

fn parse_decimal_u8(metadata: &BTreeMap<String, String>, key: &'static str) -> TransportResult<u8> {
    metadata
        .get(key)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| map_initialization_error(InitializationError::InvalidUsbMetadata { key }))
}

fn map_bumble_error(error: BumbleError) -> TransportError {
    map_open_source(error)
}

fn map_close_source(error: BumbleError) -> TransportError {
    TransportError::with_source(TransportErrorKind::CloseFailed, Arc::new(error))
}

fn map_open_source(error: impl StdError + Send + Sync + 'static) -> TransportError {
    TransportError::with_source(TransportErrorKind::OpenFailed, Arc::new(error))
}

fn map_initialization_error(error: InitializationError) -> TransportError {
    map_open_source(error)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitializationError {
    InvalidUsbMetadata { key: &'static str },
    InvalidIdentityResponse,
    InvalidVersionResponse,
    CommandDidNotComplete { opcode: u16 },
}

impl fmt::Display for InitializationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUsbMetadata { key } => {
                write!(formatter, "USB transport metadata field {key} is invalid")
            }
            Self::InvalidIdentityResponse => {
                formatter.write_str("controller returned an invalid identity response")
            }
            Self::InvalidVersionResponse => {
                formatter.write_str("controller returned an invalid version response")
            }
            Self::CommandDidNotComplete { opcode } => {
                write!(
                    formatter,
                    "HCI command {opcode:#06x} did not complete successfully"
                )
            }
        }
    }
}

impl StdError for InitializationError {}
