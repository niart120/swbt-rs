//! Controller model markers and their runtime projection.

mod sealed {
    pub trait Sealed {}
}

/// Read-only model data shared by typed and dynamic boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelSpec {
    kind: ControllerKind,
    profile_name: &'static str,
    has_left_stick: bool,
    has_right_stick: bool,
}

impl ModelSpec {
    const fn new(
        kind: ControllerKind,
        profile_name: &'static str,
        has_left_stick: bool,
        has_right_stick: bool,
    ) -> Self {
        Self {
            kind,
            profile_name,
            has_left_stick,
            has_right_stick,
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
            );

            impl sealed::Sealed for $model {}

            impl ControllerModel for $model {
                const KIND: ControllerKind = ControllerKind::$kind;
                const PROFILE_NAME: &'static str = $profile_name;
                const SPEC: &'static ModelSpec = &$spec;
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
    }
    /// Left Joy-Con model.
    JoyConL {
        kind: JoyConL,
        profile_name: "joycon_l",
        spec: JOYCON_L_SPEC,
        left_stick: true,
        right_stick: false,
    }
    /// Right Joy-Con model.
    JoyConR {
        kind: JoyConR,
        profile_name: "joycon_r",
        spec: JOYCON_R_SPEC,
        left_stick: false,
        right_stick: true,
    }
}
