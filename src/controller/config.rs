use std::{
    marker::PhantomData,
    path::{Path, PathBuf},
};

use std::time::Duration;

use crate::{
    AdapterSelector,
    model::ControllerModel,
    profile::{ControllerColors, PairingProfile},
    reporting::{self, ReportingMode},
    runtime::transport::TransportConfig,
};

#[derive(Debug)]
pub(super) struct BuilderConfig<M: ControllerModel, R: ReportingMode> {
    adapter: AdapterSelector,
    profile_path: Option<PathBuf>,
    colors: ControllerColors,
    mode: <R as reporting::sealed::Sealed>::Config,
    _model: PhantomData<fn() -> M>,
}

impl<M: ControllerModel, R: ReportingMode> BuilderConfig<M, R> {
    pub(super) fn new(
        adapter: AdapterSelector,
        profile_path: Option<PathBuf>,
        colors: ControllerColors,
        mode: <R as reporting::sealed::Sealed>::Config,
    ) -> Self {
        Self {
            adapter,
            profile_path,
            colors,
            mode,
            _model: PhantomData,
        }
    }

    pub(super) fn finalize_with_profile(
        self,
        load_profile: impl FnOnce(&Path) -> crate::Result<PairingProfile<M>>,
    ) -> crate::Result<ControllerConfig<M, R>> {
        let profile = match self.profile_path {
            Some(path) => {
                let profile = load_profile(&path)?;
                ProfileConfig::Persistent { path, profile }
            }
            None => ProfileConfig::Ephemeral,
        };

        Ok(ControllerConfig {
            adapter: self.adapter,
            profile,
            colors: self.colors,
            mode: self.mode,
        })
    }

    #[cfg(test)]
    pub(super) fn adapter(&self) -> &AdapterSelector {
        &self.adapter
    }

    pub(super) fn profile_path(&self) -> Option<&Path> {
        self.profile_path.as_deref()
    }

    #[cfg(test)]
    pub(super) fn colors(&self) -> ControllerColors {
        self.colors
    }

    #[cfg(test)]
    pub(super) fn mode_config(&self) -> &<R as reporting::sealed::Sealed>::Config {
        &self.mode
    }
}

#[derive(Debug)]
pub(super) struct ControllerConfig<M: ControllerModel, R: ReportingMode> {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds retain the selector without opening it"
        )
    )]
    pub(super) adapter: AdapterSelector,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "M6 supplies pairing keys from the profile to the controller runtime"
        )
    )]
    pub(super) profile: ProfileConfig<M>,
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds retain colors without constructing a worker"
        )
    )]
    pub(super) colors: ControllerColors,
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds retain reporting settings without constructing a worker"
        )
    )]
    pub(super) mode: <R as reporting::sealed::Sealed>::Config,
}

impl<M: ControllerModel, R: ReportingMode> ControllerConfig<M, R> {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not project a transport configuration"
        )
    )]
    pub(super) fn transport_config(&self) -> TransportConfig {
        TransportConfig::for_model::<M>()
    }
}

#[derive(Debug)]
pub(super) enum ProfileConfig<M: ControllerModel> {
    Ephemeral,
    Persistent {
        #[cfg_attr(
            not(test),
            allow(
                dead_code,
                reason = "M6 uses the profile path for atomic key persistence"
            )
        )]
        path: PathBuf,
        #[cfg_attr(
            not(test),
            allow(
                dead_code,
                reason = "M6 adapts the validated profile to the Bumble key store"
            )
        )]
        profile: PairingProfile<M>,
    },
}

impl<M: ControllerModel> ControllerConfig<M, reporting::Periodic> {
    pub(super) fn report_period(&self) -> Duration {
        self.mode.report_period()
    }
}

impl<M: ControllerModel> BuilderConfig<M, reporting::Periodic> {
    #[cfg(test)]
    pub(super) fn report_period(&self) -> Duration {
        self.mode.report_period()
    }
}
