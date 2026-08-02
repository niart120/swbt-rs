use crate::{
    error::{Error, ErrorKind},
    runtime::{
        cleanup::{CleanupFailure, ExplicitCloseError},
        command::{CommandDeliveryError, CommandEnqueueError, CommandResponseError},
        direct::{DirectTapError, DirectTapInterruption},
        lifecycle::LifecycleCommandError,
        periodic::PeriodicError,
        readiness::ReadinessError,
        worker::{
            ConnectionCommandError, ConnectionCommandFailure, ConnectionCommandKind,
            WorkerCommandError, WorkerCoreError,
        },
        worker_thread::{WorkerFailureCause, WorkerJoinError, WorkerThreadOutcome},
    },
};

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not enqueue worker commands"
    )
)]
pub(crate) fn map_enqueue_error(error: CommandEnqueueError) -> Error {
    match error {
        CommandEnqueueError::InvariantViolation => Error::new(
            ErrorKind::Internal,
            "worker command slot was unexpectedly occupied",
        ),
        CommandEnqueueError::Disconnected => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker is no longer accepting commands",
        ),
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not await worker responses"
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
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not deliver worker commands"
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
        WorkerCommandError::Connection(error) => map_connection_error(error),
        WorkerCommandError::Direct(DirectTapError::NotReady)
        | WorkerCommandError::Periodic(PeriodicError::NotReady) => Error::new(
            ErrorKind::TransportClosed,
            "controller input requires a ready transport",
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
        WorkerCommandError::Disconnected => Error::new(
            ErrorKind::Internal,
            "an unclassified disconnect interrupted the operation",
        ),
    }
}

fn map_connection_error(error: ConnectionCommandError) -> Error {
    let ConnectionCommandError { command, failure } = error;
    match failure {
        ConnectionCommandFailure::Begin(WorkerCoreError::Transport(source))
            if command == ConnectionCommandKind::Reconnect
                && source.kind() == crate::runtime::transport::TransportErrorKind::NoBond =>
        {
            Error::with_source(
                ErrorKind::NoBond,
                "controller profile has no usable Classic bond",
                source,
            )
        }
        ConnectionCommandFailure::Begin(error) => map_worker_core_error(error),
        ConnectionCommandFailure::Readiness(error) => {
            map_connection_readiness_error(command, error)
        }
        ConnectionCommandFailure::InvalidKeyStore => Error::new(
            ErrorKind::InvalidKeyStore,
            "controller pairing key store could not be read or updated",
        ),
        ConnectionCommandFailure::WorkerFailed => match command {
            ConnectionCommandKind::Pair => Error::new(
                ErrorKind::WorkerFailed,
                "controller worker terminated during pairing",
            ),
            ConnectionCommandKind::Reconnect => Error::new(
                ErrorKind::WorkerFailed,
                "controller worker terminated during reconnect",
            ),
        },
    }
}

fn map_connection_readiness_error(command: ConnectionCommandKind, error: ReadinessError) -> Error {
    let (kind, message) = match (command, &error) {
        (ConnectionCommandKind::Pair, ReadinessError::TimedOut) => {
            (ErrorKind::ConnectionTimeout, "controller pairing timed out")
        }
        (ConnectionCommandKind::Reconnect, ReadinessError::TimedOut) => (
            ErrorKind::ConnectionTimeout,
            "controller reconnect timed out",
        ),
        (ConnectionCommandKind::Pair, ReadinessError::Disconnected { .. }) => (
            ErrorKind::ConnectionFailed,
            "controller disconnected before pairing completed",
        ),
        (ConnectionCommandKind::Reconnect, ReadinessError::Disconnected { .. }) => (
            ErrorKind::ConnectionFailed,
            "controller disconnected before reconnect completed",
        ),
        (
            ConnectionCommandKind::Pair,
            ReadinessError::StaleSession { .. } | ReadinessError::HandshakeSessionMismatch { .. },
        ) => (
            ErrorKind::Protocol,
            "controller pairing protocol state was inconsistent",
        ),
        (
            ConnectionCommandKind::Reconnect,
            ReadinessError::StaleSession { .. } | ReadinessError::HandshakeSessionMismatch { .. },
        ) => (
            ErrorKind::Protocol,
            "controller reconnect protocol state was inconsistent",
        ),
        (ConnectionCommandKind::Pair, ReadinessError::Scheduler(_)) => (
            ErrorKind::ConnectionFailed,
            "controller pairing readiness failed",
        ),
        (ConnectionCommandKind::Reconnect, ReadinessError::Scheduler(_)) => (
            ErrorKind::ConnectionFailed,
            "controller reconnect readiness failed",
        ),
    };
    Error::with_source(kind, message, error)
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not join controller workers"
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
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not observe worker outcomes"
    )
)]
pub(crate) fn map_worker_failure(cause: WorkerFailureCause) -> Error {
    match cause {
        WorkerFailureCause::Core(error) => map_worker_core_error(error),
        WorkerFailureCause::Wait(_) => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker wait source terminated",
        ),
        WorkerFailureCause::CommandDelivery(error) => map_delivery_error(error),
        WorkerFailureCause::Panicked => {
            Error::new(ErrorKind::WorkerFailed, "controller worker thread panicked")
        }
        WorkerFailureCause::CompletionDisconnected => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker completion channel disconnected",
        ),
    }
}

