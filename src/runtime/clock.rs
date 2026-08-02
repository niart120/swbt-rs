use std::time::Duration;

#[must_use]
pub(crate) fn fits_protocol_timestamp(duration: Duration) -> bool {
    duration.as_nanos() <= u128::from(u64::MAX)
}

#[must_use]
pub(crate) fn protocol_timestamp(now: Duration) -> u64 {
    u64::try_from(now.as_nanos()).unwrap_or(u64::MAX)
}

#[must_use]
pub(crate) fn deadline_after(now: Duration, duration: Duration) -> Duration {
    now.saturating_add(duration)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{deadline_after, fits_protocol_timestamp, protocol_timestamp};

    #[test]
    fn protocol_clock_saturates_after_u64_nanoseconds_without_deadline_overflow() {
        let maximum = Duration::from_nanos(u64::MAX);
        let beyond = maximum
            .checked_add(Duration::from_nanos(1))
            .expect("one nanosecond beyond the protocol range fits Duration");

        assert!(fits_protocol_timestamp(maximum));
        assert!(!fits_protocol_timestamp(beyond));
        assert_eq!(protocol_timestamp(maximum), u64::MAX);
        assert_eq!(protocol_timestamp(beyond), u64::MAX);
        assert_eq!(
            deadline_after(Duration::MAX, Duration::from_nanos(1)),
            Duration::MAX
        );
    }
}
