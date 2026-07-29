use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::{
    diagnostics::{GamepadStatus, LifecycleState},
    input::InputState,
    model::ControllerModel,
    reporting::ReportingMode,
};

struct ProjectionState<M: ControllerModel> {
    lifecycle: LifecycleState,
    connected: bool,
    report_mode: Option<u8>,
    input_reports_accepted: u64,
    replies_accepted: u64,
    last_subcommand: Option<u8>,
    last_disconnect_reason: Option<u8>,
    worker_failure: Option<String>,
    snapshot: InputState<M>,
}

impl<M: ControllerModel> ProjectionState<M> {
    fn configured() -> Self {
        Self {
            lifecycle: LifecycleState::Configured,
            connected: false,
            report_mode: None,
            input_reports_accepted: 0,
            replies_accepted: 0,
            last_subcommand: None,
            last_disconnect_reason: None,
            worker_failure: None,
            snapshot: InputState::neutral(),
        }
    }
}

/// Worker-owned write handle for the controller status projection.
///
/// Each method holds the write lock only while replacing projection values.
/// Callers must invoke it outside transport, response, and join waits.
pub(crate) struct StatusPublisher<M: ControllerModel> {
    shared: Arc<RwLock<ProjectionState<M>>>,
}

impl<M: ControllerModel> Clone for StatusPublisher<M> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
        }
    }
}

/// Controller-owned read handle for status and typed input snapshots.
pub(crate) struct StatusReader<M: ControllerModel> {
    shared: Arc<RwLock<ProjectionState<M>>>,
}

pub(crate) fn status_projection<M: ControllerModel>() -> (StatusPublisher<M>, StatusReader<M>) {
    let shared = Arc::new(RwLock::new(ProjectionState::configured()));
    (
        StatusPublisher {
            shared: Arc::clone(&shared),
        },
        StatusReader { shared },
    )
}

impl<M: ControllerModel> StatusPublisher<M> {
    pub(crate) fn set_lifecycle(&self, lifecycle: LifecycleState) {
        write(&self.shared).lifecycle = lifecycle;
    }

    pub(crate) fn begin_session(&self, lifecycle: LifecycleState, snapshot: &InputState<M>) {
        let mut state = write(&self.shared);
        state.lifecycle = lifecycle;
        state.connected = false;
        state.report_mode = None;
        state.last_subcommand = None;
        state.last_disconnect_reason = None;
        state.worker_failure = None;
        state.snapshot = snapshot.clone();
    }

    pub(crate) fn set_connected(&self, connected: bool) {
        write(&self.shared).connected = connected;
    }

    pub(crate) fn set_snapshot(&self, snapshot: &InputState<M>) {
        write(&self.shared).snapshot = snapshot.clone();
    }

    pub(crate) fn set_sender_state(
        &self,
        report_mode: Option<u8>,
        input_reports_accepted: u64,
        replies_accepted: u64,
    ) {
        let mut state = write(&self.shared);
        state.report_mode = report_mode;
        state.input_reports_accepted = input_reports_accepted;
        state.replies_accepted = replies_accepted;
    }

    pub(crate) fn record_subcommand(&self, id: u8) {
        write(&self.shared).last_subcommand = Some(id);
    }

    pub(crate) fn end_session(&self, lifecycle: LifecycleState, disconnect_reason: Option<u8>) {
        let mut state = write(&self.shared);
        state.lifecycle = lifecycle;
        state.connected = false;
        state.report_mode = None;
        state.last_disconnect_reason = disconnect_reason;
    }

    pub(crate) fn close(&self, lifecycle: LifecycleState) {
        let mut state = write(&self.shared);
        state.lifecycle = lifecycle;
        state.connected = false;
        state.report_mode = None;
    }

    pub(crate) fn fail(&self, message: &'static str) {
        let mut state = write(&self.shared);
        state.lifecycle = LifecycleState::Failed;
        state.connected = false;
        state.report_mode = None;
        state.worker_failure = Some(message.to_owned());
    }
}

impl<M: ControllerModel> StatusReader<M> {
    pub(crate) fn status<R: ReportingMode>(&self) -> GamepadStatus {
        let state = read(&self.shared);
        GamepadStatus {
            lifecycle: state.lifecycle,
            connected: state.connected,
            controller_kind: M::KIND,
            reporting_kind: R::KIND,
            report_mode: state.report_mode,
            input_reports_accepted: state.input_reports_accepted,
            replies_accepted: state.replies_accepted,
            last_subcommand: state.last_subcommand,
            last_disconnect_reason: state.last_disconnect_reason,
            worker_failure: state.worker_failure.clone(),
        }
    }

    pub(crate) fn snapshot(&self) -> InputState<M> {
        read(&self.shared).snapshot.clone()
    }
}

fn read<M: ControllerModel>(
    lock: &RwLock<ProjectionState<M>>,
) -> RwLockReadGuard<'_, ProjectionState<M>> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write<M: ControllerModel>(
    lock: &RwLock<ProjectionState<M>>,
) -> RwLockWriteGuard<'_, ProjectionState<M>> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
