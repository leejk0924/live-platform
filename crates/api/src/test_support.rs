use std::{
    io,
    sync::{Arc, Mutex},
};

use application::ports::{
    auth::{Inserted, PasswordHasher, PortError, SessionStore, UserRepository},
    id::{IdGenerationError, IdGenerator},
};
use async_trait::async_trait;
use domain::{
    auth::{Password, PasswordHash, SessionToken, User, UserId, Username},
    id::SnowflakeId,
};
use infrastructure::auth::BreachedPasswordList;
use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;

use crate::{AppState, auth::AuthPorts, config::Origin};

/// Stands in for every port a test does not exercise; each call fails.
struct Unavailable;

fn unavailable<T>() -> Result<T, PortError> {
    Err(PortError("not available in this test".into()))
}

impl IdGenerator for Unavailable {
    fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
        Err(IdGenerationError::LeaseExpired)
    }
}

#[async_trait]
impl UserRepository for Unavailable {
    async fn insert(&self, _: &User) -> Result<Inserted, PortError> {
        unavailable()
    }

    async fn find_by_username(&self, _: &Username) -> Result<Option<User>, PortError> {
        unavailable()
    }

    async fn find_by_id(&self, _: UserId) -> Result<Option<User>, PortError> {
        unavailable()
    }
}

#[async_trait]
impl SessionStore for Unavailable {
    async fn create(&self, _: UserId) -> Result<SessionToken, PortError> {
        unavailable()
    }

    async fn resolve(&self, _: &SessionToken) -> Result<Option<UserId>, PortError> {
        unavailable()
    }

    async fn revoke(&self, _: &SessionToken) -> Result<(), PortError> {
        unavailable()
    }
}

#[async_trait]
impl PasswordHasher for Unavailable {
    async fn hash(&self, _: &Password) -> Result<PasswordHash, PortError> {
        unavailable()
    }

    async fn verify(&self, _: &Password, _: Option<&PasswordHash>) -> Result<bool, PortError> {
        unavailable()
    }
}

pub const TEST_ORIGIN: &str = "https://app.test";

pub fn test_state() -> AppState {
    let ports = AuthPorts {
        ids: Arc::new(Unavailable),
        users: Arc::new(Unavailable),
        sessions: Arc::new(Unavailable),
        hasher: Arc::new(Unavailable),
        breached: Arc::new(BreachedPasswordList::load()),
    };
    AppState::new(ports, vec![Origin::parse(TEST_ORIGIN).unwrap()], false)
}

/// Captures JSON log lines so tests can assert what reached the logs.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub fn subscriber(&self) -> impl Subscriber + Send + Sync + 'static {
        tracing_subscriber::fmt()
            .json()
            .with_writer(self.clone())
            .finish()
    }

    pub fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