#[cfg_attr(
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not consume worker outcomes"
    )
)]
pub(crate) fn map_worker_outcome(outcome: WorkerThreadOutcome) -> crate::Result<()> {
    match outcome {
        WorkerThreadOutcome::Closed {
            result,
            delivery_error,
        } => {
            let (cleanup_error, join_error) = match result {
                Ok(()) => (None, None),
                Err(ExplicitCloseError::Cleanup(cleanup)) => (Some(cleanup), None),
                Err(ExplicitCloseError::Join(join)) => (None, Some(join)),
                Err(ExplicitCloseError::CleanupAndJoin { cleanup, join }) => {
                    (Some(cleanup), Some(join))
                }
            };
            let mut errors = [
                cleanup_error.map(map_cleanup_error),
                delivery_error.map(map_delivery_error),
                join_error.map(map_join_error),
            ]
            .into_iter()
            .flatten();
            let Some(first) = errors.next() else {
                return Ok(());
            };
            let error = errors.fold(first, Error::with_related);
            Err(error)
        }
        WorkerThreadOutcome::Failed {
            cause,
            delivery_error,
            cleanup_error,
            join_error,
        } => {
            let mut error = map_worker_failure(cause);
            if let Some(delivery) = delivery_error {
                error = error.with_related(map_delivery_error(delivery));
            }
            if let Some(cleanup) = cleanup_error {
                error = error.with_related(map_cleanup_error(cleanup));
            }
            if let Some(join) = join_error {
                error = error.with_related(map_join_error(join));
            }
            Err(error)
        }
    }
}

pub(crate) fn map_cleanup_error(error: CleanupFailure) -> Error {
    Error::with_source(
        ErrorKind::WorkerFailed,
        "controller runtime cleanup failed",
        error,
    )
}

fn map_delivery_error(error: CommandDeliveryError) -> Error {
    match error {
        CommandDeliveryError::MissingResponse => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker could not deliver a command result",
        ),
    }
}

fn map_worker_core_error(error: WorkerCoreError) -> Error {
    match error {
        WorkerCoreError::InvalidLifecycle => Error::new(
            ErrorKind::WorkerFailed,
            "controller worker entered an invalid lifecycle state",
        ),
        WorkerCoreError::Handshake(source) => Error::with_source(
            ErrorKind::WorkerFailed,
            "controller worker handshake failed",
            source,
        ),
        WorkerCoreError::Transport(source)
            if source.kind() == crate::runtime::transport::TransportErrorKind::InvalidKeyStore =>
        {
            Error::with_source(
                ErrorKind::InvalidKeyStore,
                "controller pairing key store could not be read or updated",
                source,
            )
        }
        WorkerCoreError::Transport(source) => Error::with_source(
            ErrorKind::WorkerFailed,
            "controller worker transport failed",
            source,
        ),
    }
}

#[cfg(any(test, not(feature = "bumble")))]
pub(crate) fn unsupported_capability(capability: &str) -> Error {
    Error::new(
        ErrorKind::UnsupportedCapability,
        format!("{capability} is unavailable in this build"),
    )
}

#[cfg(test)]
mod tests {
    use std::{error::Error as StdError, io, sync::Arc};

