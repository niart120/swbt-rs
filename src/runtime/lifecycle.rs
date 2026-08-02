use crate::diagnostics::LifecycleState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleAction {
    BeginCleanup,
    Closed,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LifecycleCommandError {
    Shutdown,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeState {
    Open,
    Connecting,
    Ready,
    Closing,
    Failed,
}

pub(crate) struct LifecycleStateMachine {
    state: RuntimeState,
}

impl LifecycleStateMachine {
    pub(crate) const fn new() -> Self {
        Self {
            state: RuntimeState::Open,
        }
    }

    #[must_use]
    pub(crate) const fn state(&self) -> LifecycleState {
        match self.state {
            RuntimeState::Open => LifecycleState::Open,
            RuntimeState::Connecting => LifecycleState::Connecting,
            RuntimeState::Ready => LifecycleState::Ready,
            RuntimeState::Closing => LifecycleState::Closing,
            RuntimeState::Failed => LifecycleState::Failed,
        }
    }

    pub(crate) fn mark_failed(&mut self) {
        self.state = RuntimeState::Failed;
    }

    pub(crate) fn begin_connection(&mut self) -> bool {
        if self.state != RuntimeState::Open {
            return false;
        }
        self.state = RuntimeState::Connecting;
        true
    }

    pub(crate) fn mark_ready(&mut self) -> bool {
        if self.state != RuntimeState::Connecting {
            return false;
        }
        self.state = RuntimeState::Ready;
        true
    }

    pub(crate) fn mark_connection_ended(&mut self) -> bool {
        if !matches!(self.state, RuntimeState::Connecting | RuntimeState::Ready) {
            return false;
        }
        self.state = RuntimeState::Open;
        true
    }

    pub(crate) fn request_close(&mut self) -> LifecycleAction {
        match self.state {
            RuntimeState::Open
            | RuntimeState::Connecting
            | RuntimeState::Ready
            | RuntimeState::Failed => {
                self.state = RuntimeState::Closing;
                LifecycleAction::BeginCleanup
            }
            RuntimeState::Closing => LifecycleAction::None,
        }
    }

    pub(crate) const fn complete_close(&self) -> LifecycleAction {
        match self.state {
            RuntimeState::Closing => LifecycleAction::Closed,
            RuntimeState::Open
            | RuntimeState::Connecting
            | RuntimeState::Ready
            | RuntimeState::Failed => LifecycleAction::None,
        }
    }

    pub(crate) fn ensure_input_command(&self) -> Result<(), LifecycleCommandError> {
        match self.state {
            RuntimeState::Closing => Err(LifecycleCommandError::Shutdown),
            RuntimeState::Failed => Err(LifecycleCommandError::Failed),
            RuntimeState::Open | RuntimeState::Connecting | RuntimeState::Ready => Ok(()),
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
    use crate::diagnostics::LifecycleState;

    use super::{LifecycleAction, LifecycleCommandError, LifecycleStateMachine, RuntimeState};

    #[test]
    fn live_worker_lifecycle_starts_open_without_replaying_transport_open() {
        let lifecycle = LifecycleStateMachine::new();

        assert_eq!(lifecycle.state(), LifecycleState::Open);
        lifecycle
            .ensure_input_command()
            .expect("an open worker accepts input commands");
    }

    #[test]
    fn connection_transitions_return_a_live_worker_to_open() {
        let mut lifecycle = LifecycleStateMachine::new();

        assert!(lifecycle.begin_connection());
        assert_eq!(lifecycle.state(), LifecycleState::Connecting);
        assert!(!lifecycle.begin_connection());
        assert!(lifecycle.mark_ready());
        assert_eq!(lifecycle.state(), LifecycleState::Ready);
        assert!(lifecycle.mark_connection_ended());
        assert_eq!(lifecycle.state(), LifecycleState::Open);
        assert!(!lifecycle.mark_connection_ended());
    }

    #[test]
    fn commands_after_close_begins_remain_shutdown_for_the_terminal_worker() {
        let mut lifecycle = LifecycleStateMachine::new();
        lifecycle
            .ensure_input_command()
            .expect("input before close begins");

        assert_eq!(lifecycle.request_close(), LifecycleAction::BeginCleanup);
        assert_eq!(
            lifecycle.ensure_input_command(),
            Err(LifecycleCommandError::Shutdown)
        );
        assert_eq!(lifecycle.request_close(), LifecycleAction::None);

        assert_eq!(lifecycle.complete_close(), LifecycleAction::Closed);
        assert_eq!(
            lifecycle.ensure_input_command(),
            Err(LifecycleCommandError::Shutdown)
        );
        assert_eq!(LifecycleStateMachine::new().state(), LifecycleState::Open);
    }

    #[test]
    fn every_live_state_begins_cleanup_once_before_closing() {
        for state in [
            RuntimeState::Open,
            RuntimeState::Connecting,
            RuntimeState::Ready,
            RuntimeState::Failed,
        ] {
            let mut lifecycle = LifecycleStateMachine { state };

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
        }
    }

    #[test]
    fn failed_worker_rejects_commands_before_cleanup() {
        let mut lifecycle = LifecycleStateMachine::new();
        lifecycle.mark_failed();

        assert_eq!(lifecycle.state(), LifecycleState::Failed);
        assert_eq!(
            lifecycle.ensure_input_command(),
            Err(LifecycleCommandError::Failed)
        );
        assert_eq!(lifecycle.request_close(), LifecycleAction::BeginCleanup);
        assert_eq!(lifecycle.state(), LifecycleState::Closing);
    }
}
