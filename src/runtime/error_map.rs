use crate::{
    error::{Error, ErrorKind},
    runtime::{
        command::{CommandEnqueueError, CommandResponseError},
        direct::{DirectTapError, DirectTapInterruption},
        lifecycle::LifecycleCommandError,
        worker::{WorkerCommandError, WorkerCoreError},
        worker_thread::{WorkerFailureCause, WorkerJoinError},
    },
};

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 controller orchestration maps bounded command enqueue failures"
    )
)]
pub(crate) fn map_enqueue_error(error: CommandEnqueueError) -> Error {
    match error {
        CommandEnqueueError::Busy => Error::new(ErrorKind::Busy, "worker command queue is full"),
        CommandEnqueueError::Disconnected => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker is no longer accepting commands",
        ),
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 controller orchestration maps worker response termination"
    )
)]
pub(crate) fn map_response_error(error: CommandResponseError) -> Error {
    match error {
        CommandResponseError::WorkerFailed => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker terminated before responding",
        ),
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 controller methods map typed worker command failures"
    )
)]
pub(crate) fn map_command_error(error: WorkerCommandError) -> Error {
    match error {
        WorkerCommandError::Input(error) => error,
        WorkerCommandError::Lifecycle(LifecycleCommandError::Shutdown)
        | WorkerCommandError::Shutdown
        | WorkerCommandError::Direct(DirectTapError::Interrupted(
            DirectTapInterruption::Shutdown,
        )) => Error::new(
            ErrorKind::Shutdown,
            "controller shutdown interrupted the operation",
        ),
        WorkerCommandError::Direct(DirectTapError::NotReady) => Error::new(
            ErrorKind::TransportClosed,
            "direct input requires a ready transport",
        ),
        WorkerCommandError::Direct(error) => Error::with_source(
            ErrorKind::Internal,
            "an unclassified direct input operation failed",
            error,
        ),
        WorkerCommandError::Periodic(error) => Error::with_source(
            ErrorKind::Internal,
            "an unclassified periodic input operation failed",
            error,
        ),
        WorkerCommandError::Lifecycle(LifecycleCommandError::Failed) => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker has entered a failed state",
        ),
        WorkerCommandError::ClockOverflow => {
            Error::new(ErrorKind::Internal, "worker monotonic clock overflowed")
        }
        WorkerCommandError::DeadlineOverflow => {
            Error::new(ErrorKind::Internal, "worker operation deadline overflowed")
        }
        WorkerCommandError::Disconnected { .. } => Error::new(
            ErrorKind::Internal,
            "an unclassified disconnect interrupted the operation",
        ),
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 controller orchestration maps a joined worker panic"
    )
)]
pub(crate) fn map_join_error(error: WorkerJoinError) -> Error {
    match error {
        WorkerJoinError::Panicked => {
            Error::new(ErrorKind::WorkerFailed, "controller worker thread panicked")
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T31 controller orchestration maps terminal worker outcomes"
    )
)]
pub(crate) fn map_worker_failure(cause: WorkerFailureCause) -> Error {
    match cause {
        WorkerFailureCause::Core(error) => map_worker_core_error(error),
        WorkerFailureCause::Wait(_) => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker wait source terminated",
        ),
        WorkerFailureCause::CommandDelivery(_) => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker could not deliver a command result",
        ),
        WorkerFailureCause::Panicked => {
            Error::new(ErrorKind::WorkerFailed, "controller worker thread panicked")
        }
        WorkerFailureCause::CompletionDisconnected => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker completion channel disconnected",
        ),
    }
}

fn map_worker_core_error(error: WorkerCoreError) -> Error {
    match error {
        WorkerCoreError::DeadlineOverflow => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker deadline overflowed",
        ),
        WorkerCoreError::InvalidLifecycle => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker entered an invalid lifecycle state",
        ),
        WorkerCoreError::Session(source) => Error::with_source(
            ErrorKind::WorkerFailed,
            "controller worker session failed",
            source,
        ),
        WorkerCoreError::Handshake(source) => Error::with_source(
            ErrorKind::WorkerFailed,
            "controller worker handshake failed",
            source,
        ),
        WorkerCoreError::Transport(source) => Error::with_source(
            ErrorKind::WorkerFailed,
            "controller worker transport failed",
            source,
        ),
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T34 reports concrete backend absence through the public error boundary"
    )
)]
pub(crate) fn unsupported_capability(capability: &str) -> Error {
    Error::new(
        ErrorKind::UnsupportedCapability,
        format!("{capability} is unavailable in this build"),
    )
}
