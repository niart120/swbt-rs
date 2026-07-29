use crate::runtime::readiness::ReadySession;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleState {
    Configured,
    Open,
    Connecting,
    Ready,
    Closing,
    Closed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleAction {
    OpenTransport,
    Opened,
    BeginCleanup,
    Closed,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleCommandError {
    Shutdown,
    Failed,
}

pub(crate) struct LifecycleStateMachine {
    state: LifecycleState,
    opening: bool,
}

impl LifecycleStateMachine {
    pub(crate) const fn new() -> Self {
        Self {
            state: LifecycleState::Configured,
            opening: false,
        }
    }

    #[must_use]
    pub(crate) const fn state(&self) -> LifecycleState {
        self.state
    }

    pub(crate) fn request_open(&mut self) -> Result<LifecycleAction, LifecycleCommandError> {
        match self.state {
            LifecycleState::Configured | LifecycleState::Closed => {
                if self.opening {
                    Ok(LifecycleAction::None)
                } else {
                    self.opening = true;
                    Ok(LifecycleAction::OpenTransport)
                }
            }
            LifecycleState::Open | LifecycleState::Connecting | LifecycleState::Ready => {
                Ok(LifecycleAction::None)
            }
            LifecycleState::Closing => Err(LifecycleCommandError::Shutdown),
            LifecycleState::Failed => Err(LifecycleCommandError::Failed),
        }
    }

    pub(crate) fn complete_open(&mut self) -> LifecycleAction {
        match self.state {
            LifecycleState::Configured | LifecycleState::Closed if self.opening => {
                self.opening = false;
                self.state = LifecycleState::Open;
                LifecycleAction::Opened
            }
            LifecycleState::Configured | LifecycleState::Closed => LifecycleAction::None,
            LifecycleState::Open
            | LifecycleState::Connecting
            | LifecycleState::Ready
            | LifecycleState::Closing
            | LifecycleState::Failed => LifecycleAction::None,
        }
    }

    #[allow(
        dead_code,
        reason = "T31 and T32 controller orchestration handles transport open failure"
    )]
    pub(crate) fn fail_open(&mut self) {
        self.opening = false;
    }

    pub(crate) fn mark_failed(&mut self) {
        self.opening = false;
        self.state = LifecycleState::Failed;
    }

    pub(crate) fn begin_connection(&mut self) -> bool {
        if self.state != LifecycleState::Open {
            return false;
        }
        self.state = LifecycleState::Connecting;
        true
    }

    pub(crate) fn mark_ready(&mut self, _ready: ReadySession) -> bool {
        if self.state != LifecycleState::Connecting {
            return false;
        }
        self.state = LifecycleState::Ready;
        true
    }

    pub(crate) fn mark_connection_ended(&mut self) -> bool {
        if !matches!(
            self.state,
            LifecycleState::Connecting | LifecycleState::Ready
        ) {
            return false;
        }
        self.state = LifecycleState::Open;
        true
    }

    pub(crate) fn request_close(&mut self) -> LifecycleAction {
        match self.state {
            LifecycleState::Configured => {
                if self.opening {
                    self.opening = false;
                    self.state = LifecycleState::Closing;
                    LifecycleAction::BeginCleanup
                } else {
                    self.state = LifecycleState::Closed;
                    LifecycleAction::Closed
                }
            }
            LifecycleState::Closed => {
                if self.opening {
                    self.opening = false;
                    self.state = LifecycleState::Closing;
                    LifecycleAction::BeginCleanup
                } else {
                    LifecycleAction::None
                }
            }
            LifecycleState::Open
            | LifecycleState::Connecting
            | LifecycleState::Ready
            | LifecycleState::Failed => {
                self.opening = false;
                self.state = LifecycleState::Closing;
                LifecycleAction::BeginCleanup
            }
            LifecycleState::Closing => LifecycleAction::None,
        }
    }

    pub(crate) fn complete_close(&mut self) -> LifecycleAction {
        match self.state {
            LifecycleState::Closing => {
                self.opening = false;
                self.state = LifecycleState::Closed;
                LifecycleAction::Closed
            }
            LifecycleState::Configured
            | LifecycleState::Open
            | LifecycleState::Connecting
            | LifecycleState::Ready
            | LifecycleState::Closed
            | LifecycleState::Failed => LifecycleAction::None,
        }
    }

    pub(crate) fn ensure_input_command(&self) -> Result<(), LifecycleCommandError> {
        match self.state {
            LifecycleState::Closing | LifecycleState::Closed => {
                Err(LifecycleCommandError::Shutdown)
            }
            LifecycleState::Failed => Err(LifecycleCommandError::Failed),
            LifecycleState::Configured
            | LifecycleState::Open
            | LifecycleState::Connecting
            | LifecycleState::Ready => Ok(()),
        }
    }
}

