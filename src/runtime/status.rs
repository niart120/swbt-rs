use std::{
    marker::PhantomData,
    num::NonZeroU64,
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{
    diagnostics::{
        GamepadStatus, LifecycleState,
        event::{DiagnosticContext, DiagnosticEvent, WorkerFailureCategory},
    },
    input::InputState,
    model::ControllerModel,
    reporting::ReportingMode,
};

type DiagnosticEmitter = Arc<dyn Fn(DiagnosticEvent) + Send + Sync>;

struct ProjectionState<M: ControllerModel> {
    lifecycle: LifecycleState,
    connected: bool,
    session_id: Option<NonZeroU64>,
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
            session_id: None,
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

/// Write handle for the controller status projection.
///
/// A configured controller retains this handle so the same projection can be
/// shared with its worker. Each method holds the write lock only while
/// replacing projection values. Callers must invoke it outside transport,
/// response, and join waits.
pub(crate) struct StatusPublisher<M: ControllerModel> {
    shared: Arc<RwLock<ProjectionState<M>>>,
    emitter: DiagnosticEmitter,
    context: fn(NonZeroU64) -> DiagnosticContext,
}

impl<M: ControllerModel> Clone for StatusPublisher<M> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            emitter: Arc::clone(&self.emitter),
            context: self.context,
        }
    }
}

/// Controller-owned read handle for status and typed input snapshots.
pub(crate) struct StatusReader<M: ControllerModel, R: ReportingMode> {
    shared: Arc<RwLock<ProjectionState<M>>>,
    types: PhantomData<fn() -> R>,
}

pub(crate) fn status_projection<M: ControllerModel, R: ReportingMode>()
-> (StatusPublisher<M>, StatusReader<M, R>) {
    status_projection_with_emitter::<M, R>(Arc::new(DiagnosticEvent::emit))
}

pub(crate) fn status_projection_with_emitter<M: ControllerModel, R: ReportingMode>(
    emitter: DiagnosticEmitter,
) -> (StatusPublisher<M>, StatusReader<M, R>) {
    let shared = Arc::new(RwLock::new(ProjectionState::configured()));
    (
        StatusPublisher {
            shared: Arc::clone(&shared),
            emitter,
            context: diagnostic_context::<M, R>,
        },
        StatusReader {
            shared,
            types: PhantomData,
        },
    )
}

impl<M: ControllerModel> StatusPublisher<M> {
    pub(crate) fn set_lifecycle(&self, lifecycle: LifecycleState) {
        let event = {
            let mut state = write(&self.shared);
            if state.lifecycle == lifecycle {
                return;
            }
            state.lifecycle = lifecycle;
            self.context(&state)
                .map(|context| DiagnosticEvent::lifecycle_changed(context, lifecycle))
        };
        self.emit(event);
    }

    pub(crate) fn begin_session(
        &self,
        session_id: NonZeroU64,
        lifecycle: LifecycleState,
        snapshot: &InputState<M>,
    ) {
        let context = {
            let mut state = write(&self.shared);
            state.lifecycle = lifecycle;
            state.connected = false;
            state.session_id = Some(session_id);
            state.report_mode = None;
            state.last_subcommand = None;
            state.last_disconnect_reason = None;
            state.worker_failure = None;
            state.snapshot = snapshot.clone();
            self.context(&state).expect("session ID was just installed")
        };
        (self.emitter)(DiagnosticEvent::session_started(context));
        (self.emitter)(DiagnosticEvent::lifecycle_changed(context, lifecycle));
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
        let events = {
            let mut state = write(&self.shared);
            let previous_input = state.input_reports_accepted;
            let previous_replies = state.replies_accepted;
            state.report_mode = report_mode;
            state.input_reports_accepted = input_reports_accepted;
            state.replies_accepted = replies_accepted;
            let context = self.context(&state);
            [
                (input_reports_accepted > previous_input).then(|| {
                    DiagnosticEvent::report_tx_accepted(
                        context.expect("active session context"),
                        report_mode,
                        input_reports_accepted,
                    )
                }),
                (replies_accepted > previous_replies).then(|| {
                    DiagnosticEvent::reply_tx_accepted(
                        context.expect("active session context"),
                        report_mode,
                        replies_accepted,
                    )
                }),
            ]
        };
        for event in events.into_iter().flatten() {
            (self.emitter)(event);
        }
    }

