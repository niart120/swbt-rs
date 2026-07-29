use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bumble_hci::{Command, ReturnParameters};
use bumble_host::{Device, DeviceConfiguration};
use bumble_transport::{
    CommandResponse, Error as BumbleError, ExternalControllerInfo, ExternalHost,
    ExternalHostActivity, SplitOpenedTransport, open_split_transport,
};

use crate::adapter::AdapterSelector;

use super::{
    ActivityNotifier, ClassicAclBufferInfo, ControllerVersionInfo, TransportCapabilities,
    TransportConfig, TransportError, TransportErrorKind, TransportEvent, TransportResult,
    UsbTransportMetadata,
};

const CONTROLLER_ID: usize = 0;
const HCI_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const LOCAL_NAME_LENGTH: usize = 248;

pub(super) trait SplitTransportOpener {
    fn open_split(&mut self, selector: &str) -> bumble_transport::Result<SplitOpenedTransport>;
}

struct SystemSplitTransportOpener;

impl SplitTransportOpener for SystemSplitTransportOpener {
    fn open_split(&mut self, selector: &str) -> bumble_transport::Result<SplitOpenedTransport> {
        open_split_transport(selector)
    }
}

/// A synchronously initialized Bumble host/device pair with an owned reader.
///
/// T06 owns reader notification, terminal state, shutdown, and join here. T07
/// installs this session in the public controller lifecycle.
#[allow(
    dead_code,
    reason = "T06 adds reader cancellation/join before T07 installs this session in a port"
)]
pub(super) struct BumbleSession {
    runtime: Option<BumbleRuntime>,
    capabilities: TransportCapabilities,
    terminal: Option<TransportError>,
}

struct BumbleRuntime {
    host: ExternalHost,
    device: Device,
}

#[allow(
    dead_code,
    reason = "T07 installs the T06 session lifecycle in a concrete TransportPort"
)]
impl BumbleSession {
    pub(super) const fn capabilities(&self) -> TransportCapabilities {
        self.capabilities
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

        let mut wait = timeout;
        let terminal = loop {
            match runtime
                .host
                .wait_for_device_activity(&mut runtime.device, wait)
            {
                Ok(ExternalHostActivity::Packet) => {
                    while runtime.device.poll(&mut runtime.host) {}
                    wait = Duration::ZERO;
                }
                Ok(ExternalHostActivity::Timeout) => break None,
                Ok(ExternalHostActivity::Ended) => break Some(None),
                Err(error) => break Some(Some(error)),
            }
        };

        match terminal {
            None => Ok(Vec::new()),
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

#[allow(
    dead_code,
    reason = "T07 uses the system opener after T06 makes the reader lifecycle cancellable"
)]
pub(super) fn initialize_bumble_session(
    selector: &AdapterSelector,
    config: &TransportConfig,
    activity: ActivityNotifier,
) -> TransportResult<BumbleSession> {
    initialize_bumble_session_with(&mut SystemSplitTransportOpener, selector, config, activity)
}

pub(super) fn initialize_bumble_session_with<O>(
    opener: &mut O,
    selector: &AdapterSelector,
    config: &TransportConfig,
    activity: ActivityNotifier,
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
    let mut host = ExternalHost::new_with_activity_callback(split, move || activity.notify());
    let mut device = Device::from_config(CONTROLLER_ID, device_configuration(config))
        .map_err(map_open_source)?;
    let controller = host
        .initialize_device(&mut device, HCI_COMMAND_TIMEOUT)
        .map_err(map_bumble_error)?;
    let local_address = read_local_address(&mut host)?;
    let capabilities = controller_capabilities(&controller, &device, local_address, usb)?;

    for command in identity_commands(config) {
        send_successful_command_complete(&mut host, command)?;
    }

    Ok(BumbleSession {
        runtime: Some(BumbleRuntime { host, device }),
        capabilities,
        terminal: None,
    })
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
    let response = host
        .send_command(Command::ReadBdAddr, HCI_COMMAND_TIMEOUT)
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

fn identity_commands(config: &TransportConfig) -> [Command; 5] {
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
