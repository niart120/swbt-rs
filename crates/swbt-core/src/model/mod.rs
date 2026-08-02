//! Controller model markers and their runtime projection.

use crate::profile::{ControllerColors, Rgb24};

mod hid;
mod sealed {
    pub trait Sealed {}
}

#[doc(hidden)]
pub use hid::HidSdpPolicySpec;
use hid::{JOYCON_HID_SDP_POLICY, PRO_HID_SDP_POLICY, SWITCH_HID_REPORT_DESCRIPTOR};

/// Model-independent logical button identity.
///
/// The numeric value is a stable logical code and is not an NX report bit
/// position.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ButtonKind {
    /// A face button.
    A = 0x00,
    /// B face button.
    B = 0x01,
    /// X face button.
    X = 0x02,
    /// Y face button.
    Y = 0x03,
    /// Left shoulder button.
    L = 0x04,
    /// Right shoulder button.
    R = 0x05,
    /// Left trigger.
    ZL = 0x06,
    /// Right trigger.
    ZR = 0x07,
    /// Plus button.
    Plus = 0x08,
    /// Minus button.
    Minus = 0x09,
    /// Home button.
    Home = 0x0A,
    /// Capture button.
    Capture = 0x0B,
    /// Left stick click.
    LeftStick = 0x0C,
    /// Right stick click.
    RightStick = 0x0D,
    /// Rail SL button.
    SL = 0x0E,
    /// Rail SR button.
    SR = 0x0F,
    /// D-pad up.
    DpadUp = 0x10,
    /// D-pad down.
    DpadDown = 0x11,
    /// D-pad left.
    DpadLeft = 0x12,
    /// D-pad right.
    DpadRight = 0x13,
}

impl ButtonKind {
    /// All logical buttons in stable code order.
    pub const ALL: &'static [Self] = &[
        Self::A,
        Self::B,
        Self::X,
        Self::Y,
        Self::L,
        Self::R,
        Self::ZL,
        Self::ZR,
        Self::Plus,
        Self::Minus,
        Self::Home,
        Self::Capture,
        Self::LeftStick,
        Self::RightStick,
        Self::SL,
        Self::SR,
        Self::DpadUp,
        Self::DpadDown,
        Self::DpadLeft,
        Self::DpadRight,
    ];
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonWirePosition {
    pub byte_index: usize,
    pub mask: u8,
}

impl ButtonWirePosition {
    pub const fn new(byte_index: usize, mask: u8) -> Self {
        Self { byte_index, mask }
    }
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorCalibration {
    pub zero_raw: [i16; 3],
    pub reference_raw: [i16; 3],
}

#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelProtocolSpec {
    pub local_name: &'static str,
    pub class_of_device: u32,
    pub device_type: u8,
    pub device_info_tail: [u8; 2],
    pub battery_connection: u8,
    pub vibrator_input: u8,
    pub pairing_trigger_buttons: &'static [ButtonKind],
    pub accepted_imu_modes: &'static [u8],
    pub default_colors: ControllerColors,
    pub accelerometer_calibration: SensorCalibration,
    pub gyroscope_calibration: SensorCalibration,
    pub hid_report_descriptor: &'static [u8],
    pub hid_sdp_policy: HidSdpPolicySpec,
}

impl ModelProtocolSpec {
    const fn new(
        local_name: &'static str,
        class_of_device: u32,
        device_type: u8,
        device_info_tail: [u8; 2],
        pairing_trigger_buttons: &'static [ButtonKind],
        default_colors: ControllerColors,
        hid_sdp_policy: HidSdpPolicySpec,
    ) -> Self {
        Self {
            local_name,
            class_of_device,
            device_type,
            device_info_tail,
            battery_connection: 0x80,
            vibrator_input: 0x00,
            pairing_trigger_buttons,
            accepted_imu_modes: &[0, 1, 2, 3, 4, 5],
            default_colors,
            accelerometer_calibration: SensorCalibration {
                zero_raw: [0; 3],
                reference_raw: [0x4000; 3],
            },
            gyroscope_calibration: SensorCalibration {
                zero_raw: [0; 3],
                reference_raw: [0x343B; 3],
            },
            hid_report_descriptor: &SWITCH_HID_REPORT_DESCRIPTOR,
            hid_sdp_policy,
        }
    }
}

