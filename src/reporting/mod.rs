//! Reporting mode markers and their runtime projection.

mod sealed {
    pub trait Sealed {}
}

/// Runtime identity for a controller reporting mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReportingKind {
    /// Selects scheduled state reporting in the later runtime.
    Periodic,
    /// Selects caller-driven state reporting in the later runtime.
    Direct,
}

impl ReportingKind {
    /// All supported reporting identities in declaration order.
    pub const ALL: &'static [Self] = &[Self::Periodic, Self::Direct];
}

/// Sealed marker trait implemented by supported reporting modes.
pub trait ReportingMode: sealed::Sealed + Send + 'static {
    /// Runtime identity projected from the marker type.
    const KIND: ReportingKind;
}

/// Periodic reporting mode marker.
#[derive(Debug)]
pub enum Periodic {}

impl sealed::Sealed for Periodic {}

impl ReportingMode for Periodic {
    const KIND: ReportingKind = ReportingKind::Periodic;
}

/// Direct reporting mode marker.
#[derive(Debug)]
pub enum Direct {}

impl sealed::Sealed for Direct {}

impl ReportingMode for Direct {
    const KIND: ReportingKind = ReportingKind::Direct;
}
