pub mod auth;
pub mod broadcast;
pub mod comment;
pub mod donation;
pub mod event_bus;
pub mod id;
pub mod media;
pub mod payment;
pub mod rate_limit;
pub mod recording;

use thiserror::Error;

/// An adapter could not do its job (database down, hashing failed). The
/// message is for logs only.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct PortError(pub String);
