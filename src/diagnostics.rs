use crate::model::ControllerKind;
use crate::reporting::ReportingKind;

#[expect(
    dead_code,
    reason = "M8 T02 fixes the event contract before T03 wires runtime emission"
)]
pub(crate) mod event;

/// Lifecycle state of a controller runtime.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LifecycleState {
    /// The controller has configuration but has not opened its transport.
    Configured,
    /// The transport is open and no connection attempt is active.
    Open,
    /// A connection attempt is active.
    Connecting,
    /// The current connection is ready to accept input commands.
    Ready,
    /// Shutdown has started and new input commands are rejected.
    Closing,
    /// Runtime resources have been closed.
    Closed,
    /// The worker terminated because of an error or panic.
    Failed,
}

/// Read-only runtime diagnostics for a controller.
///
/// Accepted-report counters record transport acceptance. They do not confirm
/// delivery over the air or a change visible in the console UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GamepadStatus {
    /// Current controller lifecycle state.
    pub lifecycle: LifecycleState,
    /// Whether the current session has an active connection.
    pub connected: bool,
    /// Controller model projected from the controller type.
    pub controller_kind: ControllerKind,
    /// Reporting mode projected from the controller type.
    pub reporting_kind: ReportingKind,
    /// Input report mode accepted for the current protocol session.
    pub report_mode: Option<u8>,
    /// Number of input reports accepted during this controller's lifetime.
    pub input_reports_accepted: u64,
    /// Number of subcommand replies accepted during this controller's lifetime.
    pub replies_accepted: u64,
    /// Most recently parsed subcommand identifier in the current session.
    pub last_subcommand: Option<u8>,
    /// Most recent disconnect reason for the current session.
    pub last_disconnect_reason: Option<u8>,
    /// Sanitized description of a terminal worker error or panic.
    pub worker_failure: Option<String>,
}
