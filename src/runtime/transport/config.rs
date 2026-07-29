use crate::model::ControllerModel;

const EXTENDED_INQUIRY_RESPONSE_LEN: usize = 240;
const COMPLETE_LOCAL_NAME_DATA_TYPE: u8 = 0x09;

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps the complete transport projection into Bumble configuration and HCI commands"
    )
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TransportConfig {
    local_name: Box<str>,
    class_of_device: u32,
    extended_inquiry_response: [u8; EXTENDED_INQUIRY_RESPONSE_LEN],
    classic_enabled: bool,
    classic_accept_any: bool,
    connectable: bool,
    discoverable: bool,
    classic_sc_enabled: bool,
    classic_ssp_enabled: bool,
    le_enabled: bool,
    le_simultaneous_enabled: bool,
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T05 maps the complete transport projection into Bumble configuration and HCI commands"
    )
)]
impl TransportConfig {
    pub(crate) fn for_model<M: ControllerModel>() -> Self {
        let protocol = M::SPEC.protocol;
        let local_name = protocol.local_name.as_bytes();
        assert!(
            !local_name.is_empty() && local_name.len() <= EXTENDED_INQUIRY_RESPONSE_LEN - 2,
            "model local name must fit one complete-local-name EIR structure"
        );
        assert!(
            protocol.class_of_device <= 0x00ff_ffff,
            "model Class of Device must fit 24 bits"
        );

        let mut extended_inquiry_response = [0; EXTENDED_INQUIRY_RESPONSE_LEN];
        extended_inquiry_response[0] = (local_name.len() + 1) as u8;
        extended_inquiry_response[1] = COMPLETE_LOCAL_NAME_DATA_TYPE;
        extended_inquiry_response[2..2 + local_name.len()].copy_from_slice(local_name);

        Self {
            local_name: protocol.local_name.into(),
            class_of_device: protocol.class_of_device,
            extended_inquiry_response,
            classic_enabled: true,
            classic_accept_any: false,
            connectable: false,
            discoverable: false,
            classic_sc_enabled: false,
            classic_ssp_enabled: true,
            le_enabled: false,
            le_simultaneous_enabled: false,
        }
    }

    pub(crate) fn local_name(&self) -> &str {
        &self.local_name
    }

    pub(crate) const fn class_of_device(&self) -> u32 {
        self.class_of_device
    }

    pub(crate) const fn extended_inquiry_response(&self) -> &[u8; EXTENDED_INQUIRY_RESPONSE_LEN] {
        &self.extended_inquiry_response
    }

    pub(crate) fn complete_local_name_ad(&self) -> &[u8] {
        let total_length = usize::from(self.extended_inquiry_response[0]) + 1;
        &self.extended_inquiry_response[..total_length]
    }

    pub(crate) const fn classic_enabled(&self) -> bool {
        self.classic_enabled
    }

    pub(crate) const fn classic_accept_any(&self) -> bool {
        self.classic_accept_any
    }

    pub(crate) const fn connectable(&self) -> bool {
        self.connectable
    }

    pub(crate) const fn discoverable(&self) -> bool {
        self.discoverable
    }

    pub(crate) const fn classic_sc_enabled(&self) -> bool {
        self.classic_sc_enabled
    }

    pub(crate) const fn classic_ssp_enabled(&self) -> bool {
        self.classic_ssp_enabled
    }

    pub(crate) const fn le_enabled(&self) -> bool {
        self.le_enabled
    }

    pub(crate) const fn le_simultaneous_enabled(&self) -> bool {
        self.le_simultaneous_enabled
    }
}
