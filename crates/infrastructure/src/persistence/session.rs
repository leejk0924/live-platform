use std::time::Duration;

use application::ports::auth::{PortError, SessionStore};
use async_trait::async_trait;
use base64ct::{Base64UrlUnpadded, Encoding};
use domain::{
    auth::{SessionToken, UserId},
    id::SnowflakeId,
    secret::Secret,
};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

pub const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);
pub const ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// `resolve` refreshes `last_seen_at` only once it is this old, so a busy
/// session costs a write per minute instead of per request. The idle timeout
/// may therefore fire up to this much early.
const SLIDE_INTERVAL: Duration = Duration::from_secs(60);

/// Sessions in the `sessions` table, keyed by SHA-256 of the token so a
/// leaked table cannot be replayed. Expiry is judged by the database's
/// `now()` only, because instances' clocks may disagree.
pub struct PgSessionStore {
    pool: PgPool,
}

impl PgSessionStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

/// 256 bits from the OS RNG, base64url without padding.
fn new_token() -> Result<SessionToken, PortError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|err| PortError(format!("os rng failed: {err}")))?;
    Ok(SessionToken::new(Secret::new(
        Base64UrlUnpadded::encode_string(&bytes),
    )))
}

fn token_hash(token: &SessionToken) -> [u8; 32] {
    Sha256::digest(token.expose().as_bytes()).into()
}

#[async_trait]
impl SessionStore for PgSessionStore {
    async fn create(&self, user: UserId) -> Result<SessionToken, PortError> {
        let token = new_token()?;
        sqlx::query(
            "WITH expired AS ( \
                 DELETE FROM sessions WHERE user_id = $2 \
                   AND (expires_at <= now() OR last_seen_at <= now() - make_interval(secs => $3)) \
             ) \
             INSERT INTO sessions (token_hash, user_id, expires_at) \
             VALUES ($1, $2, now() + make_interval(secs => $4))",
        )
        .bind(token_hash(&token).as_slice())
        .bind(user.as_i64())
        .bind(IDLE_TIMEOUT.as_secs_f64())
        .bind(ABSOLUTE_TIMEOUT.as_secs_f64())
        .execute(&self.pool)
        .await
        .map_err(port_error)?;
        Ok(token)
    }

    async fn resolve(&self, token: &SessionToken) -> Result<Option<UserId>, PortError> {
        let user_id: Option<i64> = sqlx::query_scalar(
            "WITH live AS ( \
                 SELECT token_hash, user_id, last_seen_at FROM sessions \
                 WHERE token_hash = $1 \
                   AND last_seen_at > now() - make_interval(secs => $2) \
                   AND expires_at > now() \
             ), slide AS ( \
                 UPDATE sessions s SET last_seen_at = now() FROM live \
                 WHERE s.token_hash = live.token_hash \
                   AND live.last_seen_at <= now() - make_interval(secs => $3) \
             ) \
             SELECT user_id FROM live",
        )
        .bind(token_hash(token).as_slice())
        .bind(IDLE_TIMEOUT.as_secs_f64())
        .bind(SLIDE_INTERVAL.as_secs_f64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?;

        user_id
            .map(|id| {
                SnowflakeId::try_from(id).map_err(|_| PortError(format!("invalid user id {id}")))
            })
            .transpose()
    }

    async fn revoke(&self, token: &SessionToken) -> Result<(), PortError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash(token).as_slice())
            .execute(&self.pool)
            .await
            .map_err(port_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_256_bit_base64url_and_unique() {
        let a = new_token().unwrap();
        let b = new_token().unwrap();
        assert_eq!(a.expose().len(), 43);
        assert!(
            a.expose()
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
        assert_eq!(Base64UrlUnpadded::decode_vec(a.expose()).unwrap().len(), 32);
        assert_ne!(a.expose(), b.expose());
    }
}
