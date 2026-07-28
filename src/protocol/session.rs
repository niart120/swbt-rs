//! Connection-scoped pure protocol state.

use super::imu::{ImuEncodingState, ImuMode};

const SUPPORTED_INPUT_REPORT_MODE: u8 = 0x30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReportModeSelection {
    StandardFull,
    Unsupported(u8),
}

impl ReportModeSelection {
    const fn from_requested(requested: u8) -> Self {
        if requested == SUPPORTED_INPUT_REPORT_MODE {
            Self::StandardFull
        } else {
            Self::Unsupported(requested)
        }
    }

    const fn requested(self) -> u8 {
        match self {
            Self::StandardFull => SUPPORTED_INPUT_REPORT_MODE,
            Self::Unsupported(requested) => requested,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ProtocolSession {
    report_mode: Option<ReportModeSelection>,
    player_lights: Option<u8>,
    imu_mode: ImuMode,
    imu_encoding_state: ImuEncodingState,
    vibration_enabled: bool,
}

impl ProtocolSession {
    #[must_use]
    pub(crate) const fn report_mode(self) -> Option<u8> {
        match self.report_mode {
            Some(selection) => Some(selection.requested()),
            None => None,
        }
    }

    #[must_use]
    pub(crate) const fn report_mode_supported(self) -> bool {
        matches!(self.report_mode, Some(ReportModeSelection::StandardFull))
    }

    #[must_use]
    pub(crate) const fn unsupported_report_mode(self) -> Option<u8> {
        match self.report_mode {
            Some(ReportModeSelection::Unsupported(requested)) => Some(requested),
            Some(ReportModeSelection::StandardFull) | None => None,
        }
    }

    #[must_use]
    pub(crate) const fn player_lights(self) -> Option<u8> {
        self.player_lights
    }

    #[must_use]
    pub(crate) const fn imu_mode(self) -> ImuMode {
        self.imu_mode
    }

    #[must_use]
    pub(crate) const fn imu_enabled(self) -> bool {
        !matches!(self.imu_mode, ImuMode::Disabled)
    }

    #[must_use]
    pub(crate) const fn imu_encoding_state(self) -> ImuEncodingState {
        self.imu_encoding_state
    }

    #[must_use]
    pub(crate) const fn vibration_enabled(self) -> bool {
        self.vibration_enabled
    }

    #[must_use]
    pub(crate) const fn protocol_ready(self) -> bool {
        self.report_mode_supported()
            && matches!(self.player_lights, Some(player_lights) if player_lights != 0)
    }

    #[must_use]
    pub(crate) const fn with_report_mode(mut self, requested: u8) -> Self {
        self.report_mode = Some(ReportModeSelection::from_requested(requested));
        self
    }

    #[must_use]
    pub(crate) const fn with_player_lights(mut self, player_lights: u8) -> Self {
        self.player_lights = Some(player_lights);
        self
    }

    #[must_use]
    pub(crate) fn with_imu_mode(mut self, imu_mode: ImuMode) -> Self {
        self.imu_mode = imu_mode;
        self.imu_encoding_state = ImuEncodingState::default();
        self
    }

    #[must_use]
    pub(crate) const fn with_imu_encoding_state(
        mut self,
        imu_encoding_state: ImuEncodingState,
    ) -> Self {
        self.imu_encoding_state = imu_encoding_state;
        self
    }

    #[must_use]
    pub(crate) const fn with_vibration_enabled(mut self, vibration_enabled: bool) -> Self {
        self.vibration_enabled = vibration_enabled;
        self
    }
}

impl Default for ProtocolSession {
    fn default() -> Self {
        Self {
            report_mode: None,
            player_lights: None,
            imu_mode: ImuMode::Disabled,
            imu_encoding_state: ImuEncodingState::default(),
            vibration_enabled: false,
        }
    }
}