/// Read-only model data shared by typed and dynamic boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    kind: ControllerKind,
    profile_name: &'static str,
    has_left_stick: bool,
    has_right_stick: bool,
    supported_buttons: &'static [ButtonKind],
    button_wire_mapping: &'static [(ButtonKind, ButtonWirePosition)],
    #[doc(hidden)]
    pub protocol: &'static ModelProtocolSpec,
}

impl ModelSpec {
    const fn new(
        kind: ControllerKind,
        profile_name: &'static str,
        has_left_stick: bool,
        has_right_stick: bool,
        supported_buttons: &'static [ButtonKind],
        button_wire_mapping: &'static [(ButtonKind, ButtonWirePosition)],
        protocol: &'static ModelProtocolSpec,
    ) -> Self {
        Self {
            kind,
            profile_name,
            has_left_stick,
            has_right_stick,
            supported_buttons,
            button_wire_mapping,
            protocol,
        }
    }

    /// Returns the runtime controller identity.
    #[must_use]
    pub const fn kind(self) -> ControllerKind {
        self.kind
    }

    /// Returns the stable profile name.
    #[must_use]
    pub const fn profile_name(self) -> &'static str {
        self.profile_name
    }

    /// Returns whether the model has a left stick.
    #[must_use]
    pub const fn has_left_stick(self) -> bool {
        self.has_left_stick
    }

    /// Returns whether the model has a right stick.
    #[must_use]
    pub const fn has_right_stick(self) -> bool {
        self.has_right_stick
    }

    /// Returns supported logical buttons in stable code order.
    #[must_use]
    pub const fn supported_buttons(self) -> &'static [ButtonKind] {
        self.supported_buttons
    }

    /// Returns whether `button` is valid for this model.
    #[must_use]
    pub fn supports_button(self, button: ButtonKind) -> bool {
        button_wire_position(self.kind, button).is_some()
    }

    fn button_wire_position(self, button: ButtonKind) -> Option<ButtonWirePosition> {
        self.button_wire_mapping
            .iter()
            .find_map(|(kind, position)| (*kind == button).then_some(*position))
    }
}

/// Sealed marker trait implemented by supported controller models.
pub trait ControllerModel: sealed::Sealed + Send + 'static {
    /// Runtime identity projected from the marker type.
    const KIND: ControllerKind;

    /// Stable profile name projected from the marker type.
    const PROFILE_NAME: &'static str;

    /// Read-only model declaration.
    const SPEC: &'static ModelSpec;
}

/// Marker trait for controller models with a left stick.
pub trait HasLeftStick: ControllerModel {}

/// Marker trait for controller models with a right stick.
pub trait HasRightStick: ControllerModel {}

/// Marker trait for controller models with both sticks.
pub trait HasDualSticks: HasLeftStick + HasRightStick {}

macro_rules! has_left_stick {
    () => {
        false
    };
    (left $(, $rest:ident)*) => {
        true
    };
    ($other:ident $(, $rest:ident)*) => {
        has_left_stick!($($rest),*)
    };
}

macro_rules! has_right_stick {
    () => {
        false
    };
    (right $(, $rest:ident)*) => {
        true
    };
    ($other:ident $(, $rest:ident)*) => {
        has_right_stick!($($rest),*)
    };
}

macro_rules! impl_stick_capabilities {
    ($model:ident; left, right) => {
        impl HasLeftStick for $model {}
        impl HasRightStick for $model {}
        impl HasDualSticks for $model {}
    };
    ($model:ident; left) => {
        impl HasLeftStick for $model {}
    };
    ($model:ident; right) => {
        impl HasRightStick for $model {}
    };
}

