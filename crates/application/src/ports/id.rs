use async_trait::async_trait;
use domain::id::{SnowflakeId, WorkerId};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IdGenerationError {
    #[error("worker id lease is no longer valid")]
    LeaseExpired,
    #[error("clock is behind the last issued id")]
    ClockBehind,
    #[error("snowflake timestamp space is exhausted")]
    TimestampOverflow,
}

/// Issues unique, time-ordered ids. Synchronous because issuing never waits
/// on I/O.
pub trait IdGenerator: Send + Sync {
    fn next_id(&self) -> Result<SnowflakeId, IdGenerationError>;
}

/// Opaque token proving which process holds a worker id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseHolder(String);

impl LeaseHolder {
    pub fn new(raw: String) -> Self {
        Self(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLease {
    pub worker: WorkerId,
    pub holder: LeaseHolder,
    /// Highest ms since the snowflake epoch any earlier holder may have used.
    pub floor_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeaseError {
    #[error("no free worker id")]
    Exhausted,
    #[error("worker id lease was lost")]
    Lost,
    #[error("lease store unavailable: {0}")]
    Unavailable(String),
}

/// Hands out time-limited exclusive worker ids so concurrent instances never
/// share one.
#[async_trait]
pub trait WorkerIdAllocator: Send + Sync {
    async fn acquire(&self) -> Result<WorkerLease, LeaseError>;

    /// Extends the lease and records `last_ms`, the highest ms this holder
    /// has used so far.
    async fn renew(&self, lease: &WorkerLease, last_ms: u64) -> Result<(), LeaseError>;

    /// Gives the worker id back. Calling it on an already released or lost
    /// lease succeeds without effect.
    async fn release(&self, lease: &WorkerLease, last_ms: u64) -> Result<(), LeaseError>;
}
