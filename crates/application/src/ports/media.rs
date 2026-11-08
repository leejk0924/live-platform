//! Where a broadcast's media goes and who may send it. The media server
//! behind these ports is an adapter detail.

use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use domain::{
    auth::UserId,
    broadcast::{BroadcastId, BroadcastStatus},
    secret::Secret,
};

use super::PortError;

/// The media server's name for one broadcast's stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamPath(String);

impl StreamPath {
    pub fn new(raw: String) -> Self {
        Self(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestEndpoint {
    pub path: StreamPath,
    /// Where the publisher sends its WHIP offer.
    pub whip_url: String,
}

/// Picks the media node and stream path for a broadcast.
pub trait IngestRouter: Send + Sync {
    fn route(&self, broadcast: BroadcastId) -> IngestEndpoint;

    /// The broadcast `path` belongs to: the inverse of [`Self::route`], so
    /// only a path `route` would produce maps to a broadcast.
    fn broadcast_of(&self, path: &StreamPath) -> Option<BroadcastId>;
}

/// What a publisher presents to the media server. An enum so a media backend
/// with its own credential shape can be added without changing callers.
#[derive(Debug)]
pub enum PublishCredential {
    Bearer {
        token: Secret<String>,
        expires_in: Duration,
    },
}

/// Issues credentials that let one user publish to one path and nothing else.
#[async_trait]
pub trait PublishAuthorizer: Send + Sync {
    async fn authorize(
        &self,
        publisher: UserId,
        path: &StreamPath,
    ) -> Result<PublishCredential, PortError>;
}

/// Disconnects whoever is publishing to a path.
#[async_trait]
pub trait StreamTerminator: Send + Sync {
    /// Succeeds when nobody is publishing.
    async fn kick(&self, path: &StreamPath) -> Result<(), PortError>;
}

/// A publish credential's unique id (the JWT `jti`). Not secret: knowing it
/// does not let anyone publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketId(String);

impl TicketId {
    pub fn new(raw: String) -> Self {
        Self(raw)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a valid publish credential says: who may publish where, until when.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishTicket {
    pub id: TicketId,
    pub publisher: UserId,
    pub path: StreamPath,
    pub expires_at: SystemTime,
}

/// Checks the credentials the media server forwards when it asks whether to
/// let someone in. Signature and expiry only; state is the ledger's job.
pub trait MediaCredentialVerifier: Send + Sync {
    /// `None` unless `token` is an unexpired publish credential this service
    /// issued.
    fn publish_ticket(&self, token: &Secret<String>) -> Option<PublishTicket>;

    /// Whether `token` is an unexpired credential this service issued for its
    /// own calls to the media server's control interface.
    fn is_control_credential(&self, token: &Secret<String>) -> bool;
}

/// Why a ticket was not redeemed. Deleted broadcasts count as
/// [`RedeemRejection::NotFound`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedeemRejection {
    NotFound,
    NotOwner,
    /// The ticket was redeemed before.
    AlreadyUsed,
    /// Not in [`BroadcastStatus::ACTIVE`] when the rejection was read.
    InStatus(BroadcastStatus),
}

/// Remembers which publish tickets were used, so each lets in one publisher.
#[async_trait]
pub trait PublishTicketLedger: Send + Sync {
    /// Records the ticket's first use for `broadcast`, in the same statement
    /// that checks the ticket's publisher owns the broadcast and that it is
    /// active. A rejected ticket stays unused.
    async fn redeem(
        &self,
        ticket: &PublishTicket,
        broadcast: BroadcastId,
    ) -> Result<Result<(), RedeemRejection>, PortError>;
}
