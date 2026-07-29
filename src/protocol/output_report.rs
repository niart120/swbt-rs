//! Host-to-device output report parsing.

use super::error::ProtocolError;

const SUBCOMMAND_REPORT_ID: u8 = 0x01;
const RUMBLE_REPORT_ID: u8 = 0x10;
const RUMBLE_SIZE: usize = 8;
const SUBCOMMAND_MINIMUM_SIZE: usize = 11;
const RUMBLE_MINIMUM_SIZE: usize = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RawRumble([u8; RUMBLE_SIZE]);

impl RawRumble {
    #[must_use]
    #[cfg(test)]
    pub(crate) const fn bytes(&self) -> &[u8; RUMBLE_SIZE] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SubcommandRequest<'a> {
    id: u8,
    payload: &'a [u8],
}

impl<'a> SubcommandRequest<'a> {
    #[must_use]
    pub(crate) const fn id(self) -> u8 {
        self.id
    }

    #[must_use]
    pub(crate) const fn payload(self) -> &'a [u8] {
        self.payload
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputReport<'a> {
    Subcommand {
        packet_id: u8,
        rumble: RawRumble,
        request: SubcommandRequest<'a>,
    },
    Rumble {
        packet_id: u8,
        rumble: RawRumble,
    },
}

impl<'a> OutputReport<'a> {
    #[must_use]
    pub(crate) const fn report_id(self) -> u8 {
        match self {
            Self::Subcommand { .. } => SUBCOMMAND_REPORT_ID,
            Self::Rumble { .. } => RUMBLE_REPORT_ID,
        }
    }

    #[must_use]
    pub(crate) const fn packet_id(self) -> u8 {
        match self {
            Self::Subcommand { packet_id, .. } | Self::Rumble { packet_id, .. } => packet_id,
        }
    }

    #[must_use]
    pub(crate) const fn rumble(&self) -> &RawRumble {
        match self {
            Self::Subcommand { rumble, .. } | Self::Rumble { rumble, .. } => rumble,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub(crate) const fn subcommand(self) -> Option<SubcommandRequest<'a>> {
        match self {
            Self::Subcommand { request, .. } => Some(request),
            Self::Rumble { .. } => None,
        }
    }
}

pub(crate) fn parse_output_report(raw: &[u8]) -> Result<OutputReport<'_>, ProtocolError> {
    let Some(report_id) = raw.first().copied() else {
        return Err(ProtocolError::OutputReportEmpty);
    };
    match report_id {
        SUBCOMMAND_REPORT_ID => parse_subcommand_report(raw),
        RUMBLE_REPORT_ID => parse_rumble_report(raw),
        _ => Err(ProtocolError::UnsupportedOutputReport { report_id }),
    }
}

fn parse_subcommand_report(raw: &[u8]) -> Result<OutputReport<'_>, ProtocolError> {
    require_length(raw, SUBCOMMAND_REPORT_ID, SUBCOMMAND_MINIMUM_SIZE)?;
    Ok(OutputReport::Subcommand {
        packet_id: raw[1],
        rumble: copy_rumble(raw),
        request: SubcommandRequest {
            id: raw[10],
            payload: &raw[11..],
        },
    })
}

fn parse_rumble_report(raw: &[u8]) -> Result<OutputReport<'_>, ProtocolError> {
    require_length(raw, RUMBLE_REPORT_ID, RUMBLE_MINIMUM_SIZE)?;
    Ok(OutputReport::Rumble {
        packet_id: raw[1],
        rumble: copy_rumble(raw),
    })
}

fn require_length(raw: &[u8], report_id: u8, minimum: usize) -> Result<(), ProtocolError> {
    if raw.len() < minimum {
        Err(ProtocolError::TruncatedOutputReport {
            report_id,
            minimum,
            actual: raw.len(),
        })
    } else {
        Ok(())
    }
}

fn copy_rumble(raw: &[u8]) -> RawRumble {
    let mut rumble = [0; RUMBLE_SIZE];
    rumble.copy_from_slice(&raw[2..10]);
    RawRumble(rumble)
}
