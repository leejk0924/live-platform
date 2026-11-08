use async_trait::async_trait;
use domain::{
    auth::{UserId, Username},
    broadcast::{
        Broadcast, BroadcastDescription, BroadcastId, BroadcastStatus, BroadcastTitle, Version,
    },
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

/// Why a status transition matched no row. Deleted broadcasts count as
/// [`TransitionRejection::NotFound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionRejection {
    NotFound,
    NotOwner,
    /// The broadcast was in this status when the rejection was read, which
    /// may already differ from the status the write saw.
    InStatus(BroadcastStatus),
    /// Going live would give the owner a second active broadcast.
    OwnerAlreadyActive,
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

    /// Applies [`domain::broadcast::BroadcastEvent::Start`] if `owner` owns the broadcast, and
    /// bumps the version.
    async fn start(
        &self,
        id: BroadcastId,
        owner: UserId,
    ) -> Result<Result<Broadcast, TransitionRejection>, PortError>;

    /// Applies [`domain::broadcast::BroadcastEvent::Stop`] if `owner` owns the broadcast, and
    /// bumps the version.
    async fn stop(
        &self,
        id: BroadcastId,
        owner: UserId,
    ) -> Result<Result<Broadcast, TransitionRejection>, PortError>;
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
