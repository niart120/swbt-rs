use std::fmt;
use std::marker::PhantomData;

use crate::input::{Button, ButtonSet, ImuFrame, ImuSamples, Stick};
use crate::model::{self, ControllerModel, HasDualSticks, HasLeftStick, HasRightStick};

/// A complete model-valid controller input state.
pub struct InputState<M: ControllerModel> {
    buttons: ButtonSet<M>,
    left_stick: Stick,
    right_stick: Stick,
    imu_frames: [ImuFrame; 3],
    model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> InputState<M> {
    /// Returns a state with no pressed buttons, centered sticks, and neutral IMU.
    #[must_use]
    pub const fn neutral() -> Self {
        Self {
            buttons: ButtonSet::new(),
            left_stick: Stick::center(),
            right_stick: Stick::center(),
            imu_frames: [ImuFrame::neutral(); 3],
            model: PhantomData,
        }
    }

    /// Returns a state whose pressed buttons are replaced by `buttons`.
    #[must_use]
    pub fn with_buttons(mut self, buttons: impl IntoIterator<Item = Button<M>>) -> Self {
        self.buttons = buttons.into_iter().collect();
        self
    }

    /// Returns a state whose IMU report slots are replaced by `samples`.
    #[must_use]
    pub fn with_imu(mut self, samples: impl Into<ImuSamples>) -> Self {
        self.imu_frames = samples.into().into_frames();
        self
    }

    /// Iterates over pressed buttons in stable logical-code order.
    pub fn buttons(&self) -> impl Iterator<Item = Button<M>> + '_ {
        self.buttons.iter()
    }

    /// Returns the three ordered IMU report slots.
    #[must_use]
    pub const fn imu_frames(&self) -> &[ImuFrame; 3] {
        &self.imu_frames
    }

    #[doc(hidden)]
    pub const fn wire_sticks(&self) -> (Stick, Stick) {
        (self.left_stick, self.right_stick)
    }
}

impl<M: HasLeftStick> InputState<M> {
    /// Returns a state with a replaced left stick.
    #[must_use]
    pub const fn with_left_stick(mut self, stick: Stick) -> Self {
        self.left_stick = stick;
        self
    }

    /// Returns the left stick.
    #[must_use]
    pub const fn left_stick(&self) -> Stick {
        self.left_stick
    }
}

impl<M: HasRightStick> InputState<M> {
    /// Returns a state with a replaced right stick.
    #[must_use]
    pub const fn with_right_stick(mut self, stick: Stick) -> Self {
        self.right_stick = stick;
        self
    }

    /// Returns the right stick.
    #[must_use]
    pub const fn right_stick(&self) -> Stick {
        self.right_stick
    }
}

impl<M: HasDualSticks> InputState<M> {
    /// Returns a state with both sticks replaced.
    #[must_use]
    pub const fn with_sticks(mut self, left: Stick, right: Stick) -> Self {
        self.left_stick = left;
        self.right_stick = right;
        self
    }
}

impl<M: ControllerModel> Default for InputState<M> {
    fn default() -> Self {
        Self::neutral()
    }
}

impl<M: ControllerModel> Clone for InputState<M> {
    fn clone(&self) -> Self {
        Self {
            buttons: self.buttons,
            left_stick: self.left_stick,
            right_stick: self.right_stick,
            imu_frames: self.imu_frames,
            model: PhantomData,
        }
    }
}

impl<M: ControllerModel> fmt::Debug for InputState<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InputState")
            .field("model", &M::KIND)
            .field("buttons", &self.buttons)
            .field("left_stick", &self.left_stick)
            .field("right_stick", &self.right_stick)
            .field("imu_frames", &self.imu_frames)
            .finish()
    }
}

impl<M: ControllerModel> PartialEq for InputState<M> {
    fn eq(&self, other: &Self) -> bool {
        self.buttons == other.buttons
            && self.left_stick == other.left_stick
            && self.right_stick == other.right_stick
            && self.imu_frames == other.imu_frames
    }
}

impl<M: ControllerModel> Eq for InputState<M> {}

/// Pro Controller input state.
pub type ProInputState = InputState<model::Pro>;

/// Left Joy-Con input state.
pub type JoyConLInputState = InputState<model::JoyConL>;

/// Right Joy-Con input state.
pub type JoyConRInputState = InputState<model::JoyConR>;
