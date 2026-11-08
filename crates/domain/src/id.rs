//! Snowflake IDs: 1 sign bit (always 0) | 41 bits ms since [`EPOCH_UNIX_MS`] | 10 bits worker | 12 bits sequence.

use std::{fmt, str::FromStr};

use thiserror::Error;

/// 2026-01-01T00:00:00Z in Unix milliseconds.
pub const EPOCH_UNIX_MS: u64 = 1_767_225_600_000;

const WORKER_BITS: u32 = 10;
const SEQUENCE_BITS: u32 = 12;
const TIMESTAMP_SHIFT: u32 = WORKER_BITS + SEQUENCE_BITS;

pub const MAX_TIMESTAMP_MS: u64 = (1 << 41) - 1;
pub const MAX_WORKER: u16 = (1 << WORKER_BITS) - 1;
pub const MAX_SEQUENCE: u16 = (1 << SEQUENCE_BITS) - 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnowflakeId(i64);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid snowflake id")]
pub struct InvalidSnowflakeId;

impl SnowflakeId {
    fn from_parts(timestamp_ms: u64, worker: WorkerId, sequence: u16) -> Self {
        debug_assert!(timestamp_ms <= MAX_TIMESTAMP_MS && sequence <= MAX_SEQUENCE);
        let raw = (timestamp_ms << TIMESTAMP_SHIFT)
            | (u64::from(worker.0) << SEQUENCE_BITS)
            | u64::from(sequence);
        Self(raw as i64)
    }

    pub fn as_i64(self) -> i64 {
        self.0
    }

    /// Milliseconds since [`EPOCH_UNIX_MS`].
    pub fn timestamp_ms(self) -> u64 {
        (self.0 as u64) >> TIMESTAMP_SHIFT
    }

    pub fn worker(self) -> WorkerId {
        WorkerId(((self.0 as u64 >> SEQUENCE_BITS) & u64::from(MAX_WORKER)) as u16)
    }

    pub fn sequence(self) -> u16 {
        (self.0 as u64 & u64::from(MAX_SEQUENCE)) as u16
    }
}

impl TryFrom<i64> for SnowflakeId {
    type Error = InvalidSnowflakeId;

    fn try_from(raw: i64) -> Result<Self, Self::Error> {
        if raw < 0 {
            return Err(InvalidSnowflakeId);
        }
        Ok(Self(raw))
    }
}

impl fmt::Display for SnowflakeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SnowflakeId {
    type Err = InvalidSnowflakeId;

    /// Accepts only plain decimal digits: no sign, no whitespace.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(InvalidSnowflakeId);
        }
        s.parse::<i64>()
            .map_err(|_| InvalidSnowflakeId)
            .and_then(Self::try_from)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkerId(u16);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("worker id {0} is out of range 0..={MAX_WORKER}")]
pub struct InvalidWorkerId(pub u16);

