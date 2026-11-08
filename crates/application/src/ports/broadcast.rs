use async_trait::async_trait;
use domain::{
    auth::{UserId, Username},
    broadcast::{Broadcast, BroadcastDescription, BroadcastId, BroadcastTitle, Version},
};

use super::PortError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewBroadcast {
    pub id: BroadcastId,
    pub owner_id: UserId,
    pub title: BroadcastTitle,
    pub description: BroadcastDescription,
}

/// Why a conditional write matched no row. Deleted broadcasts count as
/// [`WriteRejection::NotFound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteRejection {
    NotFound,
    NotOwner,
    VersionMismatch,
    NotDeletable,
}

/// The write side. Each write checks ownership, version and status in the
/// same statement that changes the row, so no other request can slip in
/// between the check and the change.
#[async_trait]
pub trait BroadcastRepository: Send + Sync {
    /// Stores a new broadcast in `READY` at [`Version::FIRST`].
    async fn insert(&self, new: &NewBroadcast) -> Result<Broadcast, PortError>;

    /// Replaces title and description if `owner` owns the broadcast and its
    /// version is still `expected`, and bumps the version.
    async fn update(
        &self,
        id: BroadcastId,
        owner: UserId,
        expected: Version,
        title: &BroadcastTitle,
        description: &BroadcastDescription,
    ) -> Result<Result<Broadcast, WriteRejection>, PortError>;

    /// Soft-deletes the broadcast if `owner` owns it and its status is
    /// deletable.
    async fn soft_delete(
        &self,
        id: BroadcastId,
        owner: UserId,
    ) -> Result<Result<(), WriteRejection>, PortError>;
}

/// A broadcast as viewers see it, with its owner's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastView {
    pub broadcast: Broadcast,
    pub owner_name: Username,
}

/// The read side. Deleted broadcasts are never returned.
#[async_trait]
pub trait BroadcastQuery: Send + Sync {
    /// Newest first, only ids below `before` when given, at most `limit`.
    async fn list(
        &self,
        before: Option<BroadcastId>,
        limit: u32,
    ) -> Result<Vec<BroadcastView>, PortError>;

    async fn get(&self, id: BroadcastId) -> Result<Option<BroadcastView>, PortError>;
}