impl Default for LifecycleStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{LifecycleAction, LifecycleCommandError, LifecycleState, LifecycleStateMachine};

    #[test]
    fn open_close_and_reopen_follow_idempotent_transitions() {
        let mut lifecycle = LifecycleStateMachine::new();

        assert_eq!(lifecycle.state(), LifecycleState::Configured);
        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::OpenTransport));
        assert_eq!(lifecycle.state(), LifecycleState::Configured);
        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::None));
        lifecycle.fail_open();
        assert_eq!(lifecycle.state(), LifecycleState::Configured);
        assert_eq!(lifecycle.complete_open(), LifecycleAction::None);
        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::OpenTransport));
        assert_eq!(lifecycle.complete_open(), LifecycleAction::Opened);
        assert_eq!(lifecycle.state(), LifecycleState::Open);
        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::None));
        assert_eq!(lifecycle.state(), LifecycleState::Open);

        assert_eq!(lifecycle.request_close(), LifecycleAction::BeginCleanup);
        assert_eq!(lifecycle.state(), LifecycleState::Closing);
        assert_eq!(lifecycle.request_close(), LifecycleAction::None);
        assert_eq!(lifecycle.state(), LifecycleState::Closing);

        assert_eq!(lifecycle.complete_close(), LifecycleAction::Closed);
        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        assert_eq!(lifecycle.complete_close(), LifecycleAction::None);
        assert_eq!(lifecycle.request_close(), LifecycleAction::None);

        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::OpenTransport));
        assert_eq!(lifecycle.state(), LifecycleState::Closed);
        assert_eq!(lifecycle.complete_open(), LifecycleAction::Opened);
        assert_eq!(lifecycle.state(), LifecycleState::Open);
    }

    #[test]
    fn commands_after_close_begins_are_shutdown_until_reopen() {
        let mut lifecycle = LifecycleStateMachine::new();
        lifecycle.request_open().expect("initial open");
        lifecycle.complete_open();
        lifecycle
            .ensure_input_command()
            .expect("input before close begins");

        assert_eq!(lifecycle.request_close(), LifecycleAction::BeginCleanup);
        assert_eq!(
            lifecycle.ensure_input_command(),
            Err(LifecycleCommandError::Shutdown)
        );
        assert_eq!(
            lifecycle.request_open(),
            Err(LifecycleCommandError::Shutdown)
        );

        assert_eq!(lifecycle.complete_close(), LifecycleAction::Closed);
        assert_eq!(
            lifecycle.ensure_input_command(),
            Err(LifecycleCommandError::Shutdown)
        );

        assert_eq!(lifecycle.request_open(), Ok(LifecycleAction::OpenTransport));
        assert_eq!(
            lifecycle.ensure_input_command(),
            Err(LifecycleCommandError::Shutdown)
        );
        assert_eq!(lifecycle.complete_open(), LifecycleAction::Opened);
        lifecycle
            .ensure_input_command()
            .expect("input after reopen");
    }

    #[test]
    fn every_live_state_begins_cleanup_once_before_closing() {
        for state in [
            LifecycleState::Open,
            LifecycleState::Connecting,
            LifecycleState::Ready,
            LifecycleState::Failed,
        ] {
            let mut lifecycle = LifecycleStateMachine {
                state,
                opening: false,
            };

            if matches!(
                state,
                LifecycleState::Open | LifecycleState::Connecting | LifecycleState::Ready
            ) {
                assert_eq!(
                    lifecycle.request_open(),
                    Ok(LifecycleAction::None),
                    "repeated open from {state:?}"
                );
                assert_eq!(lifecycle.state(), state);
            } else {
                assert_eq!(lifecycle.request_open(), Err(LifecycleCommandError::Failed));
                assert_eq!(lifecycle.state(), LifecycleState::Failed);
            }

            assert_eq!(
                lifecycle.request_close(),
                LifecycleAction::BeginCleanup,
                "first close from {state:?}"
            );
            assert_eq!(lifecycle.state(), LifecycleState::Closing);
            assert_eq!(
                lifecycle.ensure_input_command(),
                Err(LifecycleCommandError::Shutdown)
            );
            assert_eq!(lifecycle.request_close(), LifecycleAction::None);
            assert_eq!(lifecycle.complete_close(), LifecycleAction::Closed);
            assert_eq!(lifecycle.state(), LifecycleState::Closed);
        }

        let mut configured = LifecycleStateMachine::new();
        assert_eq!(configured.request_close(), LifecycleAction::Closed);
        assert_eq!(configured.state(), LifecycleState::Closed);
    }
}
