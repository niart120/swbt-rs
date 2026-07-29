use std::collections::VecDeque;
use std::error::Error as StdError;
use std::sync::mpsc::{
    Receiver, RecvTimeoutError, SyncSender, TryRecvError, TrySendError, sync_channel,
};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use super::{
    ActivityNotifier, HidChannel, SendAcceptance, TransportError, TransportErrorKind,
    TransportEvent, TransportPort, TransportResult,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::runtime) enum ScriptedSendOutcome {
    Accepted,
    Rejected,
    Closed,
    AcceptedThenDisconnect { reason: Option<u8> },
    AcceptedThenEvent(TransportEvent),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FakeTransportCounters {
    pub(crate) open: usize,
    pub(crate) disconnect: usize,
    pub(crate) close: usize,
}

#[derive(Clone)]
pub(in crate::runtime) struct FakeTransportControl {
    shared: Arc<Shared>,
}

pub(in crate::runtime) struct FakeTransport {
    shared: Arc<Shared>,
    events: Receiver<QueuedEvent>,
    max_poll_batch: usize,
    is_open: bool,
    source_terminal_observed: bool,
}

struct Shared {
    events: SyncSender<QueuedEvent>,
    lifecycle: Mutex<FakeLifecycle>,
    send_script: Mutex<VecDeque<ScriptedSendOutcome>>,
    accepted_interrupts: Mutex<Vec<Box<[u8]>>>,
    counters: Mutex<FakeTransportCounters>,
    terminal: Mutex<Option<Terminal>>,
}

struct FakeLifecycle {
    is_open: bool,
    activity: Option<ActivityNotifier>,
}

enum QueuedEvent {
    Event(TransportEvent),
    Terminal,
}

#[derive(Clone)]
enum Terminal {
    Closed,
    EventQueueOverflow,
    Source(Arc<dyn StdError + Send + Sync>),
}

impl FakeTransport {
    pub(in crate::runtime) fn with_limits(
        event_capacity: usize,
        max_poll_batch: usize,
    ) -> (Self, FakeTransportControl) {
        assert!(event_capacity > 0, "event capacity must be positive");
        assert!(max_poll_batch > 0, "poll batch limit must be positive");
        let (event_sender, events) = sync_channel(event_capacity);
        let shared = Arc::new(Shared {
            events: event_sender,
            lifecycle: Mutex::new(FakeLifecycle {
                is_open: false,
                activity: None,
            }),
            send_script: Mutex::new(VecDeque::new()),
            accepted_interrupts: Mutex::new(Vec::new()),
            counters: Mutex::new(FakeTransportCounters::default()),
            terminal: Mutex::new(None),
        });
        (
            Self {
                shared: Arc::clone(&shared),
                events,
                max_poll_batch,
                is_open: false,
                source_terminal_observed: false,
            },
            FakeTransportControl { shared },
        )
    }

    fn terminal(&self) -> Option<Terminal> {
        lock(&self.shared.terminal).clone()
    }

    fn terminal_error(&self) -> TransportError {
        self.terminal()
            .as_ref()
            .map(terminal_error)
            .unwrap_or_else(|| TransportError::new(TransportErrorKind::SourceTerminated))
    }

    fn receive_first(&self, timeout: Duration) -> TransportResult<Option<QueuedEvent>> {
        if timeout.is_zero() {
            return match self.events.try_recv() {
                Ok(event) => Ok(Some(event)),
                Err(TryRecvError::Empty) => Ok(None),
                Err(TryRecvError::Disconnected) => {
                    Err(TransportError::new(TransportErrorKind::SourceTerminated))
                }
            };
        }

        match self.events.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(TransportError::new(TransportErrorKind::SourceTerminated))
            }
        }
    }

    fn push_event(result: &mut Vec<TransportEvent>, queued: QueuedEvent) {
        if let QueuedEvent::Event(event) = queued {
            result.push(event);
        }
    }
}

impl TransportPort for FakeTransport {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()> {
        if self.is_open {
            return self.shared.ensure_active();
        }
        {
            let mut lifecycle = lock(&self.shared.lifecycle);
            let terminal = lock(&self.shared.terminal);
            if let Some(terminal) = terminal.as_ref() {
                return Err(terminal_error(terminal));
            }
            lifecycle.activity = Some(activity);
            lifecycle.is_open = true;
        }
        self.is_open = true;
        self.source_terminal_observed = false;
        lock(&self.shared.counters).open += 1;
        Ok(())
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>> {
        if !self.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }

        let terminal = self.terminal();
        if matches!(terminal, Some(Terminal::EventQueueOverflow)) {
            return Err(self.terminal_error());
        }
        if self.source_terminal_observed {
            return Err(self.terminal_error());
        }

        let receive_timeout = if terminal.is_some() {
            Duration::ZERO
        } else {
            timeout
        };
        let Some(first) = self.receive_first(receive_timeout)? else {
            if self.terminal().is_some() {
                self.source_terminal_observed = true;
                return Err(self.terminal_error());
            }
            return Ok(Vec::new());
        };
        if matches!(first, QueuedEvent::Terminal) {
            self.source_terminal_observed = true;
            return Err(self.terminal_error());
        }

        let mut result = Vec::with_capacity(self.max_poll_batch);
        Self::push_event(&mut result, first);
        while result.len() < self.max_poll_batch {
            match self.events.try_recv() {
                Ok(QueuedEvent::Event(event)) => result.push(event),
                Ok(QueuedEvent::Terminal) => {
                    self.source_terminal_observed = true;
                    return Ok(result);
                }
                Err(TryRecvError::Empty) => {
                    match self.terminal() {
                        Some(Terminal::EventQueueOverflow) => {
                            return Err(self.terminal_error());
                        }
                        Some(Terminal::Closed | Terminal::Source(_)) => {
                            self.source_terminal_observed = true;
                        }
                        None => {}
                    }
                    break;
                }
                Err(TryRecvError::Disconnected) => {
                    return Err(TransportError::new(TransportErrorKind::SourceTerminated));
                }
            }
        }
        if matches!(self.terminal(), Some(Terminal::EventQueueOverflow)) {
            return Err(self.terminal_error());
        }
        Ok(result)
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        if !self.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        self.shared.send_interrupt_if_active(payload)
    }

    fn drain_interrupt(&mut self, _timeout: Duration) -> TransportResult<()> {
        if !self.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        self.shared.ensure_active()
    }

    fn disconnect(&mut self) -> TransportResult<()> {
        if !self.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        self.shared.disconnect_if_active()
    }

    fn close(&mut self) -> TransportResult<()> {
        if !self.is_open {
            return Ok(());
        }
        {
            let mut lifecycle = lock(&self.shared.lifecycle);
            lifecycle.is_open = false;
            lifecycle.activity = None;
        }
        self.is_open = false;
        lock(&self.shared.counters).close += 1;
        Ok(())
    }
}

impl FakeTransportControl {
    pub(in crate::runtime) fn script_sends(
        &self,
        outcomes: impl IntoIterator<Item = ScriptedSendOutcome>,
    ) {
        lock(&self.shared.send_script).extend(outcomes);
    }

    pub(in crate::runtime) fn accepted_interrupts(&self) -> Vec<Box<[u8]>> {
        lock(&self.shared.accepted_interrupts).clone()
    }

    pub(crate) fn counters(&self) -> FakeTransportCounters {
        *lock(&self.shared.counters)
    }

    pub(in crate::runtime) fn inject_connected(&self) -> TransportResult<()> {
        self.shared.enqueue_if_open(TransportEvent::Connected)
    }

    pub(in crate::runtime) fn inject_hid_channel_opened(
        &self,
        channel: HidChannel,
    ) -> TransportResult<()> {
        self.shared
            .enqueue_if_open(TransportEvent::HidChannelOpened { channel })
    }

    pub(in crate::runtime) fn inject_hid_output(
        &self,
        channel: HidChannel,
        payload: &[u8],
    ) -> TransportResult<()> {
        self.shared.enqueue_if_open(TransportEvent::HidOutput {
            channel,
            payload: Box::from(payload),
        })
    }

    pub(in crate::runtime) fn inject_disconnected(
        &self,
        reason: Option<u8>,
    ) -> TransportResult<()> {
        self.shared
            .enqueue_if_open(TransportEvent::Disconnected { reason })
    }

    pub(in crate::runtime) fn terminate_with(
        &self,
        source: impl StdError + Send + Sync + 'static,
    ) -> TransportResult<()> {
        let source: Arc<dyn StdError + Send + Sync> = Arc::new(source);
        self.shared.terminate_if_open(source)
    }
}