    use crate::{
        error::ErrorKind,
        runtime::{
            cleanup::{CleanupFailure, CleanupPhase, ExplicitCloseError},
            command::CommandDeliveryError,
            direct::DirectTapError,
            periodic::PeriodicError,
            readiness::ReadinessError,
            scheduler::SchedulerError,
            transport::{TransportError, TransportErrorKind},
            worker::{
                ConnectionCommandError, ConnectionCommandFailure, ConnectionCommandKind,
                WorkerCommandError, WorkerCoreError,
            },
            worker_thread::{WorkerFailureCause, WorkerJoinError, WorkerThreadOutcome},
        },
    };

    use super::{map_command_error, map_worker_failure, map_worker_outcome};

    #[test]
    fn key_store_transport_terminal_maps_to_the_public_profile_category() {
        let error = map_worker_failure(WorkerFailureCause::Core(WorkerCoreError::Transport(
            TransportError::new(TransportErrorKind::InvalidKeyStore),
        )));

        assert_eq!(error.kind(), ErrorKind::InvalidKeyStore);
        assert_eq!(
            error.to_string(),
            "controller pairing key store could not be read or updated"
        );
        let debug = format!("{error:?}");
        assert!(debug.contains("InvalidKeyStore"));
        assert!(!debug.contains("TransportError"));
    }

    #[test]
    fn direct_and_periodic_not_ready_map_to_transport_closed() {
        for error in [
            WorkerCommandError::Direct(DirectTapError::NotReady),
            WorkerCommandError::Periodic(PeriodicError::NotReady),
        ] {
            let error = map_command_error(error);

            assert_eq!(error.kind(), ErrorKind::TransportClosed);
            assert_eq!(
                error.to_string(),
                "controller input requires a ready transport"
            );
            assert!(error.source().is_none());
        }
    }

    #[test]
    fn connection_readiness_errors_map_to_recoverable_kinds_and_keep_their_source() {
        for command in [
            ConnectionCommandKind::Pair,
            ConnectionCommandKind::Reconnect,
        ] {
            for (readiness, expected_kind) in [
                (ReadinessError::TimedOut, ErrorKind::ConnectionTimeout),
                (
                    ReadinessError::Disconnected { reason: Some(0x13) },
                    ErrorKind::ConnectionFailed,
                ),
                (
                    ReadinessError::Scheduler(SchedulerError::DeadlineOverflow),
                    ErrorKind::ConnectionFailed,
                ),
            ] {
                let error =
                    map_command_error(WorkerCommandError::Connection(ConnectionCommandError {
                        command,
                        failure: ConnectionCommandFailure::Readiness(readiness),
                    }));

                assert_eq!(error.kind(), expected_kind);
                assert!(
                    error
                        .source()
                        .and_then(|source| source.downcast_ref::<ReadinessError>())
                        .is_some()
                );
            }
        }
    }

    #[test]
    fn no_bond_category_is_reconnect_only_and_keeps_its_source() {
        let pair_error =
            map_command_error(WorkerCommandError::Connection(ConnectionCommandError {
                command: ConnectionCommandKind::Pair,
                failure: ConnectionCommandFailure::Begin(WorkerCoreError::Transport(
                    TransportError::new(TransportErrorKind::NoBond),
                )),
            }));
        assert_eq!(pair_error.kind(), ErrorKind::WorkerFailed);
        assert!(
            pair_error
                .source()
                .and_then(|source| source.downcast_ref::<TransportError>())
                .is_some()
        );

        let reconnect_error =
            map_command_error(WorkerCommandError::Connection(ConnectionCommandError {
                command: ConnectionCommandKind::Reconnect,
                failure: ConnectionCommandFailure::Begin(WorkerCoreError::Transport(
                    TransportError::new(TransportErrorKind::NoBond),
                )),
            }));
        assert_eq!(reconnect_error.kind(), ErrorKind::NoBond);
        assert!(
            reconnect_error
                .source()
                .and_then(|source| source.downcast_ref::<TransportError>())
                .is_some()
        );
    }

