use std::{marker::PhantomData, path::PathBuf};

#[cfg(test)]
use std::{path::Path, time::Duration};

use crate::{
    AdapterSelector,
    model::ControllerModel,
    profile::ControllerColors,
    reporting::{self, ReportingMode},
};

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T29 consumes validated builder settings when constructing ControllerConfig"
    )
)]
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

    #[cfg(test)]
    pub(super) fn adapter(&self) -> &AdapterSelector {
        &self.adapter
    }

    #[cfg(test)]
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

impl<M: ControllerModel> BuilderConfig<M, reporting::Periodic> {
    #[cfg(test)]
    pub(super) fn report_period(&self) -> Duration {
        self.mode.report_period()
    }
}