macro_rules! controller_models {
    (
        $(
            $(#[$meta:meta])*
            $model:ident {
                kind: $kind:ident,
                profile_name: $profile_name:literal,
                spec: $spec:ident,
                protocol_spec: $protocol_spec:ident,
                protocol: {
                    local_name: $local_name:literal,
                    class_of_device: $class_of_device:literal,
                    device_type: $device_type:literal,
                    device_info_tail: $device_info_tail:expr,
                    pairing_trigger_buttons: [$($pairing_button:ident),+ $(,)?],
                    hid_sdp_policy: $hid_sdp_policy:expr,
                    default_colors: [
                        $body_color:expr,
                        $button_color:expr,
                        $left_grip_color:expr,
                        $right_grip_color:expr $(,)?
                    ],
                },
                sticks: [$($stick:ident),* $(,)?],
                buttons: [
                    $(
                        $button_kind:ident => $button_const:ident
                            @ $byte_index:literal / $mask:literal
                    ),* $(,)?
                ],
            }
        )+
    ) => {
        /// Runtime identity for a supported controller model.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum ControllerKind {
            $(
                $(#[$meta])*
                $kind,
            )+
        }

        impl ControllerKind {
            /// All supported controller identities in declaration order.
            pub const ALL: &'static [Self] = &[
                $(Self::$kind,)+
            ];

            /// Returns the stable profile name for this controller identity.
            #[must_use]
            pub const fn profile_name(self) -> &'static str {
                match self {
                    $(Self::$kind => $profile_name,)+
                }
            }

            pub(crate) const fn spec(self) -> &'static ModelSpec {
                match self {
                    $(Self::$kind => &$spec,)+
                }
            }
        }

        $(
            $(#[$meta])*
            #[derive(Debug)]
            pub enum $model {}

            static $protocol_spec: ModelProtocolSpec = ModelProtocolSpec::new(
                $local_name,
                $class_of_device,
                $device_type,
                $device_info_tail,
                &[$(ButtonKind::$pairing_button),+],
                ControllerColors::new(
                    $body_color,
                    $button_color,
                    $left_grip_color,
                    $right_grip_color,
                ),
                $hid_sdp_policy,
            );

            static $spec: ModelSpec = ModelSpec::new(
                ControllerKind::$kind,
                $profile_name,
                has_left_stick!($($stick),*),
                has_right_stick!($($stick),*),
                &[$(ButtonKind::$button_kind),*],
                &[
                    $(
                        (
                            ButtonKind::$button_kind,
                            ButtonWirePosition::new($byte_index, $mask),
                        ),
                    )*
                ],
                &$protocol_spec,
            );

            impl sealed::Sealed for $model {}

            impl ControllerModel for $model {
                const KIND: ControllerKind = ControllerKind::$kind;
                const PROFILE_NAME: &'static str = $profile_name;
                const SPEC: &'static ModelSpec = &$spec;
            }

            impl_stick_capabilities!($model; $($stick),*);

            impl crate::input::Button<$model> {
                $(
                    #[doc = concat!("The `", stringify!($button_kind), "` button.")]
                    pub const $button_const: Self =
                        Self::from_kind_unchecked(ButtonKind::$button_kind);
                )*
            }
        )+
    };
}

#[doc(hidden)]
pub fn button_wire_position(
    controller: ControllerKind,
    button: ButtonKind,
) -> Option<ButtonWirePosition> {
    controller.spec().button_wire_position(button)
}

