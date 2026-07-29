use std::time::Duration;

#[cfg(test)]
use std::path::Path;

use crate::{
    CreateProfileOptions, ProfileIdentity,
    error::{Error, ErrorKind},
    model::ControllerModel,
    profile::{ProfileCreateTargetPort, ProfileCreateTargetState},
    reporting::ReportingMode,
};

use super::config::BuilderConfig;

pub(super) struct CreateProfilePlan<M: ControllerModel, R: ReportingMode> {
    #[allow(
        dead_code,
        reason = "T31 consumes the validated builder settings during create-profile orchestration"
    )]
    config: BuilderConfig<M, R>,
    #[allow(
        dead_code,
        reason = "T31 creates and reopens the profile at the validated target path"
    )]
    path: std::path::PathBuf,
    #[allow(
        dead_code,
        reason = "T31 applies the caller's timeout to fake pairing readiness"
    )]
    pair_timeout: Duration,
}

impl<M: ControllerModel, R: ReportingMode> CreateProfilePlan<M, R> {
    #[cfg(test)]
    pub(super) fn profile_path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub(super) const fn pair_timeout(&self) -> Duration {
        self.pair_timeout
    }
}

pub(super) fn validate_target<M: ControllerModel, R: ReportingMode>(
    config: BuilderConfig<M, R>,
    options: CreateProfileOptions,
    target: &mut impl ProfileCreateTargetPort,
) -> crate::Result<CreateProfilePlan<M, R>> {
    let path = config.profile_path().ok_or_else(|| {
        Error::new(
            ErrorKind::ProfilePathRequired,
            "profile creation requires a target path",
        )
    })?;

    match options.identity {
        ProfileIdentity::AdapterDefault => {}
        ProfileIdentity::LocalAddress(_) => {
            return Err(Error::new(
                ErrorKind::UnsupportedCapability,
                "explicit local address profiles are not supported",
            ));
        }
    }

    match target.inspect(path).map_err(|source| {
        Error::with_source(
            ErrorKind::Internal,
            "profile creation target could not be inspected",
            source,
        )
    })? {
        ProfileCreateTargetState::Absent => Ok(CreateProfilePlan {
            path: path.to_owned(),
            config,
            pair_timeout: options.pair_timeout,
        }),
        ProfileCreateTargetState::Existing => Err(Error::new(
            ErrorKind::ProfileAlreadyExists,
            "profile creation target already exists",
        )),
    }
}
