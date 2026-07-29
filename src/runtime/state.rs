use crate::{input::InputState, model::ControllerModel, runtime::status::StatusPublisher};

pub(crate) struct InputStateStore<M: ControllerModel> {
    committed: InputState<M>,
    status: Option<StatusPublisher<M>>,
}

impl<M: ControllerModel> InputStateStore<M> {
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self {
            committed: InputState::neutral(),
            status: None,
        }
    }

    pub(crate) fn with_status(status: StatusPublisher<M>) -> Self {
        Self {
            committed: InputState::neutral(),
            status: Some(status),
        }
    }

    #[must_use]
    pub(crate) fn snapshot(&self) -> InputState<M> {
        self.committed.clone()
    }

    pub(crate) fn commit(&mut self, next: InputState<M>) {
        self.committed = next;
        if let Some(status) = self.status.as_ref() {
            status.set_snapshot(&self.committed);
        }
    }

    pub(crate) fn reset_to_neutral(&mut self) {
        self.committed = InputState::neutral();
        if let Some(status) = self.status.as_ref() {
            status.set_snapshot(&self.committed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::SwitchHidProtocol,
        runtime::{
            sender::ReportSender,
            state::InputStateStore,
            transport::{
                TransportErrorKind, TransportPort, activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];

    #[test]
    fn commit_replaces_current_without_transport() {
        let mut store = InputStateStore::<Pro>::new();
        let pressed = pressed_state(ButtonKind::A);

        store.commit(pressed.clone());

        assert_eq!(store.snapshot(), pressed);
    }

    #[test]
    fn rejected_send_does_not_roll_back_committed_state() {
        let mut store = InputStateStore::<Pro>::new();
        let pressed = pressed_state(ButtonKind::A);
        store.commit(pressed.clone());
        let snapshot = store.snapshot();
        let protocol = protocol();
        let (mut transport, control) = open_transport();
        control.script_sends([ScriptedSendOutcome::Rejected]);
        let mut sender = ReportSender::<Pro>::new();

        let error = sender
            .send_input(&protocol, &snapshot, 0, &mut transport)
            .expect_err("scripted send must be rejected");

        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(store.snapshot(), pressed);
        assert_eq!(sender.timer(), 0);
    }

    #[test]
    fn new_session_reset_restores_neutral() {
        let mut store = InputStateStore::<Pro>::new();
        store.commit(pressed_state(ButtonKind::A));

        store.reset_to_neutral();

        assert_eq!(store.snapshot(), InputState::<Pro>::neutral());

        store.commit(pressed_state(ButtonKind::B));
        store.reset_to_neutral();

        assert_eq!(store.snapshot(), InputState::<Pro>::neutral());
    }

    fn pressed_state(kind: ButtonKind) -> InputState<Pro> {
        let button = ProButton::try_from(kind).expect("button supported by Pro Controller");
        InputState::neutral().with_buttons([button])
    }

    fn protocol() -> SwitchHidProtocol<Pro> {
        SwitchHidProtocol::new(None, DEVICE_INFO_ADDRESS)
    }

    fn open_transport() -> (FakeTransport, FakeTransportControl) {
        let (mut transport, control) = FakeTransport::with_limits(8, 8);
        let (notifier, _wake_receiver) = activity_channel();
        transport.open(notifier).expect("open fake transport");
        assert_eq!(
            transport
                .poll(Duration::ZERO)
                .expect("opened fake has no queued events"),
            []
        );
        (transport, control)
    }
}
