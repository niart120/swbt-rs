//! Read-only virtual SPI flash.

use std::marker::PhantomData;

use crate::{
    model::{ControllerModel, SensorCalibration},
    profile::ControllerColors,
};

use super::error::ProtocolError;

const ADDRESS_LIMIT: u32 = 0x80000;
pub(crate) const MAX_READ_SIZE: usize = 0x1D;
const ERASED_BYTE: u8 = 0xFF;
const DEVICE_TYPE_ADDRESS: u32 = 0x6012;
const COLOR_INFO_EXISTS_ADDRESS: u32 = 0x601B;
const FACTORY_ACCELEROMETER_CALIBRATION_ADDRESS: u32 = 0x6020;
const FACTORY_ACCELEROMETER_CALIBRATION_END: u32 = 0x602B;
const FACTORY_GYROSCOPE_CALIBRATION_ADDRESS: u32 = 0x602C;
const FACTORY_GYROSCOPE_CALIBRATION_END: u32 = 0x6037;
const CONTROLLER_COLORS_ADDRESS: u32 = 0x6050;
const CONTROLLER_COLORS_END: u32 = 0x605B;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SpiRead {
    bytes: [u8; MAX_READ_SIZE],
    len: usize,
}

impl SpiRead {
    #[must_use]
    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

pub(crate) struct VirtualSpiFlash<M: ControllerModel> {
    colors: ControllerColors,
    model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> VirtualSpiFlash<M> {
    #[must_use]
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 and T33 protocol construction initializes virtual SPI state"
        )
    )]
    pub(crate) fn new(colors: Option<ControllerColors>) -> Self {
        Self {
            colors: colors.unwrap_or(M::SPEC.protocol.default_colors),
            model: PhantomData,
        }
    }

    pub(crate) fn read(&self, address: u32, size: usize) -> Result<SpiRead, ProtocolError> {
        if size > MAX_READ_SIZE {
            return Err(ProtocolError::SpiReadTooLarge {
                size,
                maximum: MAX_READ_SIZE,
            });
        }

        let Some(end) = address.checked_add(size as u32) else {
            return Err(ProtocolError::SpiAddressOutOfRange { address, size });
        };
        if end > ADDRESS_LIMIT {
            return Err(ProtocolError::SpiAddressOutOfRange { address, size });
        }

        let mut bytes = [ERASED_BYTE; MAX_READ_SIZE];
        for (offset, byte) in bytes[..size].iter_mut().enumerate() {
            *byte = self.byte_at(address + offset as u32);
        }
        Ok(SpiRead { bytes, len: size })
    }

    fn byte_at(&self, address: u32) -> u8 {
        let protocol = M::SPEC.protocol;
        match address {
            DEVICE_TYPE_ADDRESS => protocol.device_type,
            COLOR_INFO_EXISTS_ADDRESS => 0x01,
            FACTORY_ACCELEROMETER_CALIBRATION_ADDRESS..=FACTORY_ACCELEROMETER_CALIBRATION_END => {
                calibration_byte(
                    protocol.accelerometer_calibration,
                    address - FACTORY_ACCELEROMETER_CALIBRATION_ADDRESS,
                )
            }
            FACTORY_GYROSCOPE_CALIBRATION_ADDRESS..=FACTORY_GYROSCOPE_CALIBRATION_END => {
                calibration_byte(
                    protocol.gyroscope_calibration,
                    address - FACTORY_GYROSCOPE_CALIBRATION_ADDRESS,
                )
            }
            CONTROLLER_COLORS_ADDRESS..=CONTROLLER_COLORS_END => {
                self.colors.to_spi_bytes()[(address - CONTROLLER_COLORS_ADDRESS) as usize]
            }
            _ => ERASED_BYTE,
        }
    }
}

fn calibration_byte(calibration: SensorCalibration, offset: u32) -> u8 {
    let value_index = (offset / 2) as usize;
    let axis = value_index % 3;
    let value = if value_index < 3 {
        calibration.zero_raw[axis]
    } else {
        calibration.reference_raw[axis]
    };
    value.to_le_bytes()[(offset % 2) as usize]
}
