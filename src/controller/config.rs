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
        not(test),
        allow(
            dead_code,
            reason = "T31 selects the adapter when opening the controller runtime"
        )
    )]
    pub(super) adapter: AdapterSelector,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 supplies the pairing profile to the controller runtime"
        )
    )]
    pub(super) profile: ProfileConfig<M>,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 supplies fixed controller colors to the runtime"
        )
    )]
    pub(super) colors: ControllerColors,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T31 supplies reporting-specific settings to the worker"
        )
    )]
    pub(super) mode: <R as reporting::sealed::Sealed>::Config,
}

#[derive(Debug)]
pub(super) enum ProfileConfig<M: ControllerModel> {
    Ephemeral,
    Persistent {
        #[cfg_attr(
            not(test),
            allow(
                dead_code,
                reason = "T31 retains the profile path for runtime persistence"
            )
        )]
        path: PathBuf,
        #[cfg_attr(
            not(test),
            allow(
                dead_code,
                reason = "T31 supplies the validated pairing profile to the runtime"
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
