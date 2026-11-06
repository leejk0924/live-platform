use async_trait::async_trait;
use domain::auth::{Password, PasswordHash, SessionToken, User, UserId, Username};

use super::PortError;

#[async_trait]
pub trait PasswordHasher: Send + Sync {
    async fn hash(&self, password: &Password) -> Result<PasswordHash, PortError>;

    /// With `None`, verifies against a fixed dummy hash and returns false, so
    /// a missing account costs as much time as a wrong password.
    async fn verify(
        &self,
        password: &Password,
        hash: Option<&PasswordHash>,
    ) -> Result<bool, PortError>;
}

pub trait BreachedPasswords: Send + Sync {
    fn contains(&self, password: &Password) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Inserted {
    Created,
    UsernameTaken,
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Never overwrites: an existing username yields [`Inserted::UsernameTaken`].
    async fn insert(&self, user: &User) -> Result<Inserted, PortError>;

    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, PortError>;

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, PortError>;
}

/// Server-side sessions. Token generation and storage format belong to the
/// adapter.
#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create(&self, user: UserId) -> Result<SessionToken, PortError>;

    /// Returns the owner of a live session and marks it as just used. Expired,
    /// revoked and unknown tokens all resolve to `None`.
    async fn resolve(&self, token: &SessionToken) -> Result<Option<UserId>, PortError>;

    /// Succeeds for an unknown or already revoked token.
    async fn revoke(&self, token: &SessionToken) -> Result<(), PortError>;
}
