use std::num::NonZeroU64;

#[cfg(test)]
use serde_json::{Map, Value, json};

use crate::{
    diagnostics::LifecycleState,
    model::{ButtonKind, ControllerKind},
    reporting::ReportingKind,
};

pub(crate) const DIAGNOSTICS_TARGET: &str = "swbt::diagnostics";
pub(crate) const DIAGNOSTICS_SCHEMA: &str = "swbt.diagnostics";
pub(crate) const DIAGNOSTICS_SCHEMA_VERSION: u64 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticContext {
    controller_kind: ControllerKind,
    reporting_kind: ReportingKind,
    session_id: NonZeroU64,
}

impl DiagnosticContext {
    pub(crate) const fn new(
        controller_kind: ControllerKind,
        reporting_kind: ReportingKind,
        session_id: NonZeroU64,
    ) -> Self {
        Self {
            controller_kind,
            reporting_kind,
            session_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkerFailureCategory {
    Transport,
    Wait,
    CommandDelivery,
    Panicked,
    CompletionDisconnected,
    Internal,
}

impl WorkerFailureCategory {
    const fn name(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::Wait => "wait",
            Self::CommandDelivery => "command_delivery",
            Self::Panicked => "panicked",
            Self::CompletionDisconnected => "completion_disconnected",
            Self::Internal => "internal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiagnosticEvent {
    Environment {
        controller_kind: ControllerKind,
        reporting_kind: ReportingKind,
    },
    SessionStarted {
        context: DiagnosticContext,
    },
    LifecycleChanged {
        context: DiagnosticContext,
        lifecycle: LifecycleState,
    },
    SubcommandObserved {
        context: DiagnosticContext,
        subcommand_id: u8,
    },
    ReportTxAccepted {
        context: DiagnosticContext,
        report_mode: Option<u8>,
        input_reports_accepted: u64,
    },
    ReplyTxAccepted {
        context: DiagnosticContext,
        report_mode: Option<u8>,
        replies_accepted: u64,
    },
    SessionEnded {
        context: DiagnosticContext,
        lifecycle: LifecycleState,
        disconnect_reason: Option<u8>,
    },
    WorkerFailed {
        context: DiagnosticContext,
        failure_category: WorkerFailureCategory,
    },
    UnsupportedButton {
        context: DiagnosticContext,
        button_kind: ButtonKind,
    },
}

impl DiagnosticEvent {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "M8 T08 wires the probe environment event into trace capture"
        )
    )]
    pub(crate) const fn environment(
        controller_kind: ControllerKind,
        reporting_kind: ReportingKind,
    ) -> Self {
        Self::Environment {
            controller_kind,
            reporting_kind,
        }
    }

    pub(crate) const fn session_started(context: DiagnosticContext) -> Self {
        Self::SessionStarted { context }
    }

    pub(crate) const fn lifecycle_changed(
        context: DiagnosticContext,
        lifecycle: LifecycleState,
    ) -> Self {
        Self::LifecycleChanged { context, lifecycle }
    }

    pub(crate) const fn subcommand_observed(context: DiagnosticContext, subcommand_id: u8) -> Self {
        Self::SubcommandObserved {
            context,
            subcommand_id,
        }
    }

    pub(crate) const fn report_tx_accepted(
        context: DiagnosticContext,
        report_mode: Option<u8>,
        input_reports_accepted: u64,
    ) -> Self {
        Self::ReportTxAccepted {
            context,
            report_mode,
            input_reports_accepted,
        }
    }

    pub(crate) const fn reply_tx_accepted(
        context: DiagnosticContext,
        report_mode: Option<u8>,
        replies_accepted: u64,
    ) -> Self {
        Self::ReplyTxAccepted {
            context,
            report_mode,
            replies_accepted,
        }
    }

    pub(crate) const fn session_ended(
        context: DiagnosticContext,
        lifecycle: LifecycleState,
        disconnect_reason: Option<u8>,
    ) -> Self {
        Self::SessionEnded {
            context,
            lifecycle,
            disconnect_reason,
        }
    }

    pub(crate) const fn worker_failed(
        context: DiagnosticContext,
        failure_category: WorkerFailureCategory,
    ) -> Self {
        Self::WorkerFailed {
            context,
            failure_category,
        }
    }

    pub(crate) const fn unsupported_button(
        context: DiagnosticContext,
        button_kind: ButtonKind,
    ) -> Self {
        Self::UnsupportedButton {
            context,
            button_kind,
        }
    }

    pub(crate) fn emit(self) {
        match self {
            Self::Environment {
                controller_kind,
                reporting_kind,
            } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "environment",
                controller_kind = controller_kind.profile_name(),
                reporting_kind = reporting_name(reporting_kind),
                package_version = env!("CARGO_PKG_VERSION"),
                target_os = std::env::consts::OS,
                target_arch = std::env::consts::ARCH,
            ),
            Self::SessionStarted { context } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "session_started",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
            ),
            Self::LifecycleChanged { context, lifecycle } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "lifecycle_changed",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
                lifecycle = lifecycle_name(lifecycle),
            ),
            Self::SubcommandObserved {
                context,
                subcommand_id,
            } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "subcommand_observed",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
                subcommand_id,
            ),
            Self::ReportTxAccepted {
                context,
                report_mode,
                input_reports_accepted,
            } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "report_tx_accepted",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
                report_mode,
                input_reports_accepted,
            ),
            Self::ReplyTxAccepted {
                context,
                report_mode,
                replies_accepted,
            } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "reply_tx_accepted",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
                report_mode,
                replies_accepted,
            ),
            Self::SessionEnded {
                context,
                lifecycle,
                disconnect_reason,
            } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "session_ended",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
                lifecycle = lifecycle_name(lifecycle),
                disconnect_reason,
            ),
            Self::WorkerFailed {
                context,
                failure_category,
            } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "worker_failed",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
                failure_category = failure_category.name(),
            ),
            Self::UnsupportedButton {
                context,
                button_kind,
            } => tracing::event!(
                target: DIAGNOSTICS_TARGET,
                tracing::Level::INFO,
                schema = DIAGNOSTICS_SCHEMA,
                schema_version = DIAGNOSTICS_SCHEMA_VERSION,
                event = "unsupported_button",
                controller_kind = context.controller_kind.profile_name(),
                reporting_kind = reporting_name(context.reporting_kind),
                session_id = context.session_id.get(),
                button_kind = button_name(button_kind),
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn to_value(self) -> Value {
        match self {
            Self::Environment {
                controller_kind,
                reporting_kind,
            } => json!({
                "schema": DIAGNOSTICS_SCHEMA,
                "schema_version": DIAGNOSTICS_SCHEMA_VERSION,
                "event": "environment",
                "controller_kind": controller_kind.profile_name(),
                "reporting_kind": reporting_name(reporting_kind),
                "package_version": env!("CARGO_PKG_VERSION"),
                "target_os": std::env::consts::OS,
                "target_arch": std::env::consts::ARCH,
            }),
            Self::SessionStarted { context } => record(context, "session_started", Map::new()),
            Self::LifecycleChanged { context, lifecycle } => record(
                context,
                "lifecycle_changed",
                fields([("lifecycle", json!(lifecycle_name(lifecycle)))]),
            ),
            Self::SubcommandObserved {
                context,
                subcommand_id,
            } => record(
                context,
                "subcommand_observed",
                fields([("subcommand_id", json!(subcommand_id))]),
            ),
            Self::ReportTxAccepted {
                context,
                report_mode,
                input_reports_accepted,
            } => record(
                context,
                "report_tx_accepted",
                fields([
                    ("report_mode", json!(report_mode)),
                    ("input_reports_accepted", json!(input_reports_accepted)),
                ]),
            ),
            Self::ReplyTxAccepted {
                context,
                report_mode,
                replies_accepted,
            } => record(
                context,
                "reply_tx_accepted",
                fields([
                    ("report_mode", json!(report_mode)),
                    ("replies_accepted", json!(replies_accepted)),
                ]),
            ),
            Self::SessionEnded {
                context,
                lifecycle,
                disconnect_reason,
            } => record(
                context,
                "session_ended",
                fields([
                    ("lifecycle", json!(lifecycle_name(lifecycle))),
                    ("disconnect_reason", json!(disconnect_reason)),
                ]),
            ),
            Self::WorkerFailed {
                context,
                failure_category,
            } => record(
                context,
                "worker_failed",
                fields([("failure_category", json!(failure_category.name()))]),
            ),
            Self::UnsupportedButton {
                context,
                button_kind,
            } => record(
                context,
                "unsupported_button",
                fields([("button_kind", json!(button_name(button_kind)))]),
            ),
        }
    }
}

