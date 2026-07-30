#[cfg(feature = "bumble")]
mod bumble;
#[cfg(all(test, feature = "bumble"))]
mod bumble_tests;
mod capabilities;
#[cfg(feature = "bumble")]
mod classic;
mod config;
#[cfg(feature = "bumble")]
mod hidp;
#[cfg(feature = "bumble")]
mod sdp;
#[cfg(all(test, feature = "bumble"))]
mod virtual_tests;

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

#[cfg(feature = "bumble")]
pub(crate) use bumble::BumbleTransportPort;
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
    not(any(test, feature = "bumble")),
    allow(
        dead_code,
        reason = "feature-disabled builds do not construct transport activity notifiers"
    )
)]
pub(crate) struct ActivityNotifier {
    sender: SyncSender<()>,
}

impl ActivityNotifier {
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not notify transport activity"
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
        allow(dead_code, reason = "M4 constructs accepted HID interrupt send tokens")
    )]
    pub(in crate::runtime) const ACCEPTED: Self = Self(());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code, reason = "M4 produces HID channel events"))]
pub(crate) enum HidChannel {
    Control,
    Interrupt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    allow(dead_code, reason = "M4 produces connection and HID runtime events")
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
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not construct concrete closed ports"
        )
    )]
    Closed,
    SendRejected,
    #[cfg_attr(
        not(feature = "bumble"),
        allow(
            dead_code,
            reason = "feature-disabled builds do not perform concrete ACL drain waits"
        )
    )]
    DrainTimedOut,
    #[cfg_attr(
        not(test),
        allow(dead_code, reason = "M4 bounds the connection and HID event queue")
    )]
    EventQueueOverflow,
    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not observe transport source termination"
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
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not construct concrete transport errors"
        )
    )]
    pub(crate) const fn new(kind: TransportErrorKind) -> Self {
        Self { kind, source: None }
    }

    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not preserve concrete backend sources"
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
            TransportErrorKind::DrainTimedOut => "transport send drain timed out",
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
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not open concrete transport ports"
        )
    )]
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities>;

    fn start_pairing(&mut self) -> TransportResult<()>;

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>>;

    /// Whether an automatic report can enter the transport without waiting in
    /// an internal controller queue.
    fn interrupt_send_capacity_available(&self) -> bool {
        true
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance>;

    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()>;

    fn disconnect(&mut self) -> TransportResult<()>;

    fn close(&mut self) -> TransportResult<()>;
}