    pub(crate) fn record_subcommand(&self, id: u8) {
        let event = {
            let mut state = write(&self.shared);
            state.last_subcommand = Some(id);
            self.context(&state)
                .map(|context| DiagnosticEvent::subcommand_observed(context, id))
        };
        self.emit(event);
    }

    pub(crate) fn record_unsupported_button(&self, button_kind: crate::ButtonKind) {
        let event = {
            let state = read(&self.shared);
            self.context(&state)
                .map(|context| DiagnosticEvent::unsupported_button(context, button_kind))
        };
        self.emit(event);
    }

    pub(crate) fn end_session(&self, lifecycle: LifecycleState, disconnect_reason: Option<u8>) {
        let event = {
            let mut state = write(&self.shared);
            let context = self.context(&state);
            state.lifecycle = lifecycle;
            state.connected = false;
            state.session_id = None;
            state.report_mode = None;
            state.last_disconnect_reason = disconnect_reason;
            context.map(|context| {
                DiagnosticEvent::session_ended(context, lifecycle, disconnect_reason)
            })
        };
        self.emit(event);
    }

    pub(crate) fn close(&self, lifecycle: LifecycleState) {
        let event = {
            let mut state = write(&self.shared);
            let context = self.context(&state);
            state.lifecycle = lifecycle;
            state.connected = false;
            state.session_id = None;
            state.report_mode = None;
            context.map(|context| DiagnosticEvent::session_ended(context, lifecycle, None))
        };
        self.emit(event);
    }

    pub(crate) fn fail(&self, message: &'static str, category: WorkerFailureCategory) {
        let events = {
            let mut state = write(&self.shared);
            if state.worker_failure.is_some() {
                return;
            }
            let context = self.context(&state);
            state.lifecycle = LifecycleState::Failed;
            state.connected = false;
            state.session_id = None;
            state.report_mode = None;
            state.worker_failure = Some(message.to_owned());
            context.map(|context| {
                [
                    DiagnosticEvent::worker_failed(context, category),
                    DiagnosticEvent::lifecycle_changed(context, LifecycleState::Failed),
                    DiagnosticEvent::session_ended(context, LifecycleState::Failed, None),
                ]
            })
        };
        for event in events.into_iter().flatten() {
            (self.emitter)(event);
        }
    }

    fn emit(&self, event: Option<DiagnosticEvent>) {
        if let Some(event) = event {
            (self.emitter)(event);
        }
    }

    fn context(&self, state: &ProjectionState<M>) -> Option<DiagnosticContext> {
        state.session_id.map(self.context)
    }
}

impl<M: ControllerModel, R: ReportingMode> StatusReader<M, R> {
    pub(crate) fn status(&self) -> GamepadStatus {
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

fn diagnostic_context<M: ControllerModel, R: ReportingMode>(
    session_id: NonZeroU64,
) -> DiagnosticContext {
    DiagnosticContext::new(M::KIND, R::KIND, session_id)
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

#[cfg(test)]
mod tests {
    use std::{
        num::NonZeroU64,
        sync::{Arc, Mutex},
    };

    use crate::{ButtonKind, LifecycleState, model, reporting};

    use super::status_projection_with_emitter;

    #[test]
    fn unsupported_button_event_uses_the_active_session_context() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let (publisher, _) = status_projection_with_emitter::<model::JoyConL, reporting::Direct>(
            Arc::new(move |event| {
                captured.lock().unwrap().push(event);
            }),
        );

        publisher.record_unsupported_button(ButtonKind::A);
        assert!(events.lock().unwrap().is_empty());

        publisher.begin_session(
            NonZeroU64::new(7).unwrap(),
            LifecycleState::Ready,
            &crate::JoyConLInputState::neutral(),
        );
        events.lock().unwrap().clear();
        publisher.record_unsupported_button(ButtonKind::A);

        let events = events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].to_value(),
            serde_json::json!({
                "schema": "swbt.diagnostics",
                "schema_version": 1,
                "event": "unsupported_button",
                "controller_kind": "joycon_l",
                "reporting_kind": "direct",
                "session_id": 7,
                "button_kind": "a",
            })
        );
    }
}
