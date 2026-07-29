use std::time::Duration;

#[cfg(test)]
use std::path::Path;

use crate::{
    CreateProfileOptions, ProfileIdentity,
    error::{Error, ErrorKind},
    model::ControllerModel,
    profile::{
        ProfileCreatePort, ProfileCreateTargetPort, ProfileCreateTargetState, ProfileDocument,
    },
    reporting::{self, ReportingMode},
    runtime::{
        cleanup::CloseMode,
        status::{StatusPublisher, status_projection},
    },
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

    fn require_supported_backend(
        self,
        ensure_supported: impl FnOnce(&BuilderConfig<M, R>) -> crate::Result<()>,
    ) -> crate::Result<BackendSupportedCreateProfilePlan<M, R>> {
        ensure_supported(&self.config)?;
        Ok(BackendSupportedCreateProfilePlan { plan: self })
    }
}

struct BackendSupportedCreateProfilePlan<M: ControllerModel, R: ReportingMode> {
    plan: CreateProfilePlan<M, R>,
}

impl<M: ControllerModel, R: ReportingMode> BackendSupportedCreateProfilePlan<M, R> {
    fn persist_and_reopen(
        self,
        store: &mut impl ProfileCreatePort,
    ) -> crate::Result<ReopenedCreateProfilePlan<M, R>> {
        let CreateProfilePlan {
            config,
            path,
            pair_timeout,
        } = self.plan;
        let document = ProfileDocument::empty_adapter_default::<M>();
        let bytes = document.to_json_bytes()?;
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

        let profile = read_typed_profile::<M>(store, &path)?;
        let config = config.finalize_with_profile(|configured_path| {
            debug_assert!(
                configured_path == path,
                "validated profile path changed before typed reopen"
            );
            Ok(profile)
        })?;

        Ok(ReopenedCreateProfilePlan {
            config,
            pair_timeout,
        })
    }
}

struct ReopenedCreateProfilePlan<M: ControllerModel, R: ReportingMode> {
    config: ControllerConfig<M, R>,
    pair_timeout: Duration,
}

/// Type-erased command and shutdown boundary for an owned controller runtime.
pub(super) trait ControllerRuntimePort<M, R>: Send
where
    M: ControllerModel,
    R: ReportingMode,
{
    /// Sends one reporting-specific command and waits for its worker response.
    fn request(
        &mut self,
        command: <R as reporting::sealed::Sealed>::Command<M>,
    ) -> crate::Result<()>;

    /// Performs explicit cleanup and consumes the runtime owner.
    fn close(self: Box<Self>, mode: CloseMode) -> crate::Result<()>;
}

/// Worker ownership retained from HCI-open through explicit close.
pub(super) struct ControllerRuntime<M: ControllerModel, R: ReportingMode> {
    port: Box<dyn ControllerRuntimePort<M, R>>,
}

impl<M: ControllerModel, R: ReportingMode> ControllerRuntime<M, R> {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds cannot construct a concrete runtime port"
        )
    )]
    pub(super) fn from_port(port: impl ControllerRuntimePort<M, R> + 'static) -> Self {
        Self {
            port: Box::new(port),
        }
    }

    pub(super) fn request(
        &mut self,
        command: <R as reporting::sealed::Sealed>::Command<M>,
    ) -> crate::Result<()> {
        self.port.request(command)
    }

    pub(super) fn close(self, mode: CloseMode) -> crate::Result<()> {
        self.port.close(mode)
    }

    #[cfg(test)]
    pub(super) fn new(owner: impl Send + 'static) -> Self {
        Self::from_port(TestRuntimeToken(owner))
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "the token is retained solely to verify runtime ownership and Drop"
)]
struct TestRuntimeToken<T: Send>(T);

#[cfg(test)]
impl<M, R, T> ControllerRuntimePort<M, R> for TestRuntimeToken<T>
where
    M: ControllerModel,
    R: ReportingMode,
    T: Send,
{
    fn request(
        &mut self,
        _command: <R as reporting::sealed::Sealed>::Command<M>,
    ) -> crate::Result<()> {
        Err(Error::new(
            ErrorKind::WorkerFailed,
            "test runtime token cannot process worker commands",
        ))
    }

    fn close(self: Box<Self>, _mode: CloseMode) -> crate::Result<()> {
        Ok(())
    }
}

/// Crate-private lifecycle seam used by the create-profile orchestrator.
///
/// Backend capability checks happen before persistence. Creating an attempt is
/// side-effect-free; the orchestrator retains that attempt through open and
/// pairing so either failure follows the same explicit cleanup path.
pub(super) trait CreateProfileRuntimeBackend<M: ControllerModel, R: ReportingMode> {
    type Attempt: CreateProfileRuntimeAttempt<M, R>;

    /// Checks backend availability without creating a profile or opening I/O.
    fn ensure_supported(&mut self, config: &BuilderConfig<M, R>) -> crate::Result<()>;

    /// Creates an inactive attempt that owns the shared status writer.
    fn begin_attempt(&mut self, status: StatusPublisher<M>) -> Self::Attempt;
}

/// One create-profile runtime attempt owned by the orchestrator.
pub(super) trait CreateProfileRuntimeAttempt<M: ControllerModel, R: ReportingMode>:
    Sized
{
    /// Opens runtime resources for the typed controller configuration.
    fn open(&mut self, config: &ControllerConfig<M, R>) -> crate::Result<()>;

    /// Completes pairing and returns only after protocol readiness.
    fn pair_to_ready(&mut self, pair_timeout: Duration) -> crate::Result<()>;

    /// Performs best-effort cleanup without sending a neutral report.
    ///
    /// Implementations retain the bounded drain used by explicit close and
    /// disarm their `Drop` fallback before returning. Resource-owning
    /// implementations also provide that fallback for panic and early-return
    /// paths.
    ///
    /// # Errors
    ///
    /// Returns a structured worker or transport error after attempting every
    /// cleanup phase.
    fn cleanup_without_neutral(self) -> crate::Result<()>;

    /// Transfers a successfully paired attempt into controller ownership.
    fn into_ready(self) -> ControllerRuntime<M, R>;
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
    let plan = plan.require_supported_backend(|config| backend.ensure_supported(config))?;
    let reopened = plan.persist_and_reopen(store)?;
    let (status_publisher, status_reader) = status_projection();
    let mut attempt = backend.begin_attempt(status_publisher.clone());
    if let Err(primary) = attempt.open(&reopened.config) {
        return Err(with_cleanup_error(
            primary,
            attempt.cleanup_without_neutral(),
        ));
    }
    if let Err(primary) = attempt.pair_to_ready(reopened.pair_timeout) {
        return Err(with_cleanup_error(
            primary,
            attempt.cleanup_without_neutral(),
        ));
    }
    let runtime = attempt.into_ready();

    Ok(Controller::from_ready_runtime(
        reopened.config,
        status_publisher,
        status_reader,
        runtime,
    ))
}

pub(super) fn reject_unavailable_backend<M, R>(
    plan: CreateProfilePlan<M, R>,
) -> crate::Result<Controller<M, R>>
where
    M: ControllerModel,
    R: ReportingMode,
{
    let _supported = plan.require_supported_backend(|_| {
        Err(crate::runtime::error_map::unsupported_capability(
            "Bluetooth transport",
        ))
    })?;

    Err(Error::new(
        ErrorKind::Internal,
        "unavailable backend passed its capability gate",
    ))
}

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
