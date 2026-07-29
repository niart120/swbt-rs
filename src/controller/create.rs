use std::{marker::PhantomData, time::Duration};

#[cfg(test)]
use std::path::Path;

use crate::{
    CreateProfileOptions, ProfileIdentity,
    error::{Error, ErrorKind},
    model::ControllerModel,
    profile::{
        ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState, ProfileDocument,
    },
    reporting::ReportingMode,
    runtime::status::{StatusPublisher, status_projection},
};

use super::{
    Controller,
    build::read_typed_profile,
    config::{BuilderConfig, ControllerConfig},
};

pub(super) struct CreateProfilePlan<M: ControllerModel, R: ReportingMode> {
    config: BuilderConfig<M, R>,
    path: std::path::PathBuf,
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

    fn persist_and_reopen(
        self,
        store: &mut impl ProfileCreatePort,
    ) -> crate::Result<ReopenedCreateProfilePlan<M, R>> {
        let document = ProfileDocument::empty_adapter_default::<M>();
        let bytes = document.to_json_bytes()?;
        store.create_new(&self.path, &bytes).map_err(|source| {
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

        let profile = read_typed_profile::<M>(store, &self.path)?;
        let path = self.path;
        let config = self.config.finalize_with_profile(|configured_path| {
            debug_assert!(
                configured_path == path,
                "validated profile path changed before typed reopen"
            );
            Ok(profile)
        })?;

        Ok(ReopenedCreateProfilePlan {
            config,
            pair_timeout: self.pair_timeout,
        })
    }
}

struct ReopenedCreateProfilePlan<M: ControllerModel, R: ReportingMode> {
    config: ControllerConfig<M, R>,
    pair_timeout: Duration,
}

trait RuntimeOwner: Send {}

impl<T: Send> RuntimeOwner for T {}

/// Runtime ownership returned only after a backend reports protocol readiness.
pub(super) struct ReadyRuntime<M: ControllerModel, R: ReportingMode> {
    _owner: Box<dyn RuntimeOwner>,
    _types: PhantomData<fn() -> (M, R)>,
}

impl<M: ControllerModel, R: ReportingMode> ReadyRuntime<M, R> {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T33 constructs a ready runtime from the concrete worker owner"
        )
    )]
    pub(super) fn new(owner: impl Send + 'static) -> Self {
        Self {
            _owner: Box::new(owner),
            _types: PhantomData,
        }
    }
}

/// Crate-private lifecycle seam used by the create-profile orchestrator.
///
/// A successful `pair_to_ready` call must return only after the shared status
/// projection represents a protocol-ready controller.
pub(super) trait CreateProfileRuntimeBackend<M: ControllerModel, R: ReportingMode> {
    type Opened;

    /// Checks backend availability without creating a profile or opening I/O.
    fn ensure_supported(&mut self, config: &BuilderConfig<M, R>) -> crate::Result<()>;

    /// Opens the configured runtime and takes a clone of its status writer.
    fn open(
        &mut self,
        config: &ControllerConfig<M, R>,
        status: StatusPublisher<M>,
    ) -> crate::Result<Self::Opened>;

    /// Pairs an opened runtime and returns ownership only after readiness.
    fn pair_to_ready(
        &mut self,
        opened: Self::Opened,
        pair_timeout: Duration,
    ) -> crate::Result<ReadyRuntime<M, R>>;
}

pub(super) fn create_profile<M, R>(
    plan: CreateProfilePlan<M, R>,
    store: &mut impl ProfileCreatePort,
    backend: &mut impl CreateProfileRuntimeBackend<M, R>,
) -> crate::Result<Controller<M, R>>
where
    M: ControllerModel,
    R: ReportingMode,
{
    backend.ensure_supported(&plan.config)?;
    let reopened = plan.persist_and_reopen(store)?;
    let (status_publisher, status_reader) = status_projection();
    let opened = backend.open(&reopened.config, status_publisher.clone())?;
    let runtime = backend.pair_to_ready(opened, reopened.pair_timeout)?;

    Ok(Controller::from_ready_runtime(
        reopened.config,
        status_publisher,
        status_reader,
        runtime,
    ))
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
