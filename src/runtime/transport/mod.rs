mod bumble;
#[cfg(test)]
mod bumble_tests;
mod capabilities;
mod config;
mod profile_key_store;

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

pub(crate) use bumble::BumbleTransportPort;
pub(crate) use capabilities::TransportCapabilities;
pub(crate) use config::TransportConfig;
pub(crate) use profile_key_store::ProfileKeyStoreFactory;

#[cfg(test)]
pub(in crate::runtime) mod fake;
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
    pub(in crate::runtime) const ACCEPTED: Self = Self(());
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HidChannel {
    Control,
    Interrupt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    OpenFailed,
    /// The controller returned an unusable all-zero public address.
    InvalidControllerIdentity,
    /// The initialized controller address differs from the persisted identity.
    IdentityMismatch,
    /// An explicit identity write started, but the final adapter state is uncertain.
    AdapterIdentityRecoveryRequired,
    /// The controller lacks the Classic feature or ACL buffers required by NX.
    UnsupportedController,
    /// The configured pairing profile could not supply or persist key material.
    InvalidKeyStore,
    /// The configured pairing profile has no usable Classic bond.
    NoBond,
    Closed,
    SendRejected,
    DrainTimedOut,
    EventQueueOverflow,
    SourceTerminated,
    CloseFailed,
}

#[derive(Clone)]
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
            TransportErrorKind::OpenFailed => "transport could not be opened or initialized",
            TransportErrorKind::InvalidControllerIdentity => {
                "transport controller returned an invalid identity"
            }
            TransportErrorKind::IdentityMismatch => {
                "transport controller identity does not match the pairing profile"
            }
            TransportErrorKind::AdapterIdentityRecoveryRequired => {
                "transport adapter identity is uncertain after a write"
            }
            TransportErrorKind::UnsupportedController => {
                "transport controller lacks required Classic ACL capability"
            }
            TransportErrorKind::InvalidKeyStore => {
                "transport pairing key store could not be read or updated"
            }
            TransportErrorKind::NoBond => "transport has no usable Classic bond",
            TransportErrorKind::Closed => "transport is closed",
            TransportErrorKind::SendRejected => "transport rejected the send",
            TransportErrorKind::DrainTimedOut => "transport send drain timed out",
            TransportErrorKind::EventQueueOverflow => "transport event queue overflowed",
            TransportErrorKind::SourceTerminated => "transport source terminated",
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
    fn open(&mut self, activity: ActivityNotifier) -> TransportResult<TransportCapabilities>;

    fn start_pairing(&mut self) -> TransportResult<()>;

    fn start_reconnect(&mut self) -> TransportResult<()> {
        Err(TransportError::new(TransportErrorKind::NoBond))
    }

    fn poll(&mut self, timeout: Duration) -> TransportResult<Vec<TransportEvent>>;

    /// Whether an automatic report can enter the transport without waiting in
    /// an internal controller queue.
    fn interrupt_send_capacity_available(&self) -> bool {
        true
    }

    fn send_interrupt(&mut self, payload: &[u8]) -> TransportResult<SendAcceptance>;

    /// Wait until pending interrupt packets have left the host-side queue.
    ///
    /// Packets already in the controller's flow-control window do not keep this
    /// operation pending while completion credit is outstanding.
    fn drain_interrupt(&mut self, timeout: Duration) -> TransportResult<()>;

    fn disconnect(&mut self) -> TransportResult<()>;

    fn close(&mut self) -> TransportResult<()>;
}
