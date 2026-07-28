//! Pure composition of parsed output reports and prepared protocol effects.

use crate::{input::InputState, model::ControllerModel, profile::ControllerColors};

use super::{
    error::ProtocolError,
    output_report::{OutputReport, RawRumble, SubcommandRequest, parse_output_report},
    session::ProtocolSession,
    spi::VirtualSpiFlash,
    subcommand::{
        DeviceInfoBluetoothAddress, PreparedSessionReply, PreparedSubcommandReply,
        try_prepare_spi_reply, try_prepare_stateful_reply, try_prepare_stateless_reply,
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PreparedOutputAction {
    Reply(PreparedSubcommandReply),
    SessionReply(PreparedSessionReply),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum OutputPreparation<'a> {
    RumbleOnly {
        packet_id: u8,
        rumble: RawRumble,
    },
    Subcommand {
        packet_id: u8,
        rumble: RawRumble,
        request: SubcommandRequest<'a>,
        outcome: Result<PreparedOutputAction, ProtocolError>,
    },
}

pub(crate) struct SwitchHidProtocol<M: ControllerModel> {
    spi: VirtualSpiFlash<M>,
    device_info_address: DeviceInfoBluetoothAddress,
}

impl<M: ControllerModel> SwitchHidProtocol<M> {
    #[must_use]
    pub(crate) fn new(
        colors: Option<ControllerColors>,
        device_info_address: DeviceInfoBluetoothAddress,
    ) -> Self {
        Self {
            spi: VirtualSpiFlash::new(colors),
            device_info_address,
        }
    }

    pub(crate) fn prepare_output_report<'a>(
        &self,
        raw: &'a [u8],
        state: &InputState<M>,
        timer: u8,
        current_session: ProtocolSession,
    ) -> Result<OutputPreparation<'a>, ProtocolError> {
        match parse_output_report(raw)? {
            OutputReport::Rumble {
                packet_id, rumble, ..
            } => Ok(OutputPreparation::RumbleOnly { packet_id, rumble }),
            OutputReport::Subcommand {
                packet_id,
                rumble,
                request,
                ..
            } => {
                let outcome = self.prepare_subcommand(request, state, timer, current_session);
                Ok(OutputPreparation::Subcommand {
                    packet_id,
                    rumble,
                    request,
                    outcome,
                })
            }
        }
    }

    fn prepare_subcommand(
        &self,
        request: SubcommandRequest<'_>,
        state: &InputState<M>,
        timer: u8,
        current_session: ProtocolSession,
    ) -> Result<PreparedOutputAction, ProtocolError> {
        if let Some(reply) =
            try_prepare_stateless_reply(request, state, timer, self.device_info_address)?
        {
            return Ok(PreparedOutputAction::Reply(reply));
        }
        if let Some(reply) = try_prepare_spi_reply(request, state, timer, &self.spi)? {
            return Ok(PreparedOutputAction::Reply(reply));
        }
        if let Some(reply) = try_prepare_stateful_reply(request, state, timer, current_session)? {
            return Ok(PreparedOutputAction::SessionReply(reply));
        }
        Err(ProtocolError::UnsupportedSubcommand {
            subcommand_id: request.id(),
        })
    }
}
