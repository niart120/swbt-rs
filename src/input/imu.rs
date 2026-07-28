use crate::error::{Error, ErrorKind, Result};

/// One raw six-axis sensor frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImuFrame {
    accel: [i16; 3],
    gyro: [i16; 3],
}

impl ImuFrame {
    /// Virtual gyroscope sensitivity in degrees per second per raw unit.
    pub const GYRO_DPS_PER_RAW: f64 = 0.070;

    /// Virtual accelerometer sensitivity in G per raw unit.
    pub const ACCEL_G_PER_RAW: f64 = 1.0 / 4096.0;

    /// Returns a frame with all axes set to zero.
    #[must_use]
    pub const fn neutral() -> Self {
        Self {
            accel: [0; 3],
            gyro: [0; 3],
        }
    }

    /// Creates a frame from raw accelerometer and gyroscope axes.
    #[must_use]
    pub const fn raw(accel: [i16; 3], gyro: [i16; 3]) -> Self {
        Self { accel, gyro }
    }

    /// Returns the raw accelerometer axes.
    #[must_use]
    pub const fn accel(self) -> [i16; 3] {
        self.accel
    }

    /// Returns the raw gyroscope axes.
    #[must_use]
    pub const fn gyro(self) -> [i16; 3] {
        self.gyro
    }

    /// Returns a frame with replaced accelerometer axes.
    #[must_use]
    pub const fn with_accel(self, accel: [i16; 3]) -> Self {
        Self {
            accel,
            gyro: self.gyro,
        }
    }

    /// Returns a frame with replaced gyroscope axes.
    #[must_use]
    pub const fn with_gyro(self, gyro: [i16; 3]) -> Self {
        Self {
            accel: self.accel,
            gyro,
        }
    }

    /// Creates a frame from gyroscope rates in radians per second.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when a rate is non-finite or its
    /// converted raw value is outside the signed 16-bit range.
    pub fn gyro_rate(x_rad_s: f64, y_rad_s: f64, z_rad_s: f64) -> Result<Self> {
        Ok(Self::raw(
            [0; 3],
            [
                gyro_rate_to_raw("x_rad_s", x_rad_s)?,
                gyro_rate_to_raw("y_rad_s", y_rad_s)?,
                gyro_rate_to_raw("z_rad_s", z_rad_s)?,
            ],
        ))
    }

    /// Returns gyroscope rates in radians per second.
    #[must_use]
    pub fn to_gyro_rate(self) -> [f64; 3] {
        self.gyro
            .map(|raw| (f64::from(raw) * Self::GYRO_DPS_PER_RAW).to_radians())
    }

    /// Returns a frame with gyroscope rates replaced from radians per second.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] under the same conditions as
    /// [`ImuFrame::gyro_rate`].
    pub fn with_gyro_rate(self, x_rad_s: f64, y_rad_s: f64, z_rad_s: f64) -> Result<Self> {
        Ok(self.with_gyro(Self::gyro_rate(x_rad_s, y_rad_s, z_rad_s)?.gyro))
    }

    /// Creates a frame from accelerations in G.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when a value is non-finite or its
    /// converted raw value is outside the signed 16-bit range.
    pub fn accel_g(x_g: f64, y_g: f64, z_g: f64) -> Result<Self> {
        Ok(Self::raw(
            [
                acceleration_to_raw("x_g", x_g)?,
                acceleration_to_raw("y_g", y_g)?,
                acceleration_to_raw("z_g", z_g)?,
            ],
            [0; 3],
        ))
    }

    /// Returns accelerations in G.
    #[must_use]
    pub fn to_accel_g(self) -> [f64; 3] {
        self.accel.map(|raw| f64::from(raw) * Self::ACCEL_G_PER_RAW)
    }

    /// Returns a frame with accelerations replaced from values in G.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] under the same conditions as
    /// [`ImuFrame::accel_g`].
    pub fn with_accel_g(self, x_g: f64, y_g: f64, z_g: f64) -> Result<Self> {
        Ok(self.with_accel(Self::accel_g(x_g, y_g, z_g)?.accel))
    }
}

fn gyro_rate_to_raw(name: &str, value: f64) -> Result<i16> {
    rounded_i16(name, value.to_degrees() / ImuFrame::GYRO_DPS_PER_RAW)
}

fn acceleration_to_raw(name: &str, value: f64) -> Result<i16> {
    rounded_i16(name, value / ImuFrame::ACCEL_G_PER_RAW)
}

fn rounded_i16(name: &str, value: f64) -> Result<i16> {
    if !value.is_finite() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{name} must convert from a finite value"),
        ));
    }

    let rounded = value.round_ties_even();
    if rounded < f64::from(i16::MIN) || rounded > f64::from(i16::MAX) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{name} converts outside the signed 16-bit range: {rounded}"),
        ));
    }
    Ok(rounded as i16)
}

/// One repeated IMU frame or three ordered frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImuSamples {
    /// Repeat one frame in all three report slots.
    Repeat(ImuFrame),
    /// Preserve three frames in report order.
    Frames([ImuFrame; 3]),
}

impl ImuSamples {
    /// Expands these samples into the three report slots.
    #[must_use]
    pub const fn into_frames(self) -> [ImuFrame; 3] {
        match self {
            Self::Repeat(frame) => [frame; 3],
            Self::Frames(frames) => frames,
        }
    }
}

impl From<ImuFrame> for ImuSamples {
    fn from(frame: ImuFrame) -> Self {
        Self::Repeat(frame)
    }
}

impl From<[ImuFrame; 3]> for ImuSamples {
    fn from(frames: [ImuFrame; 3]) -> Self {
        Self::Frames(frames)
    }
}
