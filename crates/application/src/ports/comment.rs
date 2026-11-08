use async_trait::async_trait;
use domain::{
    auth::{UserId, Username},
    broadcast::{BroadcastId, BroadcastStatus, Version},
    comment::{ChatPolicy, Comment, CommentBody, CommentId, Revision},
};

use super::PortError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewComment {
    pub id: CommentId,
    pub broadcast_id: BroadcastId,
    pub author_id: UserId,
    pub body: CommentBody,
}

/// A comment as followers see it: with its author's name and where its
/// latest change sits in the broadcast's change feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentView {
    pub comment: Comment,
    pub author_name: Username,
    pub revision: Revision,
}

/// Why a conditional write to a comment matched no row. Deleted comments,
/// and comments of deleted broadcasts, count as [`CommentWriteRejection::NotFound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentWriteRejection {
    NotFound,
    NotAuthor,
    VersionMismatch,
}

/// The write side. Each write checks the broadcast, the author and the
/// version in the same statement that changes the row, and gives the
/// comment a new [`Revision`].
#[async_trait]
pub trait CommentRepository: Send + Sync {
    /// Stores the comment at [`Version::FIRST`] if its broadcast exists and
    /// [`domain::comment::accepts_comments`]; `None` otherwise.
    async fn insert(&self, new: &NewComment) -> Result<Option<CommentView>, PortError>;

    /// Replaces the body if `author` wrote the comment and its version is
    /// still `expected`, and bumps the version.
    async fn edit(
        &self,
        broadcast: BroadcastId,
        id: CommentId,
        author: UserId,
        expected: Version,
        body: &CommentBody,
    ) -> Result<Result<CommentView, CommentWriteRejection>, PortError>;

    /// Soft-deletes the comment if `author` wrote it, and returns its
    /// tombstone.
    async fn soft_delete(
        &self,
        broadcast: BroadcastId,
        id: CommentId,
        author: UserId,
    ) -> Result<Result<CommentView, CommentWriteRejection>, PortError>;
}

/// The read side, always within one broadcast.
#[async_trait]
pub trait CommentQuery: Send + Sync {
    /// Comments not deleted, newest first, only ids below `before` when
    /// given, at most `limit`.
    async fn list(
        &self,
        broadcast: BroadcastId,
        before: Option<CommentId>,
        limit: u32,
    ) -> Result<Vec<CommentView>, PortError>;

    /// The comments changed after `after` (all of them when `None`), deleted
    /// ones included, by ascending revision, at most `limit`.
    async fn changes_since(
        &self,
        broadcast: BroadcastId,
        after: Option<Revision>,
        limit: u32,
    ) -> Result<Vec<CommentView>, PortError>;

    /// The newest revision of the broadcast's comments; `None` before the
    /// first comment.
    async fn latest_revision(&self, broadcast: BroadcastId) -> Result<Option<Revision>, PortError>;
}

/// What posting to a broadcast depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatTarget {
    pub owner: UserId,
    pub status: BroadcastStatus,
    pub policy: ChatPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyRejection {
    NotFound,
    NotOwner,
}

/// The broadcast side of the chat. Deleted broadcasts are never returned.
#[async_trait]
pub trait ChatPolicyStore: Send + Sync {
    async fn target(&self, broadcast: BroadcastId) -> Result<Option<ChatTarget>, PortError>;

    /// Replaces the policy if `owner` owns the broadcast. The broadcast's
    /// version stays as it is.
    async fn set(
        &self,
        broadcast: BroadcastId,
        owner: UserId,
        policy: ChatPolicy,
    ) -> Result<Result<(), PolicyRejection>, PortError>;
}
