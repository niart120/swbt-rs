use std::fmt;

use super::{TransportError, TransportErrorKind, TransportResult};

const BR_EDR_NOT_SUPPORTED_MASK: u8 = 0x20;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps Bumble initialization metadata into this transport-neutral snapshot"
    )
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ControllerVersionInfo {
    hci_version: u8,
    hci_subversion: u16,
    lmp_version: u8,
    company_identifier: u16,
    lmp_subversion: u16,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps Bumble initialization metadata into this transport-neutral snapshot"
    )
)]
impl ControllerVersionInfo {
    pub(crate) const fn new(
        hci_version: u8,
        hci_subversion: u16,
        lmp_version: u8,
        company_identifier: u16,
        lmp_subversion: u16,
    ) -> Self {
        Self {
            hci_version,
            hci_subversion,
            lmp_version,
            company_identifier,
            lmp_subversion,
        }
    }

    pub(crate) const fn hci_version(self) -> u8 {
        self.hci_version
    }

    pub(crate) const fn hci_subversion(self) -> u16 {
        self.hci_subversion
    }

    pub(crate) const fn lmp_version(self) -> u8 {
        self.lmp_version
    }

    pub(crate) const fn company_identifier(self) -> u16 {
        self.company_identifier
    }

    pub(crate) const fn lmp_subversion(self) -> u16 {
        self.lmp_subversion
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps Bumble Classic ACL buffer metadata into capabilities"
    )
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClassicAclBufferInfo {
    packet_length: u16,
    packet_count: u16,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps Bumble Classic ACL buffer metadata into capabilities"
    )
)]
impl ClassicAclBufferInfo {
    pub(crate) const fn new(packet_length: u16, packet_count: u16) -> Self {
        Self {
            packet_length,
            packet_count,
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps Bumble split-transport metadata into capabilities"
    )
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct UsbTransportMetadata {
    vendor_id: u16,
    product_id: u16,
    bus_number: u8,
    device_address: u8,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps Bumble split-transport metadata into capabilities"
    )
)]
impl UsbTransportMetadata {
    pub(crate) const fn new(
        vendor_id: u16,
        product_id: u16,
        bus_number: u8,
        device_address: u8,
    ) -> Self {
        Self {
            vendor_id,
            product_id,
            bus_number,
            device_address,
        }
    }

    pub(crate) const fn vendor_id(self) -> u16 {
        self.vendor_id
    }

    pub(crate) const fn product_id(self) -> u16 {
        self.product_id
    }

    pub(crate) const fn bus_number(self) -> u8 {
        self.bus_number
    }

    pub(crate) const fn device_address(self) -> u8 {
        self.device_address
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransportCapabilities {
    local_address: [u8; 6],
    local_version: Option<ControllerVersionInfo>,
    classic_capable: bool,
    usb: UsbTransportMetadata,
}

impl TransportCapabilities {
    #[cfg(feature = "bumble")]
    pub(crate) fn from_validated_classic_controller(
        local_address: [u8; 6],
        local_version: ControllerVersionInfo,
        usb: UsbTransportMetadata,
    ) -> TransportResult<Self> {
        if local_address == [0; 6] {
            return Err(TransportError::new(
                TransportErrorKind::InvalidControllerIdentity,
            ));
        }

        Ok(Self {
            local_address,
            local_version: Some(local_version),
            classic_capable: true,
            usb,
        })
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T05 constructs capabilities after Bumble initialization"
        )
    )]
    pub(crate) fn from_initialized_controller(
        local_address: [u8; 6],
        local_version: Option<ControllerVersionInfo>,
        lmp_feature_page_0: Option<[u8; 8]>,
        classic_acl: Option<ClassicAclBufferInfo>,
        usb: UsbTransportMetadata,
    ) -> TransportResult<Self> {
        if local_address == [0; 6] {
            return Err(TransportError::new(
                TransportErrorKind::InvalidControllerIdentity,
            ));
        }
        let classic_capable =
            lmp_feature_page_0
                .zip(classic_acl)
                .is_some_and(|(features, classic_acl)| {
                    features[4] & BR_EDR_NOT_SUPPORTED_MASK == 0
                        && classic_acl.packet_length > 0
                        && classic_acl.packet_count > 0
                });

        Ok(Self {
            local_address,
            local_version,
            classic_capable,
            usb,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        Self::from_initialized_controller(
            [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d],
            Some(ControllerVersionInfo::new(
                0x09, 0x1234, 0x09, 0x000a, 0x5678,
            )),
            Some([0; 8]),
            Some(ClassicAclBufferInfo::new(1021, 8)),
            UsbTransportMetadata::new(0x0a12, 0x0001, 1, 7),
        )
        .expect("test controller capabilities are valid")
    }

    pub(crate) const fn local_address(self) -> [u8; 6] {
        self.local_address
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T08 emits controller version diagnostics from initialized capabilities"
        )
    )]
    pub(crate) const fn local_version(self) -> Option<ControllerVersionInfo> {
        self.local_version
    }

    pub(crate) const fn classic_capable(self) -> bool {
        self.classic_capable
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T08 emits adapter diagnostics from initialized capabilities"
        )
    )]
    pub(crate) const fn usb(self) -> UsbTransportMetadata {
        self.usb
    }
}

impl fmt::Debug for TransportCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportCapabilities")
            .field("local_version", &self.local_version)
            .field("classic_capable", &self.classic_capable)
            .field("usb", &self.usb)
            .finish_non_exhaustive()
    }
}
