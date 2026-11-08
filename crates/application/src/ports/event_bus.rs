//! Live events of one broadcast, delivered to every API instance that has a
//! follower of it. Comment changes are announced by
//! [`super::comment::CommentRepository`] with the write itself, so they
//! arrive in revision order; other events go through [`EventBus::publish`].
//! Delivery is still best effort: a follower learns that it may have missed
//! events and catches up from the store, which holds the truth.

use std::sync::Arc;

use async_trait::async_trait;
use domain::{broadcast::BroadcastId, comment::ChatPolicy};

use super::comment::CommentView;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealtimeEvent {
    pub broadcast: BroadcastId,
    /// Sent to followers at once instead of with the next batch, such as the
    /// broadcaster's own comments.
    pub priority: bool,
    pub kind: EventKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventKind {
    /// A comment was posted, edited or deleted.
    Comment(CommentView),
    ChatPolicy(ChatPolicy),
}

#[derive(Debug, Clone)]
pub enum Delivery {
    Event(Arc<RealtimeEvent>),
    /// Events may have been lost (the follower fell behind, or the bus lost
    /// its upstream for a while); catch up from the store.
    Missed,
}

#[async_trait]
pub trait EventBus: Send + Sync {
    /// Announces a change that is not a comment write, such as a new chat
    /// policy. Never fails the caller: the change it announces is already
    /// stored, and a follower that misses it catches up. Failures are the
    /// adapter's to log.
    async fn publish(&self, event: RealtimeEvent);

    /// Events of `broadcast` published from now until the subscription is
    /// dropped.
    fn subscribe(&self, broadcast: BroadcastId) -> Box<dyn EventSubscription>;
}

#[async_trait]
pub trait EventSubscription: Send {
    /// The next delivery, or `None` once the bus has shut down. Dropping the
    /// future before it completes loses nothing.
    async fn next(&mut self) -> Option<Delivery>;
}
