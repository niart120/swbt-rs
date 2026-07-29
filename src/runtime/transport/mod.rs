use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

#[cfg(test)]
pub(in crate::runtime) mod fake;
#[cfg(test)]
mod tests;

#[derive(Clone)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "T22 registers the coalescing activity notifier")
)]
pub(crate) struct ActivityNotifier {
    sender: SyncSender<()>,
}

impl ActivityNotifier {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "T22 wakes the worker without blocking producers")
    )]
    pub(crate) fn notify(&self) {
        match self.sender.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => {}
        }
    }
}

#[cfg_attr(
    not(test),
    allow(dead_code, reason = "T22 constructs the coalescing activity channel")
)]
pub(crate) fn activity_channel() -> (ActivityNotifier, Receiver<()>) {
    let (sender, receiver) = sync_channel(1);
    (ActivityNotifier { sender }, receiver)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SendAcceptance(());

impl SendAcceptance {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "M3 concrete transports construct accepted send tokens"
        )
    )]
    pub(in crate::runtime) const ACCEPTED: Self = Self(());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M3 concrete transports produce HID channel events"
    )
)]
pub(crate) enum HidChannel {
    Control,
    Interrupt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "M3 concrete transports produce runtime events")
)]
pub(crate) enum TransportEvent {
    Connected,
    HidChannelOpened {
        channel: HidChannel,
    },
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
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "M3 concrete transports report closed ports")
    )]
    Closed,
    SendRejected,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "M3 concrete transports report event queue overflow"
        )
    )]
    EventQueueOverflow,
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "M3 concrete transports report terminated event sources"
        )
    )]
    SourceTerminated,
}

#[derive(Clone)]
pub(crate) struct TransportError {
    kind: TransportErrorKind,
    source: Option<Arc<dyn StdError + Send + Sync>>,
}

impl TransportError {
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "M3 concrete transports construct typed errors")
    )]
    pub(crate) const fn new(kind: TransportErrorKind) -> Self {
        Self { kind, source: None }
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "M3 concrete transports preserve sanitized backend sources"
        )
    )]
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
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "M3 concrete ports are opened by T31 orchestration"
        )
    )]
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<()>;

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>>;

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance>;

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()>;

    fn disconnect(&mut self) -> TransportResult<()>;

    fn close(&mut self) -> TransportResult<()>;
}
