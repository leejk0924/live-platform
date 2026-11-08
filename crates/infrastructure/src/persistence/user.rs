use application::ports::{
    PortError,
    auth::{Inserted, UserRepository},
};
use async_trait::async_trait;
use domain::{
    auth::{Credential, PasswordHash, User, UserId, Username},
    id::SnowflakeId,
};
use sqlx::PgPool;

pub struct PgUserRepository {
    pool: PgPool,
}

impl PgUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn port_error(err: sqlx::Error) -> PortError {
    PortError(err.to_string())
}

type UserRow = (i64, String, Option<String>, bool);

/// The table's CHECK makes `is_demo` and `password_hash` mutually exclusive,
/// so every valid row maps to exactly one [`Credential`].
fn to_user((id, username, password_hash, is_demo): UserRow) -> Result<User, PortError> {
    let id = SnowflakeId::try_from(id).map_err(|_| PortError(format!("invalid user id {id}")))?;
    let username = Username::parse(&username)
        .map_err(|_| PortError(format!("invalid stored username for user {id}")))?;
    let credential = match (password_hash, is_demo) {
        (Some(hash), false) => Credential::Password(PasswordHash::new(hash)),
        (None, true) => Credential::DemoOnly,
        _ => return Err(PortError(format!("user {id} has no single credential"))),
    };
    Ok(User {
        id,
        username,
        credential,
    })
}

#[async_trait]
impl UserRepository for PgUserRepository {
    async fn insert(&self, user: &User) -> Result<Inserted, PortError> {
        let (password_hash, is_demo) = match &user.credential {
            Credential::Password(hash) => (Some(hash.as_str()), false),
            Credential::DemoOnly => (None, true),
        };
        let result = sqlx::query(
            "INSERT INTO users (id, username, password_hash, is_demo) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (username) DO NOTHING",
        )
        .bind(user.id.as_i64())
        .bind(user.username.as_str())
        .bind(password_hash)
        .bind(is_demo)
        .execute(&self.pool)
        .await
        .map_err(port_error)?;

        Ok(if result.rows_affected() == 1 {
            Inserted::Created
        } else {
            Inserted::UsernameTaken
        })
    }

    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, PortError> {
        sqlx::query_as::<_, UserRow>(
            "SELECT id, username, password_hash, is_demo FROM users WHERE username = $1",
        )
        .bind(username.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?
        .map(to_user)
        .transpose()
    }

    async fn find_by_id(&self, id: UserId) -> Result<Option<User>, PortError> {
        sqlx::query_as::<_, UserRow>(
            "SELECT id, username, password_hash, is_demo FROM users WHERE id = $1",
        )
        .bind(id.as_i64())
        .fetch_optional(&self.pool)
        .await
        .map_err(port_error)?
        .map(to_user)
        .transpose()
    }
}
