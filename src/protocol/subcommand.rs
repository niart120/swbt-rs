//! Pure Nintendo Switch subcommand reply preparation.

use crate::{
    input::InputState,
    model::{ButtonKind, ControllerModel},
};

use super::{error::ProtocolError, input_report::encode_0x30, output_report::SubcommandRequest};

const SUBCOMMAND_REPLY_SIZE: usize = 50;
const SUBCOMMAND_REPLY_DATA_SIZE: usize = 35;
const SUBCOMMAND_REPLY_ID: u8 = 0x21;
const DEVICE_INFO_SUBCOMMAND: u8 = 0x02;
const TRIGGER_ELAPSED_SUBCOMMAND: u8 = 0x04;
const SIMPLE_ACK_SUBCOMMAND: u8 = 0x08;
const MCU_CONFIG_SUBCOMMAND: u8 = 0x21;
const DEVICE_INFO_ACK: u8 = 0x82;
const TRIGGER_ELAPSED_ACK: u8 = 0x83;
const SIMPLE_ACK: u8 = 0x80;
const MCU_CONFIG_ACK: u8 = 0xA0;
const DEVICE_INFO_FIRMWARE: [u8; 2] = [0x04, 0x00];
const DEVICE_INFO_MARKER: u8 = 0x02;
const TRIGGER_ELAPSED_TICKS: [u8; 2] = 300_u16.to_le_bytes();
const MCU_CONFIG_DATA: [u8; 34] = [
    0x01, 0x00, 0xFF, 0x00, 0x08, 0x00, 0x1B, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0xC8,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeviceInfoBluetoothAddress([u8; 6]);

impl DeviceInfoBluetoothAddress {
    #[must_use]
    pub(crate) const fn from_wire_bytes(bytes: [u8; 6]) -> Self {
        Self(bytes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedSubcommandReply {
    bytes: [u8; SUBCOMMAND_REPLY_SIZE],
    next_timer: u8,
}

impl PreparedSubcommandReply {
    #[must_use]
    pub(crate) const fn bytes(&self) -> &[u8; SUBCOMMAND_REPLY_SIZE] {
        &self.bytes
    }

    #[must_use]
    pub(crate) const fn next_timer(self) -> u8 {
        self.next_timer
    }
}

pub(crate) fn try_prepare_stateless_reply<M: ControllerModel>(
    request: SubcommandRequest<'_>,
    state: &InputState<M>,
    timer: u8,
    device_info_address: DeviceInfoBluetoothAddress,
) -> Result<Option<PreparedSubcommandReply>, ProtocolError> {
    let reply = match request.id() {
        DEVICE_INFO_SUBCOMMAND => prepare_0x21(
            request.id(),
            DEVICE_INFO_ACK,
            &device_info_data::<M>(device_info_address),
            state,
            timer,
        )?,
        TRIGGER_ELAPSED_SUBCOMMAND => prepare_0x21(
            request.id(),
            TRIGGER_ELAPSED_ACK,
            &trigger_elapsed_data::<M>()?,
            state,
            timer,
        )?,
        SIMPLE_ACK_SUBCOMMAND => prepare_0x21(request.id(), SIMPLE_ACK, &[], state, timer)?,
        MCU_CONFIG_SUBCOMMAND => {
            prepare_0x21(request.id(), MCU_CONFIG_ACK, &MCU_CONFIG_DATA, state, timer)?
        }
        _ => return Ok(None),
    };
    Ok(Some(reply))
}

pub(crate) fn prepare_0x21<M: ControllerModel>(
    subcommand_id: u8,
    ack: u8,
    data: &[u8],
    state: &InputState<M>,
    timer: u8,
) -> Result<PreparedSubcommandReply, ProtocolError> {
    if data.len() > SUBCOMMAND_REPLY_DATA_SIZE {
        return Err(ProtocolError::SubcommandReplyDataTooLarge {
            size: data.len(),
            maximum: SUBCOMMAND_REPLY_DATA_SIZE,
        });
    }

    let input = encode_0x30(state, timer, &[0; 36]);
    let mut bytes = [0; SUBCOMMAND_REPLY_SIZE];
    bytes[..13].copy_from_slice(&input.bytes()[..13]);
    bytes[0] = SUBCOMMAND_REPLY_ID;
    bytes[13] = ack;
    bytes[14] = subcommand_id;
    bytes[15..15 + data.len()].copy_from_slice(data);
    Ok(PreparedSubcommandReply {
        bytes,
        next_timer: input.next_timer(),
    })
}

fn device_info_data<M: ControllerModel>(
    device_info_address: DeviceInfoBluetoothAddress,
) -> [u8; 12] {
    let protocol = M::SPEC.protocol;
    let mut data = [0; 12];
    data[..2].copy_from_slice(&DEVICE_INFO_FIRMWARE);
    data[2] = protocol.device_type;
    data[3] = DEVICE_INFO_MARKER;
    data[4..10].copy_from_slice(&device_info_address.0);
    data[10..].copy_from_slice(&protocol.device_info_tail);
    data
}

fn trigger_elapsed_data<M: ControllerModel>() -> Result<[u8; 14], ProtocolError> {
    let mut data = [0; 14];
    for &button in M::SPEC.protocol.pairing_trigger_buttons {
        let offset = trigger_elapsed_offset(button)
            .ok_or(ProtocolError::UnsupportedTriggerElapsedButton { button })?;
        data[offset..offset + 2].copy_from_slice(&TRIGGER_ELAPSED_TICKS);
    }
    Ok(data)
}

const fn trigger_elapsed_offset(button: ButtonKind) -> Option<usize> {
    match button {
        ButtonKind::L => Some(0),
        ButtonKind::R => Some(2),
        ButtonKind::ZL => Some(4),
        ButtonKind::ZR => Some(6),
        ButtonKind::SL => Some(8),
        ButtonKind::SR => Some(10),
        ButtonKind::Home => Some(12),
        _ => None,
    }
}
