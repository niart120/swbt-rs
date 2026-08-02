//! Pure composition of parsed output reports and prepared protocol effects.

use crate::{input::InputState, model::ControllerModel, profile::ControllerColors};

use super::{
    error::ProtocolError,
    imu::{ImuEncodingState, encode_imu_block},
    input_report::{PreparedInputReport, encode_0x30},
    output_report::SubcommandRequest,
    session::ProtocolSession,
    spi::VirtualSpiFlash,
    subcommand::{
        DeviceInfoBluetoothAddress, PreparedSessionReply, PreparedSubcommandReply,
        try_prepare_spi_reply, try_prepare_stateful_reply, try_prepare_stateless_reply,
    },
};

#[cfg(test)]
use super::output_report::{OutputReport, RawRumble, parse_output_report};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputPreparation {
    report: PreparedInputReport,
    next_imu_encoding_state: ImuEncodingState,
}

impl InputPreparation {
    #[must_use]
    pub const fn bytes(&self) -> &[u8; 49] {
        self.report.bytes()
    }

    #[must_use]
    pub const fn next_timer(self) -> u8 {
        self.report.next_timer()
    }

    #[must_use]
    pub const fn next_imu_encoding_state(self) -> ImuEncodingState {
        self.next_imu_encoding_state
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreparedOutputAction {
    Reply(PreparedSubcommandReply),
    SessionReply(PreparedSessionReply),
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg(test)]
pub enum OutputPreparation<'a> {
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

pub struct SwitchHidProtocol<M: ControllerModel> {
    spi: VirtualSpiFlash<M>,
    device_info_address: DeviceInfoBluetoothAddress,
}

impl<M: ControllerModel> SwitchHidProtocol<M> {
    #[must_use]
    pub fn new(colors: Option<ControllerColors>, device_info_address: [u8; 6]) -> Self {
        Self {
            spi: VirtualSpiFlash::new(colors),
            device_info_address: DeviceInfoBluetoothAddress::from_wire_bytes(device_info_address),
        }
    }

    #[must_use]
    pub fn prepare_input_report(
        &self,
        state: &InputState<M>,
        timer: u8,
        current_session: ProtocolSession,
        now_ns: u64,
    ) -> InputPreparation {
        let imu = encode_imu_block(
            current_session.imu_encoding_state(),
            current_session.imu_mode(),
            state.imu_frames(),
            M::SPEC.protocol.gyroscope_calibration,
            now_ns,
        );
        InputPreparation {
            report: encode_0x30(state, timer, imu.block()),
            next_imu_encoding_state: imu.next_state(),
        }
    }

    #[cfg(test)]
    pub fn prepare_output_report<'a>(
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

    pub fn prepare_subcommand(
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
