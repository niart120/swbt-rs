use std::time::Duration;

#[cfg(test)]
use std::path::Path;

use crate::{
    CreateProfileOptions, ProfileIdentity,
    error::{Error, ErrorKind},
    model::ControllerModel,
    profile::{
        PairingProfile, ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState,
    },
    reporting::{self, ReportingMode},
    runtime::{
        cleanup::CloseMode,
        error_map::{map_command_error, map_enqueue_error, map_response_error, map_worker_outcome},
        status::{StatusPublisher, status_projection},
        worker::RuntimeCommand,
        worker_thread::WorkerOwner,
    },
};

use super::{
    Controller,
    config::{BuilderConfig, ControllerConfig},
};

pub(super) struct CreateProfilePlan<M: ControllerModel, R: ReportingMode> {
    config: BuilderConfig<M, R>,
    path: std::path::PathBuf,
    identity: ProfileIdentity,
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

    #[cfg(all(test, feature = "bumble"))]
    pub(super) const fn identity(&self) -> ProfileIdentity {
        self.identity
    }

    fn persist_and_configure(
        self,
        store: &mut impl ProfileCreatePort,
    ) -> crate::Result<(ControllerConfig<M, R>, Duration)> {
        let CreateProfilePlan {
            config,
            path,
            identity,
            pair_timeout,
        } = self;
        let profile = PairingProfile::<M>::empty(identity);
        let bytes = profile.to_json_bytes()?;
        store.create_new(&path, &bytes).map_err(|source| {
            let (kind, message) = if source.kind() == std::io::ErrorKind::AlreadyExists {
                (
                    ErrorKind::ProfileAlreadyExists,
                    "profile creation target already exists",
                )
            } else {
                (ErrorKind::Internal, "pairing profile could not be created")
            };
            Error::with_source(kind, message, source)
        })?;

        let config = config.finalize_with_profile(|configured_path| {
            debug_assert!(
                configured_path == path,
                "validated profile path changed before typed configuration"
            );
            Ok(profile)
        })?;

        Ok((config, pair_timeout))
    }
}

/// Worker ownership retained from HCI-open through explicit close.
pub(super) struct ControllerRuntime<M: ControllerModel, R: ReportingMode> {
    owner: WorkerOwner<RuntimeCommand<M, R>>,
}

impl<M: ControllerModel, R: ReportingMode> ControllerRuntime<M, R> {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds cannot construct a worker owner"
        )
    )]
    pub(super) const fn new(owner: WorkerOwner<RuntimeCommand<M, R>>) -> Self {
        Self { owner }
    }

    pub(super) fn request(
        &mut self,
        command: <R as reporting::sealed::Sealed>::Command<M>,
    ) -> crate::Result<()> {
        let response = self
            .owner
            .try_enqueue(RuntimeCommand::Input(command))
            .map_err(map_enqueue_error)?;
        response
            .recv()
            .map_err(map_response_error)?
            .map_err(map_command_error)
    }

    pub(super) fn pair(&mut self, timeout: Duration) -> crate::Result<()> {
        let response = self
            .owner
            .try_enqueue(RuntimeCommand::Pair { timeout })
            .map_err(map_enqueue_error)?;
        response
            .recv()
            .map_err(map_response_error)?
            .map_err(map_command_error)
    }

    pub(super) fn reconnect(&mut self, timeout: Duration) -> crate::Result<()> {
        let response = self
            .owner
            .try_enqueue(RuntimeCommand::Reconnect { timeout })
            .map_err(map_enqueue_error)?;
        response
            .recv()
            .map_err(map_response_error)?
            .map_err(map_command_error)
    }

    pub(super) fn close(self, mode: CloseMode) -> crate::Result<()> {
        map_worker_outcome(self.owner.finish_explicit(mode))
    }
}

pub(super) fn create_profile<M, R>(
    plan: CreateProfilePlan<M, R>,
    store: &mut impl ProfileCreatePort,
    open_and_pair: impl FnOnce(
        &ControllerConfig<M, R>,
        StatusPublisher<M>,
        Duration,
    ) -> crate::Result<ControllerRuntime<M, R>>,
) -> crate::Result<Controller<M, R>>
where
    M: ControllerModel,
    R: ReportingMode,
{
    let (config, pair_timeout) = plan.persist_and_configure(store)?;
    let (status_publisher, status_reader) = status_projection::<M, R>();
    let runtime = open_and_pair(&config, status_publisher.clone(), pair_timeout)?;

    Ok(Controller::from_ready_runtime(
        config,
        status_publisher,
        status_reader,
        runtime,
    ))
}

#[cfg(not(feature = "bumble"))]
pub(super) fn reject_unavailable_backend<M, R>(
    plan: CreateProfilePlan<M, R>,
) -> crate::Result<Controller<M, R>>
where
    M: ControllerModel,
    R: ReportingMode,
{
    let _ = plan;
    Err(crate::runtime::error_map::unsupported_capability(
        "Bluetooth transport",
    ))
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not aggregate runtime cleanup"
    )
)]
pub(super) fn with_cleanup_error(primary: Error, cleanup: crate::Result<()>) -> Error {
    match cleanup {
        Ok(()) => primary,
        Err(cleanup) => primary.with_related(cleanup),
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

    if matches!(options.identity, ProfileIdentity::LocalAddress(_)) {
        #[cfg(not(feature = "bumble"))]
        {
            return Err(Error::new(
                ErrorKind::UnsupportedCapability,
                "explicit local address profiles require the Bumble transport",
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
            identity: options.identity,
            pair_timeout: options.pair_timeout,
        }),
        ProfileCreateTargetState::Existing => Err(Error::new(
            ErrorKind::ProfileAlreadyExists,
            "profile creation target already exists",
        )),
    }
}
