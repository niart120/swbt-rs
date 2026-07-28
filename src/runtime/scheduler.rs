use std::{error::Error as StdError, fmt, time::Duration};

const NANOS_PER_SECOND: u128 = 1_000_000_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TickDecision {
    NotDue,
    Due { skipped: u64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SchedulerError {
    ZeroPeriod,
    DeadlineOverflow,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroPeriod => formatter.write_str("report period must be non-zero"),
            Self::DeadlineOverflow => formatter.write_str("report deadline overflow"),
        }
    }
}

impl StdError for SchedulerError {}

pub(crate) struct ReportScheduler {
    period: Duration,
    next_deadline: Duration,
}

impl ReportScheduler {
    pub(crate) fn start(started_at: Duration, period: Duration) -> Result<Self, SchedulerError> {
        if period.is_zero() {
            return Err(SchedulerError::ZeroPeriod);
        }
        let next_deadline = started_at
            .checked_add(period)
            .ok_or(SchedulerError::DeadlineOverflow)?;
        Ok(Self {
            period,
            next_deadline,
        })
    }

    #[must_use]
    pub(crate) const fn next_deadline(&self) -> Duration {
        self.next_deadline
    }

    pub(crate) fn step(&mut self, now: Duration) -> Result<TickDecision, SchedulerError> {
        if now < self.next_deadline {
            return Ok(TickDecision::NotDue);
        }

        let elapsed = now - self.next_deadline;
        let period_ns = self.period.as_nanos();
        let skipped = u64::try_from(elapsed.as_nanos() / period_ns)
            .map_err(|_| SchedulerError::DeadlineOverflow)?;
        let remainder_ns = elapsed.as_nanos() % period_ns;
        let until_next_ns = period_ns - remainder_ns;
        let until_next = duration_from_total_nanos(until_next_ns)?;
        let next_deadline = now
            .checked_add(until_next)
            .ok_or(SchedulerError::DeadlineOverflow)?;

        self.next_deadline = next_deadline;
        Ok(TickDecision::Due { skipped })
    }
}

fn duration_from_total_nanos(total_nanos: u128) -> Result<Duration, SchedulerError> {
    let seconds = u64::try_from(total_nanos / NANOS_PER_SECOND)
        .map_err(|_| SchedulerError::DeadlineOverflow)?;
    let nanoseconds = u32::try_from(total_nanos % NANOS_PER_SECOND)
        .map_err(|_| SchedulerError::DeadlineOverflow)?;
    Ok(Duration::new(seconds, nanoseconds))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{ReportScheduler, SchedulerError, TickDecision};

    const REPORT_PERIOD: Duration = Duration::from_millis(8);

    #[test]
    fn late_tick_keeps_the_absolute_eight_millisecond_phase() {
        let mut clock = FakeClock::at(Duration::from_millis(100));
        let mut scheduler =
            ReportScheduler::start(clock.now(), REPORT_PERIOD).expect("valid schedule");
        assert_eq!(scheduler.next_deadline(), Duration::from_millis(108));

        clock.set(Duration::from_millis(107));
        assert_eq!(
            scheduler.step(clock.now()).expect("deadline remains valid"),
            TickDecision::NotDue
        );
        assert_eq!(scheduler.next_deadline(), Duration::from_millis(108));

        clock.set(Duration::from_millis(110));
        assert_eq!(
            scheduler.step(clock.now()).expect("deadline remains valid"),
            TickDecision::Due { skipped: 0 }
        );
        assert_eq!(scheduler.next_deadline(), Duration::from_millis(116));

        clock.set(Duration::from_millis(116));
        assert_eq!(
            scheduler.step(clock.now()).expect("deadline remains valid"),
            TickDecision::Due { skipped: 0 }
        );
        assert_eq!(scheduler.next_deadline(), Duration::from_millis(124));
    }

    #[test]
    fn overrun_skips_missed_ticks_without_a_burst() {
        let mut clock = FakeClock::at(Duration::from_millis(100));
        let mut scheduler =
            ReportScheduler::start(clock.now(), REPORT_PERIOD).expect("valid schedule");

        clock.set(Duration::from_millis(132));
        assert_eq!(
            scheduler.step(clock.now()).expect("deadline remains valid"),
            TickDecision::Due { skipped: 3 }
        );
        assert_eq!(scheduler.next_deadline(), Duration::from_millis(140));
        assert_eq!(
            scheduler.step(clock.now()).expect("deadline remains valid"),
            TickDecision::NotDue
        );
        assert_eq!(scheduler.next_deadline(), Duration::from_millis(140));
    }

    #[test]
    fn invalid_period_and_deadline_overflow_are_reported_without_wrapping() {
        assert_eq!(
            ReportScheduler::start(Duration::ZERO, Duration::ZERO).err(),
            Some(SchedulerError::ZeroPeriod)
        );
        assert_eq!(
            ReportScheduler::start(Duration::MAX, Duration::from_nanos(1)).err(),
            Some(SchedulerError::DeadlineOverflow)
        );

        let mut scheduler = ReportScheduler::start(
            Duration::MAX - Duration::from_nanos(2),
            Duration::from_nanos(1),
        )
        .expect("first deadline is representable");
        assert_eq!(
            scheduler
                .step(Duration::MAX - Duration::from_nanos(1))
                .expect("last representable deadline"),
            TickDecision::Due { skipped: 0 }
        );
        assert_eq!(scheduler.next_deadline(), Duration::MAX);

        assert_eq!(
            scheduler.step(Duration::MAX),
            Err(SchedulerError::DeadlineOverflow)
        );
        assert_eq!(scheduler.next_deadline(), Duration::MAX);
    }

    struct FakeClock {
        now: Duration,
    }

    impl FakeClock {
        const fn at(now: Duration) -> Self {
            Self { now }
        }

        const fn now(&self) -> Duration {
            self.now
        }

        fn set(&mut self, now: Duration) {
            assert!(
                now >= self.now,
                "fake monotonic clock cannot move backwards"
            );
            self.now = now;
        }
    }
}