#[cfg(test)]
fn record(context: DiagnosticContext, event: &'static str, extra: Map<String, Value>) -> Value {
    let mut record = json!({
        "schema": DIAGNOSTICS_SCHEMA,
        "schema_version": DIAGNOSTICS_SCHEMA_VERSION,
        "event": event,
        "controller_kind": context.controller_kind.profile_name(),
        "reporting_kind": reporting_name(context.reporting_kind),
        "session_id": context.session_id.get(),
    });
    record.as_object_mut().unwrap().extend(extra);
    record
}

#[cfg(test)]
fn fields<const N: usize>(fields: [(&'static str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value))
        .collect()
}

const fn reporting_name(kind: ReportingKind) -> &'static str {
    match kind {
        ReportingKind::Periodic => "periodic",
        ReportingKind::Direct => "direct",
    }
}

const fn lifecycle_name(state: LifecycleState) -> &'static str {
    match state {
        LifecycleState::Configured => "configured",
        LifecycleState::Open => "open",
        LifecycleState::Connecting => "connecting",
        LifecycleState::Ready => "ready",
        LifecycleState::Closing => "closing",
        LifecycleState::Closed => "closed",
        LifecycleState::Failed => "failed",
    }
}

const fn button_name(kind: ButtonKind) -> &'static str {
    match kind {
        ButtonKind::A => "a",
        ButtonKind::B => "b",
        ButtonKind::X => "x",
        ButtonKind::Y => "y",
        ButtonKind::L => "l",
        ButtonKind::R => "r",
        ButtonKind::ZL => "zl",
        ButtonKind::ZR => "zr",
        ButtonKind::Plus => "plus",
        ButtonKind::Minus => "minus",
        ButtonKind::Home => "home",
        ButtonKind::Capture => "capture",
        ButtonKind::LeftStick => "left_stick",
        ButtonKind::RightStick => "right_stick",
        ButtonKind::SL => "sl",
        ButtonKind::SR => "sr",
        ButtonKind::DpadUp => "dpad_up",
        ButtonKind::DpadDown => "dpad_down",
        ButtonKind::DpadLeft => "dpad_left",
        ButtonKind::DpadRight => "dpad_right",
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use serde_json::{Value, json};

    use super::{
        DIAGNOSTICS_SCHEMA, DIAGNOSTICS_SCHEMA_VERSION, DIAGNOSTICS_TARGET, DiagnosticContext,
        DiagnosticEvent, WorkerFailureCategory,
    };
    use crate::{
        diagnostics::LifecycleState,
        model::{ButtonKind, ControllerKind},
        reporting::ReportingKind,
    };

    #[test]
    fn stable_event_records_have_exact_versioned_names_and_fields() {
        let context = DiagnosticContext::new(
            ControllerKind::JoyConR,
            ReportingKind::Direct,
            NonZeroU64::new(7).unwrap(),
        );
        let cases = [
            (
                DiagnosticEvent::environment(ControllerKind::JoyConR, ReportingKind::Direct),
                json!({
                    "schema": "swbt.diagnostics",
                    "schema_version": 1,
                    "event": "environment",
                    "controller_kind": "joycon_r",
                    "reporting_kind": "direct",
                    "package_version": env!("CARGO_PKG_VERSION"),
                    "target_os": std::env::consts::OS,
                    "target_arch": std::env::consts::ARCH,
                }),
            ),
            (
                DiagnosticEvent::session_started(context),
                record_with_context("session_started", json!({}), 7),
            ),
            (
                DiagnosticEvent::lifecycle_changed(context, LifecycleState::Ready),
                record_with_context("lifecycle_changed", json!({"lifecycle": "ready"}), 7),
            ),
            (
                DiagnosticEvent::subcommand_observed(context, 0x40),
                record_with_context("subcommand_observed", json!({"subcommand_id": 0x40}), 7),
            ),
            (
                DiagnosticEvent::report_tx_accepted(context, Some(0x30), 12),
                record_with_context(
                    "report_tx_accepted",
                    json!({"report_mode": 0x30, "input_reports_accepted": 12}),
                    7,
                ),
            ),
            (
                DiagnosticEvent::reply_tx_accepted(context, Some(0x30), 4),
                record_with_context(
                    "reply_tx_accepted",
                    json!({"report_mode": 0x30, "replies_accepted": 4}),
                    7,
                ),
            ),
            (
                DiagnosticEvent::session_ended(context, LifecycleState::Open, Some(0x13)),
                record_with_context(
                    "session_ended",
                    json!({"lifecycle": "open", "disconnect_reason": 0x13}),
                    7,
                ),
            ),
            (
                DiagnosticEvent::worker_failed(context, WorkerFailureCategory::Transport),
                record_with_context("worker_failed", json!({"failure_category": "transport"}), 7),
            ),
            (
                DiagnosticEvent::unsupported_button(context, ButtonKind::DpadUp),
                record_with_context("unsupported_button", json!({"button_kind": "dpad_up"}), 7),
            ),
        ];

        assert_eq!(DIAGNOSTICS_TARGET, "swbt::diagnostics");
        assert_eq!(DIAGNOSTICS_SCHEMA, "swbt.diagnostics");
        assert_eq!(DIAGNOSTICS_SCHEMA_VERSION, 1);
        for (event, expected) in cases {
            assert_eq!(event.to_value(), expected);
        }
    }

    #[test]
    fn event_records_cannot_carry_sensitive_or_unbounded_text_fields() {
        let context = DiagnosticContext::new(
            ControllerKind::Pro,
            ReportingKind::Periodic,
            NonZeroU64::new(1).unwrap(),
        );
        let records = [
            DiagnosticEvent::environment(ControllerKind::Pro, ReportingKind::Periodic),
            DiagnosticEvent::session_started(context),
            DiagnosticEvent::lifecycle_changed(context, LifecycleState::Connecting),
            DiagnosticEvent::subcommand_observed(context, 0x03),
            DiagnosticEvent::report_tx_accepted(context, None, u64::MAX),
            DiagnosticEvent::reply_tx_accepted(context, None, u64::MAX),
            DiagnosticEvent::session_ended(context, LifecycleState::Open, None),
            DiagnosticEvent::worker_failed(context, WorkerFailureCategory::Panicked),
            DiagnosticEvent::unsupported_button(context, ButtonKind::A),
        ]
        .map(DiagnosticEvent::to_value);

        for record in records {
            let object = record.as_object().unwrap();
            for forbidden in [
                "adapter_selector",
                "usb_bus",
                "usb_address",
                "usb_port",
                "usb_serial",
                "profile_path",
                "profile_json",
                "peer_address",
                "local_address",
                "link_key",
                "raw_packet",
                "error",
                "error_source",
                "message",
            ] {
                assert!(
                    !object.contains_key(forbidden),
                    "forbidden field: {forbidden}"
                );
            }
            assert!(object.values().all(stable_scalar_or_null));
        }
    }

    #[test]
    fn failure_categories_have_stable_closed_values() {
        let context = DiagnosticContext::new(
            ControllerKind::Pro,
            ReportingKind::Periodic,
            NonZeroU64::new(1).unwrap(),
        );
        let actual = [
            WorkerFailureCategory::Transport,
            WorkerFailureCategory::Wait,
            WorkerFailureCategory::CommandDelivery,
            WorkerFailureCategory::Panicked,
            WorkerFailureCategory::CompletionDisconnected,
            WorkerFailureCategory::Internal,
        ]
        .map(|category| {
            DiagnosticEvent::worker_failed(context, category).to_value()["failure_category"]
                .as_str()
                .unwrap()
                .to_owned()
        });

        assert_eq!(
            actual,
            [
                "transport",
                "wait",
                "command_delivery",
                "panicked",
                "completion_disconnected",
                "internal",
            ]
        );
    }

    fn record_with_context(event: &str, fields: Value, session_id: u64) -> Value {
        let mut record = json!({
            "schema": "swbt.diagnostics",
            "schema_version": 1,
            "event": event,
            "controller_kind": "joycon_r",
            "reporting_kind": "direct",
            "session_id": session_id,
        });
        record
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        record
    }

    fn stable_scalar_or_null(value: &Value) -> bool {
        value.is_null() || value.is_boolean() || value.is_number() || value.is_string()
    }
}
