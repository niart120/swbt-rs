//! Reporting mode markers and their runtime projection.

pub(crate) mod sealed {
    use std::{fmt::Debug, time::Duration};

    use crate::error::{Error, ErrorKind};

    const MIN_REPORT_PERIOD: Duration = Duration::from_millis(1);
    const DEFAULT_REPORT_PERIOD: Duration = Duration::from_millis(8);
    const MAX_REPORT_PERIOD: Duration = Duration::from_secs(1);

    pub trait Sealed {
        type BuilderOptions: Debug + Send + Sync + 'static;
        type Config: Debug + Send + Sync + 'static;

        fn default_options() -> Self::BuilderOptions;
        fn validate(options: Self::BuilderOptions) -> crate::Result<Self::Config>;
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct PeriodicBuilderOptions {
        report_period: Duration,
    }

    impl PeriodicBuilderOptions {
        pub(crate) const fn default() -> Self {
            Self {
                report_period: DEFAULT_REPORT_PERIOD,
            }
        }

        pub(crate) const fn with_report_period(self, report_period: Duration) -> Self {
            Self { report_period }
        }

        pub(crate) fn validate(self) -> crate::Result<PeriodicConfig> {
            Ok(PeriodicConfig {
                report_period: self.report_period.try_into()?,
            })
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct PeriodicConfig {
        report_period: ValidatedReportPeriod,
    }

    impl PeriodicConfig {
        #[cfg_attr(
            not(test),
            allow(
                dead_code,
                reason = "T31 passes the validated period to the periodic worker"
            )
        )]
        pub(crate) const fn report_period(self) -> Duration {
            self.report_period.0
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct ValidatedReportPeriod(Duration);

    impl TryFrom<Duration> for ValidatedReportPeriod {
        type Error = Error;

        fn try_from(period: Duration) -> Result<Self, Self::Error> {
            if (MIN_REPORT_PERIOD..=MAX_REPORT_PERIOD).contains(&period) {
                Ok(Self(period))
            } else {
                Err(Error::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "report period must be between {MIN_REPORT_PERIOD:?} and \
                         {MAX_REPORT_PERIOD:?} inclusive: {period:?}"
                    ),
                ))
            }
        }
    }
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

impl sealed::Sealed for Periodic {
    type BuilderOptions = sealed::PeriodicBuilderOptions;
    type Config = sealed::PeriodicConfig;

    fn default_options() -> Self::BuilderOptions {
        sealed::PeriodicBuilderOptions::default()
    }

    fn validate(options: Self::BuilderOptions) -> crate::Result<Self::Config> {
        options.validate()
    }
}

impl ReportingMode for Periodic {
    const KIND: ReportingKind = ReportingKind::Periodic;
}

/// Direct reporting mode marker.
#[derive(Debug)]
pub enum Direct {}

impl sealed::Sealed for Direct {
    type BuilderOptions = ();
    type Config = ();

    fn default_options() -> Self::BuilderOptions {}

    fn validate(options: Self::BuilderOptions) -> crate::Result<Self::Config> {
        Ok(options)
    }
}

impl ReportingMode for Direct {
    const KIND: ReportingKind = ReportingKind::Direct;
}
