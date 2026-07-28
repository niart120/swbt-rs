//! Nintendo Switch HID input report encoding.

use crate::{
    input::{InputState, Stick},
    model::{ControllerModel, button_wire_position},
};

const INPUT_REPORT_SIZE: usize = 49;
const STANDARD_FULL_REPORT_ID: u8 = 0x30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PreparedInputReport {
    bytes: [u8; INPUT_REPORT_SIZE],
    next_timer: u8,
}

impl PreparedInputReport {
    #[must_use]
    pub(crate) const fn bytes(&self) -> &[u8; INPUT_REPORT_SIZE] {
        &self.bytes
    }

    #[must_use]
    pub(crate) const fn next_timer(self) -> u8 {
        self.next_timer
    }
}

#[must_use]
pub(crate) fn encode_neutral_0x30<M: ControllerModel>(timer: u8) -> PreparedInputReport {
    encode_0x30(&InputState::<M>::neutral(), timer, &[0; 36])
}

#[must_use]
pub(crate) fn encode_0x30<M: ControllerModel>(
    state: &InputState<M>,
    timer: u8,
    imu_block: &[u8; 36],
) -> PreparedInputReport {
    let mut bytes = [0; INPUT_REPORT_SIZE];
    bytes[0] = STANDARD_FULL_REPORT_ID;
    bytes[1] = timer;
    bytes[2] = M::SPEC.protocol.battery_connection;

    for button in state.buttons() {
        let position = button_wire_position(M::KIND, button.kind())
            .expect("Button<M> must have a wire mapping for M");
        bytes[position.byte_index] |= position.mask;
    }

    let (left_stick, right_stick) = state.wire_sticks();
    bytes[6..9].copy_from_slice(&pack_stick(left_stick));
    bytes[9..12].copy_from_slice(&pack_stick(right_stick));
    bytes[12] = M::SPEC.protocol.vibrator_input;
    bytes[13..].copy_from_slice(imu_block);

    PreparedInputReport {
        bytes,
        next_timer: timer.wrapping_add(1),
    }
}

fn pack_stick(stick: Stick) -> [u8; 3] {
    let (x, y) = stick.axes();
    [
        x as u8,
        ((x >> 8) as u8 & 0x0F) | ((y as u8 & 0x0F) << 4),
        (y >> 4) as u8,
    ]
}
