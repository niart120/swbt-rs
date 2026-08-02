use std::fmt;

#[cfg(any(test, feature = "bumble"))]
use super::{TransportError, TransportErrorKind, TransportResult};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct TransportCapabilities {
    local_address: [u8; 6],
    classic_capable: bool,
}

impl TransportCapabilities {
    #[cfg(feature = "bumble")]
    pub(crate) fn from_validated_classic_controller(
        local_address: [u8; 6],
    ) -> TransportResult<Self> {
        if local_address == [0; 6] {
            return Err(TransportError::new(
                TransportErrorKind::InvalidControllerIdentity,
            ));
        }

        Ok(Self {
            local_address,
            classic_capable: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(local_address: [u8; 6], classic_capable: bool) -> TransportResult<Self> {
        if local_address == [0; 6] {
            return Err(TransportError::new(
                TransportErrorKind::InvalidControllerIdentity,
            ));
        }
        Ok(Self {
            local_address,
            classic_capable,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_default() -> Self {
        Self::for_test([0x00, 0x1b, 0xdc, 0xf9, 0x9f, 0x7d], true)
            .expect("test controller capabilities are valid")
    }

    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not project runtime transport capabilities"
        )
    )]
    pub(crate) const fn local_address(self) -> [u8; 6] {
        self.local_address
    }

    #[cfg_attr(
        not(any(test, feature = "bumble")),
        allow(
            dead_code,
            reason = "feature-disabled builds do not validate runtime transport capabilities"
        )
    )]
    pub(crate) const fn classic_capable(self) -> bool {
        self.classic_capable
    }
}

impl fmt::Debug for TransportCapabilities {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransportCapabilities")
            .field("classic_capable", &self.classic_capable)
            .finish_non_exhaustive()
    }
}
