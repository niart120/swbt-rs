#[cfg(feature = "bumble")]
mod bumble;
#[cfg(all(test, feature = "bumble"))]
mod bumble_tests;
mod capabilities;
mod config;

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

pub(crate) use capabilities::TransportCapabilities;
#[cfg(any(feature = "bumble", test))]
pub(crate) use capabilities::{ClassicAclBufferInfo, ControllerVersionInfo, UsbTransportMetadata};
pub(crate) use config::TransportConfig;

#[cfg(test)]
pub(in crate::runtime) mod fake;
#[cfg(test)]
mod tests;

#[derive(Clone)]
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M3 concrete transports and T24 worker construction share this notifier"
    )
)]
pub(crate) struct ActivityNotifier {
    sender: SyncSender<()>,
}

impl ActivityNotifier {
    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "T23 commands, T25 shutdown, and M3 transports wake the worker"
        )
    )]
    pub(crate) fn notify(&self) {
        match self.sender.try_send(()) {
            Ok(()) | Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => {}
        }
    }
}

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "T24 worker construction creates the shared activity channel"
    )
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
        allow(
            dead_code,
            reason = "T05 concrete transports report initialization failures"
        )
    )]
    OpenFailed,
    /// The controller returned an unusable all-zero public address.
    InvalidControllerIdentity,
    /// The controller lacks the Classic feature or ACL buffers required by NX.
    UnsupportedController,
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
    #[cfg(feature = "bumble")]
    CloseFailed,
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
            TransportErrorKind::OpenFailed => "transport could not be opened or initialized",
            TransportErrorKind::InvalidControllerIdentity => {
                "transport controller returned an invalid identity"
            }
            TransportErrorKind::UnsupportedController => {
                "transport controller lacks required Classic ACL capability"
            }
            TransportErrorKind::Closed => "transport is closed",
            TransportErrorKind::SendRejected => "transport rejected the send",
            TransportErrorKind::EventQueueOverflow => "transport event queue overflowed",
            TransportErrorKind::SourceTerminated => "transport source terminated",
            #[cfg(feature = "bumble")]
            TransportErrorKind::CloseFailed => "transport could not be closed cleanly",
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
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities>;

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>>;

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance>;

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()>;

    fn disconnect(&mut self) -> TransportResult<()>;

    fn close(&mut self) -> TransportResult<()>;
}
