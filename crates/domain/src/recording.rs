//! Recordings: the finished segments of a broadcast's stream, kept for
//! replay after it ends. A broadcast may have several, one per publisher
//! connection and per segment-length cap.

use std::time::Duration;

use thiserror::Error;

use crate::{
    broadcast::BroadcastId,
    error::{DomainError, ErrorKind},
    id::SnowflakeId,
};

pub type RecordingId = SnowflakeId;

/// Recordings per list page when the client names no size, and the most it
/// may ask for.
pub const PAGE_SIZE_DEFAULT: u32 = 20;
pub const PAGE_SIZE_MAX: u32 = 50;

/// Copying a recording to storage is tried this many times before it is
/// given up as [`RecordingStatus::Failed`].
pub const MAX_STORE_ATTEMPTS: u32 = 5;
const STORE_RETRY_BASE: Duration = Duration::from_secs(30);

/// `PENDING -> READY`, or `PENDING -> FAILED` after [`MAX_STORE_ATTEMPTS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingStatus {
    /// Registered, still waiting in the spool to be copied to storage.
    Pending,
    /// In storage; it can be replayed.
    Ready,
    /// Every attempt to copy it to storage failed.
    Failed,
}

impl RecordingStatus {
    pub const ALL: [Self; 3] = [Self::Pending, Self::Ready, Self::Failed];

    /// The only status storing or giving up may leave. Adapters bind this set
    /// into their conditional writes instead of restating it.
    pub const STORABLE: [Self; 1] = [Self::Pending];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Ready => "READY",
            Self::Failed => "FAILED",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == raw)
    }
}

/// Where a recording lives in storage. Built only from ids, so it never
/// carries anything a client or the media server wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingKey(String);

impl RecordingKey {
    pub fn new(broadcast: BroadcastId, recording: RecordingId) -> Self {
        Self(format!("recordings/{broadcast}/{recording}.mp4"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What to do after the `attempts`-th try to store a recording failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreRetry {
    /// Try again once this much time has passed.
    After(Duration),
    GiveUp,
}

impl StoreRetry {
    /// Doubles from 30 seconds per failed attempt and gives up at
    /// [`MAX_STORE_ATTEMPTS`], about eight minutes after the first failure.
    pub fn after_attempt(attempts: u32) -> Self {
        if attempts >= MAX_STORE_ATTEMPTS {
            return Self::GiveUp;
        }
        Self::After(STORE_RETRY_BASE * 2u32.pow(attempts.saturating_sub(1)))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum RecordingError {
    #[error("page size must be between 1 and {PAGE_SIZE_MAX}")]
    InvalidPageSize,
    #[error("recording not found")]
    NotFound,
    #[error("the recording is not ready for replay yet")]
    NotReady,
}

impl DomainError for RecordingError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidPageSize => ErrorKind::InvalidInput,
            Self::NotFound => ErrorKind::NotFound,
            Self::NotReady => ErrorKind::Conflict,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::InvalidPageSize => "INVALID_PAGE_SIZE",
            Self::NotFound => "RECORDING_NOT_FOUND",
            Self::NotReady => "RECORDING_NOT_READY",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    #[test]
    fn status_round_trips_and_only_pending_is_storable() {
        for status in RecordingStatus::ALL {
            assert_eq!(RecordingStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(RecordingStatus::parse("ready"), None);
        assert_eq!(RecordingStatus::STORABLE, [RecordingStatus::Pending]);
    }

    #[test]
    fn key_is_built_from_the_ids_only() {
        assert_eq!(
            RecordingKey::new(id(42), id(7)).as_str(),
            "recordings/42/7.mp4"
        );
    }

    #[test]
    fn retries_back_off_doubling_then_give_up_at_the_limit() {
        let secs = |attempts| match StoreRetry::after_attempt(attempts) {
            StoreRetry::After(wait) => Some(wait.as_secs()),
            StoreRetry::GiveUp => None,
        };
        assert_eq!(
            (1..=MAX_STORE_ATTEMPTS).map(secs).collect::<Vec<_>>(),
            [Some(30), Some(60), Some(120), Some(240), None]
        );
        assert_eq!(StoreRetry::after_attempt(u32::MAX), StoreRetry::GiveUp);
    }

    #[test]
    fn errors_have_stable_codes_and_kinds() {
        assert_eq!(RecordingError::NotFound.kind(), ErrorKind::NotFound);
        assert_eq!(RecordingError::NotFound.code(), "RECORDING_NOT_FOUND");
        assert_eq!(RecordingError::NotReady.kind(), ErrorKind::Conflict);
        assert_eq!(RecordingError::NotReady.code(), "RECORDING_NOT_READY");
        assert_eq!(
            RecordingError::InvalidPageSize.kind(),
            ErrorKind::InvalidInput
        );
    }
}
