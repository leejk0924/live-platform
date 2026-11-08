//! Where a broadcast's media goes and who may send it. The media server
//! behind these ports is an adapter detail.

use std::time::Duration;

use async_trait::async_trait;
use domain::{auth::UserId, broadcast::BroadcastId, secret::Secret};

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