impl WorkerId {
    pub fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for WorkerId {
    type Error = InvalidWorkerId;

    fn try_from(raw: u16) -> Result<Self, Self::Error> {
        if raw > MAX_WORKER {
            return Err(InvalidWorkerId(raw));
        }
        Ok(Self(raw))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NextIdError {
    #[error("clock moved backwards: now {now_ms} ms < last {last_ms} ms")]
    ClockBehind { now_ms: u64, last_ms: u64 },
    #[error("sequence exhausted for {ms} ms")]
    SequenceExhausted { ms: u64 },
    #[error("timestamp no longer fits in 41 bits")]
    TimestampOverflow,
}

/// Issues strictly increasing ids for one worker. Time is passed in, so the
/// caller owns the clock and this stays a pure state machine.
#[derive(Debug, Clone)]
pub struct SnowflakeSequencer {
    worker: WorkerId,
    last_ms: u64,
    seq: u16,
}

impl SnowflakeSequencer {
    /// `floor_ms` is the highest ms a previous holder of this worker id may
    /// have used. Starting with that ms already exhausted means the first id
    /// is issued only once `now_ms > floor_ms`.
    pub fn new(worker: WorkerId, floor_ms: u64) -> Self {
        Self {
            worker,
            last_ms: floor_ms,
            seq: MAX_SEQUENCE,
        }
    }

    pub fn last_ms(&self) -> u64 {
        self.last_ms
    }

    pub fn next(&mut self, now_ms: u64) -> Result<SnowflakeId, NextIdError> {
        if now_ms < self.last_ms {
            return Err(NextIdError::ClockBehind {
                now_ms,
                last_ms: self.last_ms,
            });
        }
        if now_ms == self.last_ms {
            if self.seq == MAX_SEQUENCE {
                return Err(NextIdError::SequenceExhausted { ms: now_ms });
            }
            self.seq += 1;
        } else {
            if now_ms > MAX_TIMESTAMP_MS {
                return Err(NextIdError::TimestampOverflow);
            }
            self.last_ms = now_ms;
            self.seq = 0;
        }
        Ok(SnowflakeId::from_parts(self.last_ms, self.worker, self.seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worker(raw: u16) -> WorkerId {
        WorkerId::try_from(raw).unwrap()
    }

    #[test]
    fn clock_regression_is_rejected_then_recovers() {
        let mut seq = SnowflakeSequencer::new(worker(1), 0);
        let first = seq.next(100).unwrap();

        assert_eq!(
            seq.next(99),
            Err(NextIdError::ClockBehind {
                now_ms: 99,
                last_ms: 100
            })
        );
        assert_eq!(seq.last_ms(), 100, "a rejected call leaves state unchanged");

        let after = seq.next(101).unwrap();
        assert!(after > first);
        assert_eq!(after.timestamp_ms(), 101);
        assert_eq!(after.sequence(), 0);
    }

    #[test]
    fn sequence_exhausts_after_4096_ids_in_one_ms_then_recovers() {
        let mut seq = SnowflakeSequencer::new(worker(1), 0);
        let ids: Vec<_> = (0..4096).map(|_| seq.next(50).unwrap()).collect();
        assert_eq!(ids.last().unwrap().sequence(), MAX_SEQUENCE);

        assert_eq!(seq.next(50), Err(NextIdError::SequenceExhausted { ms: 50 }));
        assert_eq!(
            seq.next(50),
            Err(NextIdError::SequenceExhausted { ms: 50 }),
            "exhaustion does not advance state"
        );

        let next = seq.next(51).unwrap();
        assert_eq!((next.timestamp_ms(), next.sequence()), (51, 0));
        assert!(next > *ids.last().unwrap());
    }

    #[test]
    fn ids_strictly_increase() {
        let mut seq = SnowflakeSequencer::new(worker(7), 0);
        let mut prev = None;
        for now in [1, 1, 1, 2, 5, 5, 6] {
            let id = seq.next(now).unwrap();
            if let Some(prev) = prev {
                assert!(id > prev, "{id} should be greater than {prev}");
            }
            prev = Some(id);
        }
    }

    #[test]
    fn floor_ms_is_never_reused() {
        let mut seq = SnowflakeSequencer::new(worker(3), 1_000);
        assert!(matches!(
            seq.next(999),
            Err(NextIdError::ClockBehind { .. })
        ));
        assert_eq!(
            seq.next(1_000),
            Err(NextIdError::SequenceExhausted { ms: 1_000 })
        );
        assert_eq!(seq.next(1_001).unwrap().timestamp_ms(), 1_001);
    }

    #[test]
    fn timestamp_beyond_41_bits_overflows() {
        let mut seq = SnowflakeSequencer::new(worker(0), MAX_TIMESTAMP_MS - 1);
        let last = seq.next(MAX_TIMESTAMP_MS).unwrap();
        assert!(last.as_i64() > 0, "the sign bit stays clear");
        assert_eq!(
            seq.next(MAX_TIMESTAMP_MS + 1),
            Err(NextIdError::TimestampOverflow)
        );
    }

    #[test]
    fn bit_layout_round_trips() {
        let mut seq = SnowflakeSequencer::new(worker(MAX_WORKER), 0);
        seq.next(123_456).unwrap();
        let id = seq.next(123_456).unwrap();

        assert_eq!(id.timestamp_ms(), 123_456);
        assert_eq!(id.worker(), worker(MAX_WORKER));
        assert_eq!(id.sequence(), 1);
        assert_eq!(id.as_i64(), (123_456 << 22) | (1023 << 12) | 1);
        assert_eq!(SnowflakeId::try_from(id.as_i64()), Ok(id));
        assert_eq!(id.to_string().parse::<SnowflakeId>(), Ok(id));
    }

    #[test]
    fn worker_id_range() {
        assert_eq!(WorkerId::try_from(0).map(WorkerId::get), Ok(0));
        assert_eq!(WorkerId::try_from(1023).map(WorkerId::get), Ok(1023));
        assert_eq!(WorkerId::try_from(1024), Err(InvalidWorkerId(1024)));
    }

    #[test]
    fn parsing_accepts_only_non_negative_decimal() {
        assert_eq!(SnowflakeId::try_from(-1), Err(InvalidSnowflakeId));
        assert_eq!("42".parse::<SnowflakeId>().map(SnowflakeId::as_i64), Ok(42));
        for bad in ["", "-1", "+1", " 1", "1.0", "0x1", "9223372036854775808"] {
            assert!(
                bad.parse::<SnowflakeId>().is_err(),
                "{bad:?} should be rejected"
            );
        }
    }
}
