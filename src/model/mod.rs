//! Controller model markers and their runtime projection.

mod sealed {
    pub trait Sealed {}
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ButtonWirePosition {
    _byte_index: usize,
    _mask: u8,
}

impl ButtonWirePosition {
    pub(crate) const fn new(byte_index: usize, mask: u8) -> Self {
        Self {
            _byte_index: byte_index,
            _mask: mask,
        }
    }

    #[cfg(test)]
    pub(crate) const fn byte_index(self) -> usize {
        self._byte_index
    }

    #[cfg(test)]
    pub(crate) const fn mask(self) -> u8 {
        self._mask
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
}

impl ModelSpec {
    const fn new(
        kind: ControllerKind,
        profile_name: &'static str,
        has_left_stick: bool,
        has_right_stick: bool,
        supported_buttons: &'static [ButtonKind],
        button_wire_mapping: &'static [(ButtonKind, ButtonWirePosition)],
    ) -> Self {
        Self {
            kind,
            profile_name,
            has_left_stick,
            has_right_stick,
            supported_buttons,
            button_wire_mapping,
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
        self.button_wire_mapping
            .iter()
            .any(|(kind, _)| *kind == button)
    }

    #[cfg(test)]
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

macro_rules! controller_models {
    (
        $(
            $(#[$meta:meta])*
            $model:ident {
                kind: $kind:ident,
                profile_name: $profile_name:literal,
                spec: $spec:ident,
                left_stick: $left_stick:literal,
                right_stick: $right_stick:literal,
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

            #[cfg(test)]
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

            static $spec: ModelSpec = ModelSpec::new(
                ControllerKind::$kind,
                $profile_name,
                $left_stick,
                $right_stick,
                &[$(ButtonKind::$button_kind),*],
                &[
                    $(
                        (
                            ButtonKind::$button_kind,
                            ButtonWirePosition::new($byte_index, $mask),
                        ),
                    )*
                ],
            );

            impl sealed::Sealed for $model {}

            impl ControllerModel for $model {
                const KIND: ControllerKind = ControllerKind::$kind;
                const PROFILE_NAME: &'static str = $profile_name;
                const SPEC: &'static ModelSpec = &$spec;
            }

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

#[cfg(test)]
pub(crate) fn button_wire_position(
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
        left_stick: true,
        right_stick: true,
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
        left_stick: true,
        right_stick: false,
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
        left_stick: false,
        right_stick: true,
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
