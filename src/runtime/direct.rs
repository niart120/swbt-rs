use crate::{
    input::InputState,
    model::ControllerModel,
    protocol::SwitchHidProtocol,
    runtime::{
        sender::ReportSender,
        state::InputStateStore,
        transport::{SendAcceptance, TransportPort, TransportResult},
    },
};

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T12 defines Direct input transactions before T21 worker integration"
    )
)]
pub(crate) fn send_candidate<M: ControllerModel>(
    candidate: InputState<M>,
    now_ns: u64,
    store: &mut InputStateStore<M>,
    protocol: &SwitchHidProtocol<M>,
    sender: &mut ReportSender<M>,
    transport: &mut dyn TransportPort,
) -> TransportResult<SendAcceptance> {
    let acceptance = sender.send_input(protocol, &candidate, now_ns, transport)?;
    store.commit(candidate);
    Ok(acceptance)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{
        input::{InputState, ProButton},
        model::{ButtonKind, Pro},
        protocol::{DeviceInfoBluetoothAddress, SwitchHidProtocol},
        runtime::{
            direct::send_candidate,
            sender::ReportSender,
            state::InputStateStore,
            transport::{
                TransportErrorKind, TransportEvent, TransportPort, activity_channel,
                fake::{FakeTransport, FakeTransportControl, ScriptedSendOutcome},
            },
        },
    };

    const DEVICE_INFO_ADDRESS: [u8; 6] = [0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d];

    #[test]
    fn direct_send_commits_only_after_acceptance_and_retry() {
        let mut harness = Harness::new();
        harness.control.script_sends([
            ScriptedSendOutcome::Accepted,
            ScriptedSendOutcome::Rejected,
            ScriptedSendOutcome::Accepted,
        ]);
        let pressed_a = pressed_state(ButtonKind::A);
        let pressed_x = pressed_state(ButtonKind::X);

        harness
            .send(pressed_a.clone())
            .expect("first candidate accepted");
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);

        let error = harness
            .send(pressed_x.clone())
            .expect_err("second candidate rejected");
        assert_eq!(error.kind(), TransportErrorKind::SendRejected);
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);

        harness
            .send(pressed_x.clone())
            .expect("retry candidate accepted");
        assert_eq!(harness.store.snapshot(), pressed_x);
        assert_eq!(harness.sender.timer(), 2);
        assert_eq!(
            harness
                .control
                .accepted_interrupts()
                .iter()
                .map(|report| { (report[0], report[1], [report[3], report[4], report[5]],) })
                .collect::<Vec<_>>(),
            [(0x30, 0, [0x08, 0x00, 0x00]), (0x30, 1, [0x02, 0x00, 0x00]),]
        );
    }

    #[test]
    fn accepted_then_disconnect_remains_committed() {
        let mut harness = Harness::new();
        harness
            .control
            .script_sends([ScriptedSendOutcome::AcceptedThenDisconnect { reason: Some(0x13) }]);
        let pressed_a = pressed_state(ButtonKind::A);

        harness
            .send(pressed_a.clone())
            .expect("input accepted before disconnect event");
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);

        assert_eq!(
            harness
                .transport
                .poll(Duration::ZERO)
                .expect("queued disconnect follows accepted send"),
            [TransportEvent::Disconnected { reason: Some(0x13) }]
        );
        assert_eq!(harness.store.snapshot(), pressed_a);
        assert_eq!(harness.sender.timer(), 1);
    }

    struct Harness {
        protocol: SwitchHidProtocol<Pro>,
        sender: ReportSender<Pro>,
        store: InputStateStore<Pro>,
        transport: FakeTransport,
        control: FakeTransportControl,
    }

    impl Harness {
        fn new() -> Self {
            let (mut transport, control) = FakeTransport::with_limits(8, 8);
            let (notifier, _wake_receiver) = activity_channel();
            transport.open(notifier).expect("open fake transport");
            Self {
                protocol: SwitchHidProtocol::new(
                    None,
                    DeviceInfoBluetoothAddress::from_wire_bytes(DEVICE_INFO_ADDRESS),
                ),
                sender: ReportSender::new(),
                store: InputStateStore::new(),
                transport,
                control,
            }
        }

        fn send(
            &mut self,
            candidate: InputState<Pro>,
        ) -> crate::runtime::transport::TransportResult<crate::runtime::transport::SendAcceptance>
        {
            send_candidate(
                candidate,
                0,
                &mut self.store,
                &self.protocol,
                &mut self.sender,
                &mut self.transport,
            )
        }
    }

    fn pressed_state(kind: ButtonKind) -> InputState<Pro> {
        let button = ProButton::try_from(kind).expect("button supported by Pro Controller");
        InputState::neutral().with_buttons([button])
    }
}
