//! Pure IMU wire encoding.

use crate::{input::ImuFrame, model::SensorCalibration};

const IMU_BLOCK_SIZE: usize = 36;
const QUATERNION_SCALE: f64 = 1_073_741_824.0;
const IDENTITY_QUATERNION: [f64; 4] = [0.0, 0.0, 0.0, 1.0];

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ImuMode {
    Disabled = 0x00,
    Standard = 0x01,
    Quaternion1 = 0x02,
    Quaternion2 = 0x03,
    Quaternion3 = 0x04,
    Quaternion4 = 0x05,
}

impl ImuMode {
    pub(crate) const fn from_wire(value: u8) -> Option<Self> {
        match value {
            0x00 => Some(Self::Disabled),
            0x01 => Some(Self::Standard),
            0x02 => Some(Self::Quaternion1),
            0x03 => Some(Self::Quaternion2),
            0x04 => Some(Self::Quaternion3),
            0x05 => Some(Self::Quaternion4),
            _ => None,
        }
    }
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

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn orientation(self) -> [f64; 4] {
        self.orientation
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn previous_report_ns(self) -> Option<u64> {
        self.previous_report_ns
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
    current: ImuEncodingState,
    mode: ImuMode,
    frames: &[ImuFrame; 3],
    gyro_calibration: SensorCalibration,
    now_ns: u64,
) -> ImuEncodingResult {
    match mode {
        ImuMode::Disabled => ImuEncodingResult {
            block: [0; IMU_BLOCK_SIZE],
            next_state: ImuEncodingState::default(),
        },
        ImuMode::Standard => ImuEncodingResult {
            block: encode_standard_imu(frames),
            next_state: ImuEncodingState::default(),
        },
        ImuMode::Quaternion1
        | ImuMode::Quaternion2
        | ImuMode::Quaternion3
        | ImuMode::Quaternion4 => encode_quaternion_imu(current, frames, gyro_calibration, now_ns),
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

fn encode_quaternion_imu(
    current: ImuEncodingState,
    frames: &[ImuFrame; 3],
    gyro_calibration: SensorCalibration,
    now_ns: u64,
) -> ImuEncodingResult {
    let elapsed_ns = current
        .previous_report_ns
        .map_or(0, |previous| now_ns.saturating_sub(previous));
    let sample_seconds = elapsed_ns as f64 / 1_000_000_000.0 / frames.len() as f64;

    let mut orientation = current.orientation;
    for frame in frames {
        orientation = next_orientation(
            orientation,
            gyro_rates(*frame, gyro_calibration),
            sample_seconds,
        );
    }

    ImuEncodingResult {
        block: pack_mode_2(frames, orientation, now_ns / 1_000_000),
        next_state: ImuEncodingState::new(orientation, Some(now_ns)),
    }
}

fn gyro_rates(frame: ImuFrame, calibration: SensorCalibration) -> [f64; 3] {
    let raw = frame.gyro();
    std::array::from_fn(|axis| {
        let calibrated = i32::from(raw[axis]) - i32::from(calibration.zero_raw[axis]);
        (f64::from(calibrated) * ImuFrame::GYRO_DPS_PER_RAW).to_radians()
    })
}

fn next_orientation(
    orientation: [f64; 4],
    rates_rad_s: [f64; 3],
    elapsed_seconds: f64,
) -> [f64; 4] {
    let rotation = rates_rad_s.map(|rate| rate * elapsed_seconds);
    let magnitude = rotation
        .iter()
        .map(|value| value * value)
        .sum::<f64>()
        .sqrt();
    if magnitude == 0.0 {
        return orientation;
    }

    let half_angle = magnitude / 2.0;
    let vector_scale = half_angle.sin() / magnitude;
    let delta = [
        rotation[0] * vector_scale,
        rotation[1] * vector_scale,
        rotation[2] * vector_scale,
        half_angle.cos(),
    ];
    normalize(hamilton_product(orientation, delta))
}

fn hamilton_product(left: [f64; 4], right: [f64; 4]) -> [f64; 4] {
    let [lx, ly, lz, lw] = left;
    let [rx, ry, rz, rw] = right;
    [
        lw * rx + lx * rw + ly * rz - lz * ry,
        lw * ry + ly * rw + lz * rx - lx * rz,
        lw * rz + lz * rw + lx * ry - ly * rx,
        lw * rw - lx * rx - ly * ry - lz * rz,
    ]
}

fn normalize(value: [f64; 4]) -> [f64; 4] {
    let inverse = 1.0
        / value
            .iter()
            .map(|component| component * component)
            .sum::<f64>()
            .sqrt();
    value.map(|component| component * inverse)
}

fn pack_mode_2(
    frames: &[ImuFrame; 3],
    orientation: [f64; 4],
    timestamp_ms: u64,
) -> [u8; IMU_BLOCK_SIZE] {
    let mut block = [0; IMU_BLOCK_SIZE];
    for (frame_index, frame) in frames.iter().enumerate() {
        for (axis, value) in frame.accel().into_iter().enumerate() {
            let offset = frame_index * 12 + axis * 2;
            block[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
        }
    }

    let mut max_index = 0;
    for index in 1..orientation.len() {
        if orientation[index].abs() > orientation[max_index].abs() {
            max_index = index;
        }
    }
    let sign = if orientation[max_index] < 0.0 {
        -1.0
    } else {
        1.0
    };
    let components = std::array::from_fn::<i64, 3, _>(|index| {
        let scaled = (orientation[(max_index + index + 1) & 3] * sign * QUATERNION_SCALE) as i64;
        scaled >> 10
    });

    put_bits(&mut block, 48, 2, 2);
    put_bits(&mut block, 50, 2, max_index as u64);
    put_bits(&mut block, 52, 21, components[0] as u64);
    put_bits(&mut block, 73, 21, components[1] as u64);
    put_bits(&mut block, 94, 2, components[2] as u64);
    put_bits(&mut block, 144, 19, (components[2] >> 2) as u64);
    put_bits(&mut block, 271, 11, timestamp_ms);
    put_bits(&mut block, 282, 6, frames.len() as u64);
    block
}

fn put_bits(target: &mut [u8; IMU_BLOCK_SIZE], start: usize, width: usize, value: u64) {
    debug_assert!(start + width <= target.len() * 8);
    for bit in 0..width {
        if value & (1 << bit) != 0 {
            let absolute = start + bit;
            target[absolute / 8] |= 1 << (absolute % 8);
        }
    }
}
