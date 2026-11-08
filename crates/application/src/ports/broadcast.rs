use std::time::Duration;

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

/// Status changes that follow the publisher's connection rather than the
/// owner, so no owner is checked. Each is one conditional write; one that
/// does not apply in the current status changes nothing.
#[async_trait]
pub trait PublisherPresence: Send + Sync {
    /// Applies [`domain::broadcast::BroadcastEvent::Interrupt`], starting the
    /// grace period. Whether the status changed.
    async fn interrupt(&self, id: BroadcastId) -> Result<bool, PortError>;

    /// Applies [`domain::broadcast::BroadcastEvent::Resume`]. Whether the
    /// status changed.
    async fn resume(&self, id: BroadcastId) -> Result<bool, PortError>;

    /// Whether the broadcast is stored, not deleted, and in
    /// [`BroadcastStatus::ACTIVE`].
    async fn is_active(&self, id: BroadcastId) -> Result<bool, PortError>;
}

/// Where the reaper's passes run. One pass at a time across every API
/// instance.
#[async_trait]
pub trait ReaperStore: Send + Sync {
    /// Starts a pass, or `None` while another instance is in one.
    async fn begin(&self) -> Result<Option<Box<dyn ReapPass>>, PortError>;
}

/// One reaper pass. Its writes take effect together at [`ReapPass::finish`];
/// dropping it unfinished undoes them. Times are the store's clock, and
/// "now" is when the pass began.
#[async_trait]
pub trait ReapPass: Send {
    /// Applies [`domain::broadcast::BroadcastEvent::Resume`] to the
    /// interrupted broadcasts in `on_air` that were interrupted before the
    /// pass began, and [`domain::broadcast::BroadcastEvent::Interrupt`] to the
    /// live broadcasts not in `on_air` that went live longer than `grace`
    /// ago, so one that just started or resumed has time to connect.
    async fn reconcile(
        &mut self,
        on_air: &[BroadcastId],
        grace: Duration,
    ) -> Result<Reconciled, PortError>;

    /// The broadcasts in `on_air` that are not active: ended, deleted, or
    /// never stored. Nobody may publish to them.
    async fn not_active(&mut self, on_air: &[BroadcastId]) -> Result<Vec<BroadcastId>, PortError>;

    /// Applies [`domain::broadcast::BroadcastEvent::Expire`] to every
    /// broadcast interrupted longer than `grace` ago.
    async fn expire(&mut self, grace: Duration) -> Result<Vec<BroadcastId>, PortError>;

    /// Deletes the records of publish tickets that can no longer be
    /// presented. Returns how many.
    async fn forget_spent_tickets(&mut self) -> Result<u64, PortError>;

    /// Deletes every session past its idle or absolute timeout. Returns how
    /// many.
    async fn forget_expired_sessions(&mut self) -> Result<u64, PortError>;

    async fn finish(self: Box<Self>) -> Result<(), PortError>;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciled {
    pub interrupted: Vec<BroadcastId>,
    pub resumed: Vec<BroadcastId>,
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
