//! Pure IMU wire encoding.

use crate::input::ImuFrame;

const IMU_BLOCK_SIZE: usize = 36;
const IDENTITY_QUATERNION: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImuMode {
    Disabled = 0x00,
    Standard = 0x01,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImuEncodingState {
    orientation: [f64; 4],
    previous_report_ns: Option<u64>,
}

impl ImuEncodingState {
    #[must_use]
    pub(crate) const fn new(orientation: [f64; 4], previous_report_ns: Option<u64>) -> Self {
        Self {
            orientation,
            previous_report_ns,
        }
    }
}

impl Default for ImuEncodingState {
    fn default() -> Self {
        Self::new(IDENTITY_QUATERNION, None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ImuEncodingResult {
    block: [u8; IMU_BLOCK_SIZE],
    next_state: ImuEncodingState,
}

impl ImuEncodingResult {
    #[must_use]
    pub(crate) const fn block(&self) -> &[u8; IMU_BLOCK_SIZE] {
        &self.block
    }

    #[must_use]
    pub(crate) const fn next_state(self) -> ImuEncodingState {
        self.next_state
    }
}

#[must_use]
pub(crate) fn encode_imu_block(
    _current: ImuEncodingState,
    mode: ImuMode,
    frames: &[ImuFrame; 3],
    _now_ns: u64,
) -> ImuEncodingResult {
    let block = match mode {
        ImuMode::Disabled => [0; IMU_BLOCK_SIZE],
        ImuMode::Standard => encode_standard_imu(frames),
    };
    ImuEncodingResult {
        block,
        next_state: ImuEncodingState::default(),
    }
}

fn encode_standard_imu(frames: &[ImuFrame; 3]) -> [u8; IMU_BLOCK_SIZE] {
    let mut block = [0; IMU_BLOCK_SIZE];
    for (frame_index, frame) in frames.iter().enumerate() {
        let accel = frame.accel();
        let gyro = frame.gyro();
        let values = [accel[0], accel[1], accel[2], gyro[0], gyro[1], gyro[2]];
        for (value_index, value) in values.into_iter().enumerate() {
            let offset = frame_index * 12 + value_index * 2;
            block[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
    }
    block
}
