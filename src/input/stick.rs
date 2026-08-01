use std::ops::RangeInclusive;

use crate::error::{Error, ErrorKind, Result};

/// A 12-bit two-axis stick position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Stick {
    x: u16,
    y: u16,
}

impl Stick {
    /// Minimum raw axis value.
    pub const MIN: u16 = 0;

    /// Neutral raw axis value.
    pub const CENTER: u16 = 2048;

    /// Maximum raw axis value.
    pub const MAX: u16 = 4095;

    /// Returns the neutral stick position.
    #[must_use]
    pub const fn center() -> Self {
        Self {
            x: Self::CENTER,
            y: Self::CENTER,
        }
    }

    /// Creates a stick from raw 12-bit axes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when either axis exceeds 4095.
    pub fn raw(x: u16, y: u16) -> Result<Self> {
        validate_raw_axis("x", x)?;
        validate_raw_axis("y", y)?;
        Ok(Self { x, y })
    }

    /// Creates a stick from axes in the inclusive `-1.0..=1.0` range.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when either axis is non-finite or
    /// outside the accepted range.
    pub fn normalized(x: f32, y: f32) -> Result<Self> {
        Ok(Self {
            x: axis_input_to_raw("x", x, -1.0..=1.0, 1.0)?,
            y: axis_input_to_raw("y", y, -1.0..=1.0, 1.0)?,
        })
    }

    /// Creates a stick from normalized tilt axes.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] under the same conditions as
    /// [`Stick::normalized`].
    pub fn tilt(x: f32, y: f32) -> Result<Self> {
        Self::normalized(x, y)
    }

    /// Returns a stick tilted upward by `amount`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when `amount` is non-finite or
    /// outside `0.0..=1.0`.
    pub fn up(amount: f32) -> Result<Self> {
        Ok(Self {
            x: Self::CENTER,
            y: axis_input_to_raw("amount", amount, 0.0..=1.0, 1.0)?,
        })
    }

    /// Returns a stick tilted downward by `amount`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when `amount` is non-finite or
    /// outside `0.0..=1.0`.
    pub fn down(amount: f32) -> Result<Self> {
        Ok(Self {
            x: Self::CENTER,
            y: axis_input_to_raw("amount", amount, 0.0..=1.0, -1.0)?,
        })
    }

    /// Returns a stick tilted left by `amount`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when `amount` is non-finite or
    /// outside `0.0..=1.0`.
    pub fn left(amount: f32) -> Result<Self> {
        Ok(Self {
            x: axis_input_to_raw("amount", amount, 0.0..=1.0, -1.0)?,
            y: Self::CENTER,
        })
    }

    /// Returns a stick tilted right by `amount`.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorKind::InvalidInput`] when `amount` is non-finite or
    /// outside `0.0..=1.0`.
    pub fn right(amount: f32) -> Result<Self> {
        Ok(Self {
            x: axis_input_to_raw("amount", amount, 0.0..=1.0, 1.0)?,
            y: Self::CENTER,
        })
    }

    /// Returns the raw horizontal axis.
    #[must_use]
    pub const fn x(self) -> u16 {
        self.x
    }

    /// Returns the raw vertical axis.
    #[must_use]
    pub const fn y(self) -> u16 {
        self.y
    }

    /// Returns both raw axes as `(x, y)`.
    #[must_use]
    pub const fn axes(self) -> (u16, u16) {
        (self.x, self.y)
    }
}

fn validate_raw_axis(name: &str, value: u16) -> Result<()> {
    if value <= Stick::MAX {
        Ok(())
    } else {
        Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{name} must be between 0 and 4095: {value}"),
        ))
    }
}

fn axis_input_to_raw(
    name: &str,
    value: f32,
    accepted: RangeInclusive<f32>,
    direction: f32,
) -> Result<u16> {
    if !value.is_finite() || !accepted.contains(&value) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{name} must be finite and within {accepted:?}: {value}"),
        ));
    }

    Ok(normalized_axis_to_raw(value * direction))
}

fn normalized_axis_to_raw(value: f32) -> u16 {
    let distance = if value < 0.0 {
        f32::from(Stick::CENTER - Stick::MIN)
    } else {
        f32::from(Stick::MAX - Stick::CENTER)
    };
    let raw = f32::from(Stick::CENTER) + (value * distance).round_ties_even();
    raw as u16
}
