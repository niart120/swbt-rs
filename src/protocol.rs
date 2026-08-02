//! Private bridge to the backend-independent protocol engine.

pub(crate) use swbt_core::__private::{
    ImuEncodingState, InputPreparation, OutputReport, PreparedOutputAction, PreparedSessionReply,
    PreparedSubcommandReply, ProtocolError, ProtocolSession, RawRumble, SubcommandRequest,
    SwitchHidProtocol, parse_output_report,
};
