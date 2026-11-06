pub mod auth;
pub mod id;

use thiserror::Error;

/// An adapter could not do its job (database down, hashing failed). The
/// message is for logs only.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct PortError(pub String);