    #[test]
    fn closed_cleanup_and_join_keep_cleanup_primary_and_join_related() {
        let cleanup = CleanupFailure::new(
            CleanupPhase::Disconnect,
            TransportError::with_source(
                TransportErrorKind::SourceTerminated,
                Arc::new(io::Error::other("secret cleanup source")),
            ),
        );
        let outcome = WorkerThreadOutcome::Closed {
            result: Err(ExplicitCloseError::CleanupAndJoin {
                cleanup,
                join: WorkerJoinError::Panicked,
            }),
            delivery_error: None,
        };

        let error = map_worker_outcome(outcome).expect_err("cleanup and join failed");

        assert_eq!(error.kind(), ErrorKind::WorkerFailed);
        assert_eq!(error.to_string(), "controller runtime cleanup failed");
        let cleanup = error
            .source()
            .and_then(|source| source.downcast_ref::<CleanupFailure>())
            .expect("typed cleanup source");
        assert_eq!(cleanup.phase(), CleanupPhase::Disconnect);
        assert_eq!(cleanup.to_string(), "controller runtime cleanup failed");
        assert!(!format!("{cleanup:?}").contains("secret"));
        let transport = cleanup
            .source()
            .and_then(|source| source.downcast_ref::<TransportError>())
            .expect("cleanup transport source");
        assert_eq!(transport.kind(), TransportErrorKind::SourceTerminated);
        assert_eq!(
            transport.source().expect("backend source").to_string(),
            "secret cleanup source"
        );
        let join = error.related_error().expect("related join error");
        assert_eq!(join.kind(), ErrorKind::WorkerFailed);
        assert_eq!(join.to_string(), "controller worker thread panicked");
        assert!(join.related_error().is_none());
        assert!(!error.to_string().contains("secret"));
        assert!(!format!("{error:?}").contains("secret"));
    }

    #[test]
    fn closed_outcome_orders_cleanup_delivery_then_join_by_occurrence() {
        let outcome = WorkerThreadOutcome::Closed {
            result: Err(ExplicitCloseError::CleanupAndJoin {
                cleanup: CleanupFailure::new(
                    CleanupPhase::Disconnect,
                    TransportError::new(TransportErrorKind::Closed),
                ),
                join: WorkerJoinError::Panicked,
            }),
            delivery_error: Some(CommandDeliveryError::MissingResponse),
        };

        let error = map_worker_outcome(outcome).expect_err("cleanup, delivery, and join failed");

        assert_eq!(error.to_string(), "controller runtime cleanup failed");
        let delivery = error.related_error().expect("related delivery error");
        assert_eq!(
            delivery.to_string(),
            "controller worker could not deliver a command result"
        );
        let join = delivery.related_error().expect("related join error");
        assert_eq!(join.to_string(), "controller worker thread panicked");
        assert!(join.related_error().is_none());
    }

    #[test]
    fn closed_outcome_orders_missing_response_before_join_without_cleanup() {
        let outcome = WorkerThreadOutcome::Closed {
            result: Err(ExplicitCloseError::Join(WorkerJoinError::Panicked)),
            delivery_error: Some(CommandDeliveryError::MissingResponse),
        };

        let error = map_worker_outcome(outcome).expect_err("delivery and join failed");

        assert_eq!(
            error.to_string(),
            "controller worker could not deliver a command result"
        );
        let join = error.related_error().expect("related join error");
        assert_eq!(join.to_string(), "controller worker thread panicked");
        assert!(join.related_error().is_none());
    }

    #[test]
    fn failed_outcome_appends_delivery_cleanup_then_join_to_the_primary_cause() {
        let outcome = WorkerThreadOutcome::Failed {
            cause: WorkerFailureCause::Core(WorkerCoreError::Transport(TransportError::new(
                TransportErrorKind::Closed,
            ))),
            delivery_error: Some(CommandDeliveryError::MissingResponse),
            cleanup_error: Some(CleanupFailure::new(
                CleanupPhase::Disconnect,
                TransportError::new(TransportErrorKind::Closed),
            )),
            join_error: Some(WorkerJoinError::Panicked),
        };

        let error = map_worker_outcome(outcome).expect_err("worker failed");

        assert_eq!(error.to_string(), "controller worker transport failed");
        assert!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<TransportError>())
                .is_some()
        );
        let delivery = error.related_error().expect("related delivery error");
        assert_eq!(
            delivery.to_string(),
            "controller worker could not deliver a command result"
        );
        let cleanup = delivery.related_error().expect("related cleanup error");
        assert_eq!(cleanup.to_string(), "controller runtime cleanup failed");
        let join = cleanup.related_error().expect("related join error");
        assert_eq!(join.to_string(), "controller worker thread panicked");
        assert!(join.related_error().is_none());
    }

    #[test]
    fn clean_closed_outcome_maps_to_success() {
        assert!(
            map_worker_outcome(WorkerThreadOutcome::Closed {
                result: Ok(()),
                delivery_error: None,
            })
            .is_ok()
        );
    }
}
