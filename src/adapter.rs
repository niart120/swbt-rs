/// Opaque selector passed to the Bluetooth adapter backend.
///
/// The selector syntax is interpreted by the concrete backend. This type keeps
/// backend-specific selector types out of the public controller API.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AdapterSelector(Box<str>);

impl From<String> for AdapterSelector {
    fn from(value: String) -> Self {
        Self(value.into_boxed_str())
    }
}

impl From<&str> for AdapterSelector {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