controller_models! {
    /// Pro Controller model.
    Pro {
        kind: Pro,
        profile_name: "pro",
        spec: PRO_SPEC,
        protocol_spec: PRO_PROTOCOL_SPEC,
        protocol: {
            local_name: "Pro Controller",
            class_of_device: 0x002508,
            device_type: 0x03,
            device_info_tail: [0x03, 0x02],
            pairing_trigger_buttons: [L, R],
            hid_sdp_policy: PRO_HID_SDP_POLICY,
            default_colors: [
                Rgb24::new(0x32, 0x32, 0x32),
                Rgb24::new(0xFF, 0xFF, 0xFF),
                Rgb24::new(0x00, 0xB2, 0xFF),
                Rgb24::new(0xFF, 0x3B, 0x30),
            ],
        },
        sticks: [left, right],
        buttons: [
            A => A @ 3 / 0x08,
            B => B @ 3 / 0x04,
            X => X @ 3 / 0x02,
            Y => Y @ 3 / 0x01,
            L => L @ 5 / 0x40,
            R => R @ 3 / 0x40,
            ZL => ZL @ 5 / 0x80,
            ZR => ZR @ 3 / 0x80,
            Plus => PLUS @ 4 / 0x02,
            Minus => MINUS @ 4 / 0x01,
            Home => HOME @ 4 / 0x10,
            Capture => CAPTURE @ 4 / 0x20,
            LeftStick => LEFT_STICK @ 4 / 0x08,
            RightStick => RIGHT_STICK @ 4 / 0x04,
            DpadUp => DPAD_UP @ 5 / 0x02,
            DpadDown => DPAD_DOWN @ 5 / 0x01,
            DpadLeft => DPAD_LEFT @ 5 / 0x08,
            DpadRight => DPAD_RIGHT @ 5 / 0x04,
        ],
    }
    /// Left Joy-Con model.
    JoyConL {
        kind: JoyConL,
        profile_name: "joycon_l",
        spec: JOYCON_L_SPEC,
        protocol_spec: JOYCON_L_PROTOCOL_SPEC,
        protocol: {
            local_name: "Joy-Con (L)",
            class_of_device: 0x002508,
            device_type: 0x01,
            device_info_tail: [0x01, 0x01],
            pairing_trigger_buttons: [SL, SR],
            hid_sdp_policy: JOYCON_HID_SDP_POLICY,
            default_colors: [
                Rgb24::new(0x00, 0xB2, 0xFF),
                Rgb24::new(0x32, 0x32, 0x32),
                Rgb24::new(0x00, 0xB2, 0xFF),
                Rgb24::new(0x00, 0xB2, 0xFF),
            ],
        },
        sticks: [left],
        buttons: [
            L => L @ 5 / 0x40,
            ZL => ZL @ 5 / 0x80,
            Minus => MINUS @ 4 / 0x01,
            Capture => CAPTURE @ 4 / 0x20,
            LeftStick => LEFT_STICK @ 4 / 0x08,
            SL => SL @ 5 / 0x20,
            SR => SR @ 5 / 0x10,
            DpadUp => DPAD_UP @ 5 / 0x02,
            DpadDown => DPAD_DOWN @ 5 / 0x01,
            DpadLeft => DPAD_LEFT @ 5 / 0x08,
            DpadRight => DPAD_RIGHT @ 5 / 0x04,
        ],
    }
    /// Right Joy-Con model.
    JoyConR {
        kind: JoyConR,
        profile_name: "joycon_r",
        spec: JOYCON_R_SPEC,
        protocol_spec: JOYCON_R_PROTOCOL_SPEC,
        protocol: {
            local_name: "Joy-Con (R)",
            class_of_device: 0x002508,
            device_type: 0x02,
            device_info_tail: [0x01, 0x01],
            pairing_trigger_buttons: [SL, SR],
            hid_sdp_policy: JOYCON_HID_SDP_POLICY,
            default_colors: [
                Rgb24::new(0xFF, 0x3B, 0x30),
                Rgb24::new(0x32, 0x32, 0x32),
                Rgb24::new(0xFF, 0x3B, 0x30),
                Rgb24::new(0xFF, 0x3B, 0x30),
            ],
        },
        sticks: [right],
        buttons: [
            A => A @ 3 / 0x08,
            B => B @ 3 / 0x04,
            X => X @ 3 / 0x02,
            Y => Y @ 3 / 0x01,
            R => R @ 3 / 0x40,
            ZR => ZR @ 3 / 0x80,
            Plus => PLUS @ 4 / 0x02,
            Home => HOME @ 4 / 0x10,
            RightStick => RIGHT_STICK @ 4 / 0x04,
            SL => SL @ 3 / 0x20,
            SR => SR @ 3 / 0x10,
        ],
    }
}

#[cfg(test)]
mod tests;
