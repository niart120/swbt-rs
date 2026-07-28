#![cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T02 defines the transport contract before M2 worker integration"
    )
)]

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

#[cfg(test)]
mod fake;
#[cfg(test)]
mod tests;

#[derive(Clone)]
pub(crate) struct ActivityNotifier {
    sender: SyncSender<()>,
}

impl ActivityNotifier {
    pub(crate) fn notify(&self) {
        match self.sender.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => {}
        }
    }
}

pub(crate) fn activity_channel() -> (ActivityNotifier, Receiver<()>) {
    let (sender, receiver) = sync_channel(1);
    (ActivityNotifier { sender }, receiver)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SendAcceptance(());

impl SendAcceptance {
    const ACCEPTED: Self = Self(());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HidChannel {
    Control,
    Interrupt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TransportEvent {
    Connected,
    HidOutput {
        channel: HidChannel,
        payload: Box<[u8]>,
    },
    Disconnected {
        reason: Option<u8>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TransportErrorKind {
    Closed,
    SendRejected,
    EventQueueOverflow,
    SourceTerminated,
}

pub(crate) struct TransportError {
    kind: TransportErrorKind,
    source: Option<Arc<dyn StdError + Send + Sync>>,
}

impl TransportError {
    pub(crate) const fn new(kind: TransportErrorKind) -> Self {
        Self { kind, source: None }
    }

    pub(crate) fn with_source(
        kind: TransportErrorKind,
        source: Arc<dyn StdError + Send + Sync>,
    ) -> Self {
        Self {
            kind,
            source: Some(source),
        }
    }

    pub(crate) const fn kind(&self) -> TransportErrorKind {
        self.kind
    }
}

impl fmt::Debug for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportError")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            TransportErrorKind::Closed => "transport is closed",
            TransportErrorKind::SendRejected => "transport rejected the send",
            TransportErrorKind::EventQueueOverflow => "transport event queue overflowed",
            TransportErrorKind::SourceTerminated => "transport source terminated",
        };
        formatter.write_str(message)
    }
}

impl StdError for TransportError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn StdError + 'static))
    }
}

pub(crate) type TransportResult<T> = Result<T, TransportError>;

pub(crate) trait TransportPort: Send {
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()>;

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>>;

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance>;

    fn disconnect(&mut self) -> TransportResult<()>;

    fn close(&mut self) -> TransportResult<()>;
}