impl Shared {
    fn ensure_active(&self) -> TransportResult<()> {
        let lifecycle = lock(&self.lifecycle);
        if !lifecycle.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        if let Some(terminal) = lock(&self.terminal).as_ref() {
            return Err(terminal_error(terminal));
        }
        Ok(())
    }

    fn enqueue_if_open(&self, event: TransportEvent) -> TransportResult<()> {
        let lifecycle = lock(&self.lifecycle);
        if !lifecycle.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        let mut terminal = lock(&self.terminal);
        if let Some(terminal) = terminal.as_ref() {
            return Err(terminal_error(terminal));
        }

        match self.events.try_send(QueuedEvent::Event(event)) {
            Ok(()) => Self::notify(&lifecycle),
            Err(TrySendError::Full(_)) => {
                *terminal = Some(Terminal::EventQueueOverflow);
                Self::notify(&lifecycle);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
        Ok(())
    }

    fn terminate_if_open(&self, source: Arc<dyn StdError + Send + Sync>) -> TransportResult<()> {
        let lifecycle = lock(&self.lifecycle);
        if !lifecycle.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }

        let mut terminal = lock(&self.terminal);
        if let Some(terminal) = terminal.as_ref() {
            return Err(terminal_error(terminal));
        }
        *terminal = Some(Terminal::Source(source));
        match self.events.try_send(QueuedEvent::Terminal) {
            Ok(()) | Err(TrySendError::Full(_)) => Self::notify(&lifecycle),
            Err(TrySendError::Disconnected(_)) => {}
        }
        Ok(())
    }

    fn send_interrupt_if_active(&self, payload: &[u8]) -> TransportResult<SendAcceptance> {
        let lifecycle = lock(&self.lifecycle);
        if !lifecycle.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        let mut terminal = lock(&self.terminal);
        if let Some(terminal) = terminal.as_ref() {
            return Err(terminal_error(terminal));
        }

        let outcome = lock(&self.send_script)
            .pop_front()
            .unwrap_or(ScriptedSendOutcome::Accepted);
        let event = match outcome {
            ScriptedSendOutcome::Rejected => {
                return Err(TransportError::new(TransportErrorKind::SendRejected));
            }
            ScriptedSendOutcome::Closed => {
                *terminal = Some(Terminal::Closed);
                Self::notify(&lifecycle);
                return Err(TransportError::new(TransportErrorKind::Closed));
            }
            ScriptedSendOutcome::Accepted => None,
            ScriptedSendOutcome::AcceptedThenDisconnect { reason } => {
                Some(TransportEvent::Disconnected { reason })
            }
            ScriptedSendOutcome::AcceptedThenEvent(event) => Some(event),
        };
        lock(&self.accepted_interrupts).push(Box::from(payload));
        if let Some(event) = event {
            match self.events.try_send(QueuedEvent::Event(event)) {
                Ok(()) => Self::notify(&lifecycle),
                Err(TrySendError::Full(_)) => {
                    *terminal = Some(Terminal::EventQueueOverflow);
                    Self::notify(&lifecycle);
                }
                Err(TrySendError::Disconnected(_)) => {}
            }
        }
        Ok(SendAcceptance::ACCEPTED)
    }

    fn disconnect_if_active(&self) -> TransportResult<()> {
        let lifecycle = lock(&self.lifecycle);
        if !lifecycle.is_open {
            return Err(TransportError::new(TransportErrorKind::Closed));
        }
        if let Some(terminal) = lock(&self.terminal).as_ref() {
            return Err(terminal_error(terminal));
        }
        lock(&self.counters).disconnect += 1;
        Ok(())
    }

    fn notify(lifecycle: &FakeLifecycle) {
        if let Some(activity) = lifecycle.activity.as_ref() {
            activity.notify();
        }
    }
}

fn terminal_error(terminal: &Terminal) -> TransportError {
    match terminal {
        Terminal::Closed => TransportError::new(TransportErrorKind::Closed),
        Terminal::EventQueueOverflow => TransportError::new(TransportErrorKind::EventQueueOverflow),
        Terminal::Source(source) => {
            TransportError::with_source(TransportErrorKind::SourceTerminated, Arc::clone(source))
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
