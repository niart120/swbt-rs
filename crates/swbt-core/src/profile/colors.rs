use crate::error::{Error, ErrorKind};

/// A 24-bit red, green, and blue color value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb24(u32);

impl Rgb24 {
    /// Creates a color from red, green, and blue components.
    #[must_use]
    pub const fn new(red: u8, green: u8, blue: u8) -> Self {
        Self((red as u32) << 16 | (green as u32) << 8 | blue as u32)
    }

    /// Returns the packed `0xRRGGBB` value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Returns the red, green, and blue components.
    #[must_use]
    pub const fn components(self) -> [u8; 3] {
        [
            ((self.0 >> 16) & 0xFF) as u8,
            ((self.0 >> 8) & 0xFF) as u8,
            (self.0 & 0xFF) as u8,
        ]
    }
}

impl TryFrom<u32> for Rgb24 {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        if value <= 0xFF_FFFF {
            Ok(Self(value))
        } else {
            Err(Error::new(
                ErrorKind::InvalidInput,
                format!("RGB value must be between 0x000000 and 0xFFFFFF: 0x{value:X}"),
            ))
        }
    }
}

impl From<Rgb24> for u32 {
    fn from(color: Rgb24) -> Self {
        color.value()
    }
}

/// Body, button, and grip colors stored in the virtual controller profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ControllerColors {
    body: Rgb24,
    buttons: Rgb24,
    left_grip: Rgb24,
    right_grip: Rgb24,
}

impl ControllerColors {
    /// Creates a complete set of controller colors.
    #[must_use]
    pub const fn new(body: Rgb24, buttons: Rgb24, left_grip: Rgb24, right_grip: Rgb24) -> Self {
        Self {
            body,
            buttons,
            left_grip,
            right_grip,
        }
    }

    /// Returns the body color.
    #[must_use]
    pub const fn body(self) -> Rgb24 {
        self.body
    }

    /// Returns the button color.
    #[must_use]
    pub const fn buttons(self) -> Rgb24 {
        self.buttons
    }

    /// Returns the left grip color.
    #[must_use]
    pub const fn left_grip(self) -> Rgb24 {
        self.left_grip
    }

    /// Returns the right grip color.
    #[must_use]
    pub const fn right_grip(self) -> Rgb24 {
        self.right_grip
    }

    /// Returns body, button, left grip, and right grip colors in SPI RGB order.
    #[must_use]
    pub const fn to_spi_bytes(self) -> [u8; 12] {
        let body = self.body.components();
        let buttons = self.buttons.components();
        let left_grip = self.left_grip.components();
        let right_grip = self.right_grip.components();
        [
            body[0],
            body[1],
            body[2],
            buttons[0],
            buttons[1],
            buttons[2],
            left_grip[0],
            left_grip[1],
            left_grip[2],
            right_grip[0],
            right_grip[1],
            right_grip[2],
        ]
    }
}

impl Default for ControllerColors {
    fn default() -> Self {
        Self::new(
            Rgb24::new(0x32, 0x32, 0x32),
            Rgb24::new(0xFF, 0xFF, 0xFF),
            Rgb24::new(0x00, 0xB2, 0xFF),
            Rgb24::new(0xFF, 0x3B, 0x30),
        )
    }
}
