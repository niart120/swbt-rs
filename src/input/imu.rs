/// One raw six-axis sensor frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ImuFrame {
    accel: [i16; 3],
    gyro: [i16; 3],
}

impl ImuFrame {
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
