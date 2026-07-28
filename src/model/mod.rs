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

/// Read-only model data shared by typed and dynamic boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    kind: ControllerKind,
    profile_name: &'static str,
    has_left_stick: bool,
    has_right_stick: bool,
    supported_buttons: &'static [ButtonKind],
}

impl ModelSpec {
    const fn new(
        kind: ControllerKind,
        profile_name: &'static str,
        has_left_stick: bool,
        has_right_stick: bool,
        supported_buttons: &'static [ButtonKind],
    ) -> Self {
        Self {
            kind,
            profile_name,
            has_left_stick,
            has_right_stick,
            supported_buttons,
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
        self.supported_buttons.contains(&button)
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
                    $($button_kind:ident => $button_const:ident),* $(,)?
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

controller_models! {
    /// Pro Controller model.
    Pro {
        kind: Pro,
        profile_name: "pro",
        spec: PRO_SPEC,
        left_stick: true,
        right_stick: true,
        buttons: [
            A => A,
            B => B,
            X => X,
            Y => Y,
            L => L,
            R => R,
            ZL => ZL,
            ZR => ZR,
            Plus => PLUS,
            Minus => MINUS,
            Home => HOME,
            Capture => CAPTURE,
            LeftStick => LEFT_STICK,
            RightStick => RIGHT_STICK,
            DpadUp => DPAD_UP,
            DpadDown => DPAD_DOWN,
            DpadLeft => DPAD_LEFT,
            DpadRight => DPAD_RIGHT,
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
            L => L,
            ZL => ZL,
            Minus => MINUS,
            Capture => CAPTURE,
            LeftStick => LEFT_STICK,
            SL => SL,
            SR => SR,
            DpadUp => DPAD_UP,
            DpadDown => DPAD_DOWN,
            DpadLeft => DPAD_LEFT,
            DpadRight => DPAD_RIGHT,
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
            A => A,
            B => B,
            X => X,
            Y => Y,
            R => R,
            ZR => ZR,
            Plus => PLUS,
            Home => HOME,
            RightStick => RIGHT_STICK,
            SL => SL,
            SR => SR,
        ],
    }
}
