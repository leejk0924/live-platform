//! Where recordings wait, where they are kept, and how viewers reach them.
//! The media server, the storage service and the CDN behind these ports are
//! adapter details.

use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use domain::{
    auth::Username,
    broadcast::{BroadcastId, BroadcastTitle},
    recording::{RecordingId, RecordingKey, RecordingStatus, StoreRetry},
};

use super::{PortError, media::StreamPath};

/// A finished segment file in the spool, named by its path relative to the
/// spool root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpooledSegment(String);

impl SpooledSegment {
    pub fn new(raw: String) -> Self {
        Self(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where the media server leaves finished segments until they are stored.
#[async_trait]
pub trait SegmentSpool: Send + Sync {
    /// The segment the media server `reported` for `path`, or `None` unless
    /// it names a file directly in that path's spool directory.
    fn segment(&self, path: &StreamPath, reported: &str) -> Option<SpooledSegment>;

    /// Removes the segment. A segment that is already gone is not an error.
    async fn discard(&self, segment: &SpooledSegment) -> Result<(), PortError>;
}

/// Keeps recordings for replay.
#[async_trait]
pub trait RecordingStorage: Send + Sync {
    /// Copies `segment` to `key` and returns its size in bytes. Putting the
    /// same key again overwrites it, so a retry is safe.
    async fn put(&self, segment: &SpooledSegment, key: &RecordingKey) -> Result<u64, PortError>;
}

/// Issues short-lived URLs that let anyone holding them fetch one stored
/// recording.
pub trait UrlSigner: Send + Sync {
    fn sign(&self, recording: RecordingId, key: &RecordingKey, expires_at: SystemTime) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRecording {
    pub id: RecordingId,
    pub broadcast: BroadcastId,
    pub source: SpooledSegment,
    pub key: RecordingKey,
    pub duration: Duration,
}

/// A pending recording handed to one storer until its lease runs out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedRecording {
    pub id: RecordingId,
    pub broadcast: BroadcastId,
    pub source: SpooledSegment,
    pub key: RecordingKey,
    /// Including this one.
    pub attempts: u32,
}

/// The write side. Each status change is one conditional write that applies
/// only to a [`RecordingStatus::STORABLE`] recording.
#[async_trait]
pub trait RecordingRepository: Send + Sync {
    /// Stores `new` as pending, unless its broadcast is unknown or deleted or
    /// its source was registered before. Whether it was stored.
    async fn register(&self, new: &NewRecording) -> Result<bool, PortError>;

    /// Takes one pending recording that is due, counts the attempt and holds
    /// it for `lease`, after which another storer may take it again.
    async fn claim(&self, lease: Duration) -> Result<Option<ClaimedRecording>, PortError>;

    /// Marks it ready with its stored size. Whether it was still pending.
    async fn mark_stored(&self, id: RecordingId, size_bytes: u64) -> Result<bool, PortError>;

    /// Makes it due again after the wait `retry` names, or marks it failed.
    /// Whether it was still pending.
    async fn release_failed(&self, id: RecordingId, retry: StoreRetry) -> Result<bool, PortError>;
}

/// A recording as viewers see it, with its broadcast's title and owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingView {
    pub id: RecordingId,
    pub broadcast_id: BroadcastId,
    pub broadcast_title: BroadcastTitle,
    pub owner_name: Username,
    pub status: RecordingStatus,
    pub duration: Duration,
    /// Known once it is stored.
    pub size_bytes: Option<u64>,
    /// When its segment finished.
    pub recorded_at: SystemTime,
    pub key: RecordingKey,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingFilter {
    All,
    Broadcast(BroadcastId),
}

/// The read side. Recordings of a deleted broadcast are never returned, so
/// no new replay URL is issued for them. Local file URLs are re-checked on
/// every request, so deletion cuts them off at once; CDN-signed URLs stop
/// being issued at once and expire within
/// [`REPLAY_URL_TTL`](crate::recording::REPLAY_URL_TTL).
#[async_trait]
pub trait RecordingQuery: Send + Sync {
    /// Newest first, only those `filter` admits, only ids below `before`
    /// when given, at most `limit`.
    async fn list(
        &self,
        filter: RecordingFilter,
        before: Option<RecordingId>,
        limit: u32,
    ) -> Result<Vec<RecordingView>, PortError>;

    async fn get(&self, id: RecordingId) -> Result<Option<RecordingView>, PortError>;
}
