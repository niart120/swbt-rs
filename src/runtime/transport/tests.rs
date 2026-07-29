use std::error::Error as _;
use std::fmt;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use super::fake::{FakeTransport, ScriptedSendOutcome};
use super::{
    HidChannel, TransportError, TransportErrorKind, TransportEvent, TransportPort, activity_channel,
};

fn assert_error_kind(error: &TransportError, expected: TransportErrorKind) {
    assert_eq!(error.kind(), expected);
}

#[test]
fn open_close_are_idempotent_and_send_requires_open() {
    let (mut transport, control) = FakeTransport::with_limits(4, 4);
    fn assert_send<T: Send>() {}
    assert_send::<FakeTransport>();

    let error = transport
        .send_interrupt(b"before-open")
        .expect_err("send before open must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);
    let error = transport
        .poll(Duration::ZERO)
        .expect_err("poll before open must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);
    let error = transport
        .drain_interrupt(Duration::from_secs(1))
        .expect_err("drain before open must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);
    let error = control
        .inject_connected()
        .expect_err("event injection before open must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);

    let (notifier, first_wake_receiver) = activity_channel();
    transport.open(notifier).expect("first open");
    let (replacement, replacement_wake_receiver) = activity_channel();
    transport.open(replacement).expect("repeated open");
    assert_eq!(control.counters().open, 1);
    control
        .inject_connected()
        .expect("event injection while open");
    first_wake_receiver
        .try_recv()
        .expect("first notifier remains registered");
    assert_eq!(
        replacement_wake_receiver.try_recv(),
        Err(TryRecvError::Disconnected)
    );
    assert_eq!(
        transport.poll(Duration::ZERO).expect("clear event"),
        [TransportEvent::Connected]
    );

    transport
        .send_interrupt(b"accepted")
        .expect("open transport accepts the default send outcome");
    transport
        .drain_interrupt(Duration::from_secs(1))
        .expect("open transport drains pending interrupt data");
    assert_eq!(control.accepted_interrupts(), [Box::from(*b"accepted")]);

    transport.close().expect("first close");
    transport.close().expect("repeated close");
    assert_eq!(control.counters().close, 1);

    let error = transport
        .send_interrupt(b"after-close")
        .expect_err("send after close must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);
    let error = transport
        .poll(Duration::ZERO)
        .expect_err("poll after close must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);
    let error = transport
        .drain_interrupt(Duration::from_secs(1))
        .expect_err("drain after close must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);
    let error = control
        .inject_connected()
        .expect_err("event injection after close must fail");
    assert_error_kind(&error, TransportErrorKind::Closed);

    let (notifier, _wake_receiver) = activity_channel();
    transport.open(notifier).expect("reopen");
    transport.disconnect().expect("disconnect while open");
    transport.close().expect("close after reopen");
    assert_eq!(control.counters().open, 2);
    assert_eq!(control.counters().disconnect, 1);
    assert_eq!(control.counters().close, 2);
}

#[test]
fn send_outcomes_distinguish_acceptance_rejection_and_later_disconnect() {
    let (mut transport, control) = FakeTransport::with_limits(4, 4);
    let (notifier, _wake_receiver) = activity_channel();
    transport.open(notifier).expect("open");
    control.script_sends([
        ScriptedSendOutcome::Accepted,
        ScriptedSendOutcome::Rejected,
        ScriptedSendOutcome::AcceptedThenDisconnect { reason: Some(0x13) },
    ]);

    transport.send_interrupt(b"first").expect("accepted");
    let error = transport
        .send_interrupt(b"second")
        .expect_err("scripted rejection");
    assert_error_kind(&error, TransportErrorKind::SendRejected);
    transport
        .send_interrupt(b"third")
        .expect("accepted before later disconnect");

    assert_eq!(
        control.accepted_interrupts(),
        [Box::from(*b"first"), Box::from(*b"third")]
    );
    assert_eq!(
        transport.poll(Duration::ZERO).expect("poll disconnect"),
        [TransportEvent::Disconnected { reason: Some(0x13) }]
    );
    transport
        .close()
        .expect("send rejection does not prevent cleanup");
    assert_eq!(control.counters().close, 1);
}

#[test]
fn zero_poll_routes_channels_in_fifo_order_and_respects_batch_limit() {
    let (mut transport, control) = FakeTransport::with_limits(4, 2);
    let (notifier, _wake_receiver) = activity_channel();
    transport.open(notifier).expect("open");

    assert!(
        transport
            .poll(Duration::ZERO)
            .expect("empty zero poll")
            .is_empty()
    );
    control
        .inject_hid_output(HidChannel::Control, b"control")
        .expect("control event");
    control
        .inject_hid_output(HidChannel::Interrupt, b"interrupt")
        .expect("interrupt event");
    control.inject_connected().expect("connected event");

    assert_eq!(
        transport.poll(Duration::ZERO).expect("first batch"),
        [
            TransportEvent::HidOutput {
                channel: HidChannel::Control,
                payload: Box::from(*b"control"),
            },
            TransportEvent::HidOutput {
                channel: HidChannel::Interrupt,
                payload: Box::from(*b"interrupt"),
            },
        ]
    );
    assert_eq!(
        transport.poll(Duration::ZERO).expect("second batch"),
        [TransportEvent::Connected]
    );
}

#[test]
fn activity_wake_coalesces_without_losing_events() {
    let (mut transport, control) = FakeTransport::with_limits(4, 1);
    let (notifier, wake_receiver) = activity_channel();
    transport.open(notifier).expect("open");

    control.inject_connected().expect("connected event");
    control
        .inject_hid_output(HidChannel::Control, b"control")
        .expect("control event");
    control
        .inject_hid_output(HidChannel::Interrupt, b"interrupt")
        .expect("interrupt event");

    wake_receiver.try_recv().expect("one coalesced wake");
    assert_eq!(wake_receiver.try_recv(), Err(TryRecvError::Empty));
    assert_eq!(
        transport.poll(Duration::ZERO).expect("first event"),
        [TransportEvent::Connected]
    );
    assert_eq!(
        transport.poll(Duration::ZERO).expect("second event"),
        [TransportEvent::HidOutput {
            channel: HidChannel::Control,
            payload: Box::from(*b"control"),
        }]
    );
    assert_eq!(
        transport.poll(Duration::ZERO).expect("third event"),
        [TransportEvent::HidOutput {
            channel: HidChannel::Interrupt,
            payload: Box::from(*b"interrupt"),
        }]
    );
    assert!(
        transport
            .poll(Duration::ZERO)
            .expect("queue is drained")
            .is_empty()
    );

    control
        .inject_connected()
        .expect("new activity after drain");
    wake_receiver
        .try_recv()
        .expect("new activity rearms the wake");
}

#[test]
fn bounded_event_queue_overflow_is_terminal() {
    let (mut transport, control) = FakeTransport::with_limits(1, 1);
    let (notifier, wake_receiver) = activity_channel();
    transport.open(notifier).expect("open");

    control.inject_connected().expect("first event");
    control
        .inject_hid_output(HidChannel::Control, b"overflow")
        .expect("overflow is reported through poll");
    wake_receiver.try_recv().expect("overflow wakes worker");

    let error = transport
        .poll(Duration::ZERO)
        .expect_err("overflow must terminate polling");
    assert_error_kind(&error, TransportErrorKind::EventQueueOverflow);
    let repeated = transport
        .poll(Duration::ZERO)
        .expect_err("overflow remains terminal");
    assert_error_kind(&repeated, TransportErrorKind::EventQueueOverflow);
    let error = transport
        .send_interrupt(b"after-overflow")
        .expect_err("terminal transport rejects sends");
    assert_error_kind(&error, TransportErrorKind::EventQueueOverflow);
    let error = transport
        .disconnect()
        .expect_err("terminal transport rejects disconnect");
    assert_error_kind(&error, TransportErrorKind::EventQueueOverflow);
    let error = control
        .inject_connected()
        .expect_err("terminal transport rejects injected events");
    assert_error_kind(&error, TransportErrorKind::EventQueueOverflow);
    transport.close().expect("terminal transport can close");
    transport
        .close()
        .expect("terminal close remains idempotent");
    assert_eq!(control.counters().close, 1);
}

#[derive(Debug)]
struct SecretSentinel;

impl fmt::Display for SecretSentinel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PAIRING-KEY-SHOULD-NOT-BE-FORMATTED")
    }
}

impl std::error::Error for SecretSentinel {}

#[test]
fn terminal_source_follows_queued_events_without_leaking_its_text() {
    let (mut transport, control) = FakeTransport::with_limits(2, 2);
    let (notifier, wake_receiver) = activity_channel();
    transport.open(notifier).expect("open");

    control
        .inject_hid_output(HidChannel::Control, b"before-terminal")
        .expect("event before terminal source");
    control
        .terminate_with(SecretSentinel)
        .expect("terminal source injection while open");
    wake_receiver
        .try_recv()
        .expect("terminal source wakes worker");

    assert_eq!(
        transport
            .poll(Duration::ZERO)
            .expect("events before source termination remain observable"),
        [TransportEvent::HidOutput {
            channel: HidChannel::Control,
            payload: Box::from(*b"before-terminal"),
        }]
    );
    let error = transport
        .poll(Duration::ZERO)
        .expect_err("terminal source must fail polling");
    assert_error_kind(&error, TransportErrorKind::SourceTerminated);
    assert!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<SecretSentinel>())
            .is_some()
    );
    assert!(!format!("{error}").contains("PAIRING-KEY"));
    assert!(!format!("{error:?}").contains("PAIRING-KEY"));
    let error = transport
        .send_interrupt(b"after-terminal")
        .expect_err("source-terminated transport rejects sends");
    assert_error_kind(&error, TransportErrorKind::SourceTerminated);
    transport
        .close()
        .expect("source-terminated transport can close");
}
