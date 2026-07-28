use std::fmt;
use std::hash::{Hash, Hasher};
use std::iter::FromIterator;
use std::marker::PhantomData;

use crate::error::{Error, ErrorKind};
use crate::model::{self, ButtonKind, ControllerModel};

/// A logical button proven to be supported by controller model `M`.
pub struct Button<M: ControllerModel> {
    kind: ButtonKind,
    model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> Button<M> {
    pub(crate) const fn from_kind_unchecked(kind: ButtonKind) -> Self {
        Self {
            kind,
            model: PhantomData,
        }
    }

    /// Returns the model-independent logical button identity.
    #[must_use]
    pub const fn kind(self) -> ButtonKind {
        self.kind
    }
}

impl<M: ControllerModel> TryFrom<ButtonKind> for Button<M> {
    type Error = Error;

    fn try_from(kind: ButtonKind) -> Result<Self, Self::Error> {
        if M::SPEC.supports_button(kind) {
            Ok(Self::from_kind_unchecked(kind))
        } else {
            Err(Error::new(
                ErrorKind::UnsupportedInput,
                format!(
                    "{kind:?} is not supported by controller model {:?}",
                    M::KIND
                ),
            ))
        }
    }
}

impl<M: ControllerModel> Clone for Button<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: ControllerModel> Copy for Button<M> {}

impl<M: ControllerModel> fmt::Debug for Button<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(formatter)
    }
}

impl<M: ControllerModel> PartialEq for Button<M> {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl<M: ControllerModel> Eq for Button<M> {}

impl<M: ControllerModel> Hash for Button<M> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
    }
}

/// A duplicate-free set of model-valid buttons.
pub struct ButtonSet<M: ControllerModel> {
    bits: u32,
    model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel> ButtonSet<M> {
    /// Returns an empty button set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bits: 0,
            model: PhantomData,
        }
    }

    /// Returns the number of buttons in this set.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bits.count_ones() as usize
    }

    /// Returns `true` when the set has no buttons.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bits == 0
    }

    /// Returns `true` when `button` is in this set.
    #[must_use]
    pub const fn contains(&self, button: Button<M>) -> bool {
        self.bits & button_bit(button.kind) != 0
    }

    /// Inserts `button` and returns whether it was newly inserted.
    pub fn insert(&mut self, button: Button<M>) -> bool {
        let bit = button_bit(button.kind);
        let was_absent = self.bits & bit == 0;
        self.bits |= bit;
        was_absent
    }

    /// Iterates over buttons in stable logical-code order.
    pub fn iter(&self) -> impl Iterator<Item = Button<M>> + '_ {
        M::SPEC
            .supported_buttons()
            .iter()
            .copied()
            .filter(|kind| self.bits & button_bit(*kind) != 0)
            .map(Button::from_kind_unchecked)
    }
}

impl<M: ControllerModel> Default for ButtonSet<M> {
    fn default() -> Self {
        Self::new()
    }
}

impl<M: ControllerModel> FromIterator<Button<M>> for ButtonSet<M> {
    fn from_iter<T: IntoIterator<Item = Button<M>>>(buttons: T) -> Self {
        let mut set = Self::new();
        for button in buttons {
            set.insert(button);
        }
        set
    }
}

impl<M: ControllerModel> Clone for ButtonSet<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M: ControllerModel> Copy for ButtonSet<M> {}

impl<M: ControllerModel> fmt::Debug for ButtonSet<M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_set().entries(self.iter()).finish()
    }
}

impl<M: ControllerModel> PartialEq for ButtonSet<M> {
    fn eq(&self, other: &Self) -> bool {
        self.bits == other.bits
    }
}

impl<M: ControllerModel> Eq for ButtonSet<M> {}

const fn button_bit(kind: ButtonKind) -> u32 {
    1_u32 << kind as u8
}

/// Pro Controller button.
pub type ProButton = Button<model::Pro>;

/// Left Joy-Con button.
pub type JoyConLButton = Button<model::JoyConL>;

/// Right Joy-Con button.
pub type JoyConRButton = Button<model::JoyConR>;
