//! Sign-up, log-in, log-out and session checks. Every rule about who may get
//! a session lives here; handlers only translate HTTP.

use std::sync::Arc;

use domain::{
    auth::{AuthError, Credential, DemoAccount, Password, SessionToken, User, UserId, Username},
    secret::Secret,
};
use thiserror::Error;

use crate::ports::{
    PortError,
    auth::{BreachedPasswords, Inserted, PasswordHasher, SessionStore, UserRepository},
    id::{IdGenerationError, IdGenerator},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Rejected(#[from] AuthError),
    #[error(transparent)]
    Internal(#[from] PortError),
}

impl From<IdGenerationError> for Error {
    fn from(err: IdGenerationError) -> Self {
        Self::Internal(PortError(err.to_string()))
    }
}

/// A fresh session and the account it belongs to.
#[derive(Debug)]
pub struct SignedIn {
    pub user: User,
    pub token: SessionToken,
}

/// Every sign-in issues a new token and drops the one the client came with,
/// so a token planted before sign-in never becomes authenticated.
async fn sign_in(
    sessions: &dyn SessionStore,
    user: User,
    previous: Option<&SessionToken>,
) -> Result<SignedIn, Error> {
    if let Some(previous) = previous {
        sessions.revoke(previous).await?;
    }
    let token = sessions.create(user.id).await?;
    Ok(SignedIn { user, token })
}

pub struct SignUp {
    pub ids: Arc<dyn IdGenerator>,
    pub users: Arc<dyn UserRepository>,
    pub hasher: Arc<dyn PasswordHasher>,
    pub breached: Arc<dyn BreachedPasswords>,
    pub sessions: Arc<dyn SessionStore>,
}

impl SignUp {
    pub async fn run(
        &self,
        username: &str,
        password: Secret<String>,
        previous: Option<&SessionToken>,
    ) -> Result<SignedIn, Error> {
        let username = Username::parse(username)?;
        if username.is_reserved() {
            return Err(AuthError::InvalidUsername.into());
        }
        let password = Password::parse(password)?;
        if self.breached.contains(&password) {
            return Err(AuthError::PasswordBreached.into());
        }

        let user = User {
            id: self.ids.next_id()?,
            username,
            credential: Credential::Password(self.hasher.hash(&password).await?),
        };
        match self.users.insert(&user).await? {
            Inserted::Created => sign_in(self.sessions.as_ref(), user, previous).await,
            Inserted::UsernameTaken => Err(AuthError::UsernameTaken.into()),
        }
    }
}

pub struct LogIn {
    pub users: Arc<dyn UserRepository>,
    pub hasher: Arc<dyn PasswordHasher>,
    pub sessions: Arc<dyn SessionStore>,
}

impl LogIn {
    /// Unknown users, wrong passwords and demo-only accounts all fail with
    /// [`AuthError::InvalidCredentials`] after one hash verification, so
    /// neither the answer nor its timing tells them apart.
    pub async fn run(
        &self,
        username: &str,
        password: Secret<String>,
        previous: Option<&SessionToken>,
    ) -> Result<SignedIn, Error> {
        // Input no account could have is not worth a hash: its rejection
        // depends only on the input, not on which accounts exist.
        let (Ok(username), Ok(password)) = (Username::parse(username), Password::parse(password))
        else {
            return Err(AuthError::InvalidCredentials.into());
        };

        let user = self.users.find_by_username(&username).await?;
        let hash = match &user {
            Some(User {
                credential: Credential::Password(hash),
                ..
            }) => Some(hash),
            _ => None,
        };
        let verified = self.hasher.verify(&password, hash).await?;
        match user {
            Some(user) if verified => sign_in(self.sessions.as_ref(), user, previous).await,
            _ => Err(AuthError::InvalidCredentials.into()),
        }
    }
}

pub struct DemoLogIn {
    pub enabled: bool,
    pub ids: Arc<dyn IdGenerator>,
    pub users: Arc<dyn UserRepository>,
    pub sessions: Arc<dyn SessionStore>,
}

impl DemoLogIn {
    /// Creates the demo accounts that do not exist yet. Does nothing while
    /// demo login is disabled. Fails if a demo name is held by an account
    /// with a password, so startup stops instead of the first demo login.
    pub async fn seed(&self) -> Result<(), Error> {
        if !self.enabled {
            return Ok(());
        }
        for account in DemoAccount::ALL {
            let user = User {
                id: self.ids.next_id()?,
                username: account.username(),
                credential: Credential::DemoOnly,
            };
            if self.users.insert(&user).await? == Inserted::UsernameTaken {
                self.demo_user(account).await?;
            }
        }
        Ok(())
    }

    async fn demo_user(&self, account: DemoAccount) -> Result<User, Error> {
        let username = account.username();
        match self.users.find_by_username(&username).await? {
            Some(user) if user.credential == Credential::DemoOnly => Ok(user),
            Some(_) => {
                Err(PortError(format!("{} is not a demo-only account", username.as_str())).into())
            }
            None => Err(PortError(format!("{} was not seeded", username.as_str())).into()),
        }
    }

    pub async fn run(
        &self,
        account: DemoAccount,
        previous: Option<&SessionToken>,
    ) -> Result<SignedIn, Error> {
        if !self.enabled {
            return Err(AuthError::DemoLoginDisabled.into());
        }
        let user = self.demo_user(account).await?;
        sign_in(self.sessions.as_ref(), user, previous).await
    }
}

pub struct LogOut {
    pub sessions: Arc<dyn SessionStore>,
}

impl LogOut {
    /// Logging out without a session, or twice, succeeds.
    pub async fn run(&self, token: Option<&SessionToken>) -> Result<(), Error> {
        if let Some(token) = token {
            self.sessions.revoke(token).await?;
        }
        Ok(())
    }
}

pub struct Authenticate {
    pub sessions: Arc<dyn SessionStore>,
    pub users: Arc<dyn UserRepository>,
}

impl Authenticate {
    pub async fn user_id(&self, token: &SessionToken) -> Result<UserId, Error> {
        self.sessions
            .resolve(token)
            .await?
            .ok_or_else(|| AuthError::Unauthenticated.into())
    }

    pub async fn current_user(&self, token: &SessionToken) -> Result<User, Error> {
        let id = self.user_id(token).await?;
        self.users
            .find_by_id(id)
            .await?
            .ok_or_else(|| AuthError::Unauthenticated.into())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use domain::{auth::PasswordHash, id::SnowflakeId};

    use super::*;

    struct OneUser(User);

    #[async_trait]
    impl UserRepository for OneUser {
        async fn insert(&self, _: &User) -> Result<Inserted, PortError> {
            Ok(Inserted::UsernameTaken)
        }

        async fn find_by_username(&self, username: &Username) -> Result<Option<User>, PortError> {
            Ok((self.0.username == *username).then(|| self.0.clone()))
        }

        async fn find_by_id(&self, id: UserId) -> Result<Option<User>, PortError> {
            Ok((self.0.id == id).then(|| self.0.clone()))
        }
    }

    #[derive(Default)]
    struct RecordingSessions(Mutex<Vec<UserId>>);

    #[async_trait]
    impl SessionStore for RecordingSessions {
        async fn create(&self, user: UserId) -> Result<SessionToken, PortError> {
            self.0.lock().unwrap().push(user);
            Ok(SessionToken::new(Secret::new("token".into())))
        }

        async fn resolve(&self, _: &SessionToken) -> Result<Option<UserId>, PortError> {
            Ok(None)
        }

        async fn revoke(&self, _: &SessionToken) -> Result<(), PortError> {
            Ok(())
        }
    }

    struct FixedIds;

    impl IdGenerator for FixedIds {
        fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
            Ok(SnowflakeId::try_from(8).unwrap())
        }
    }

    const RIGHT: &str = "correct horse battery";
    const STORED: &str = "stored hash";

    /// Matches only `RIGHT` against `STORED`, and counts its calls.
    #[derive(Default)]
    struct CountingHasher {
        hashes: Mutex<usize>,
        verifies: Mutex<usize>,
    }

    #[async_trait]
    impl PasswordHasher for CountingHasher {
        async fn hash(&self, _: &Password) -> Result<PasswordHash, PortError> {
            *self.hashes.lock().unwrap() += 1;
            Ok(PasswordHash::new(STORED.into()))
        }

        async fn verify(
            &self,
            password: &Password,
            hash: Option<&PasswordHash>,
        ) -> Result<bool, PortError> {
            *self.verifies.lock().unwrap() += 1;
            Ok(password.expose() == RIGHT && hash.is_some_and(|h| h.as_str() == STORED))
        }
    }

    struct BreachedOnly(&'static str);

    impl BreachedPasswords for BreachedOnly {
        fn contains(&self, password: &Password) -> bool {
            password.expose() == self.0
        }
    }

    fn user(username: Username, credential: Credential) -> User {
        User {
            id: SnowflakeId::try_from(7).unwrap(),
            username,
            credential,
        }
    }

    fn named(raw: &str) -> Username {
        Username::parse(raw).unwrap()
    }

    fn secret(raw: &str) -> Secret<String> {
        Secret::new(raw.to_owned())
    }

    struct Fixture {
        hasher: Arc<CountingHasher>,
        sessions: Arc<RecordingSessions>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                hasher: Arc::default(),
                sessions: Arc::default(),
            }
        }

        fn log_in(&self, row: User) -> LogIn {
            LogIn {
                users: Arc::new(OneUser(row)),
                hasher: self.hasher.clone(),
                sessions: self.sessions.clone(),
            }
        }

        fn sign_up(&self, existing: User) -> SignUp {
            SignUp {
                ids: Arc::new(FixedIds),
                users: Arc::new(OneUser(existing)),
                hasher: self.hasher.clone(),
                breached: Arc::new(BreachedOnly("password1234")),
                sessions: self.sessions.clone(),
            }
        }

        fn hashes(&self) -> usize {
            *self.hasher.hashes.lock().unwrap()
        }

        fn verifies(&self) -> usize {
            *self.hasher.verifies.lock().unwrap()
        }

        fn sessions_created(&self) -> usize {
            self.sessions.0.lock().unwrap().len()
        }
    }

    fn alice() -> User {
        user(
            named("alice"),
            Credential::Password(PasswordHash::new(STORED.into())),
        )
    }

    #[tokio::test]
    async fn log_in_with_the_right_password_verifies_once_and_signs_in() {
        let fixture = Fixture::new();
        let signed_in = fixture
            .log_in(alice())
            .run("alice", secret(RIGHT), None)
            .await
            .unwrap();
        assert_eq!(signed_in.user.username, named("alice"));
        assert_eq!(fixture.verifies(), 1);
        assert_eq!(fixture.sessions_created(), 1);
    }

    #[tokio::test]
    async fn every_failed_log_in_verifies_once_and_looks_the_same() {
        let cases = [
            ("unknown user", alice(), "nobody", RIGHT),
            ("wrong password", alice(), "alice", "not the password"),
            (
                "demo-only account",
                user(DemoAccount::Viewer.username(), Credential::DemoOnly),
                "demo-viewer",
                RIGHT,
            ),
        ];
        for (case, row, username, password) in cases {
            let fixture = Fixture::new();
            let result = fixture
                .log_in(row.clone())
                .run(username, secret(password), None)
                .await;
            assert!(
                matches!(result, Err(Error::Rejected(AuthError::InvalidCredentials))),
                "{case}: {result:?}"
            );
            assert_eq!(fixture.verifies(), 1, "{case}");
            assert_eq!(fixture.sessions_created(), 0, "{case}");
        }
    }

    #[tokio::test]
    async fn sign_up_never_hashes_a_reserved_name_or_a_breached_password() {
        let cases = [
            ("demo-mallory", RIGHT, AuthError::InvalidUsername),
            ("mallory", "password1234", AuthError::PasswordBreached),
        ];
        for (username, password, expected) in cases {
            let fixture = Fixture::new();
            let result = fixture
                .sign_up(alice())
                .run(username, secret(password), None)
                .await;
            assert!(
                matches!(&result, Err(Error::Rejected(err)) if *err == expected),
                "{username}: {result:?}"
            );
            assert_eq!(fixture.hashes(), 0, "{username}");
            assert_eq!(fixture.sessions_created(), 0, "{username}");
        }
    }

    #[tokio::test]
    async fn sign_up_with_a_taken_username_creates_no_session() {
        let fixture = Fixture::new();
        let result = fixture
            .sign_up(alice())
            .run("alice", secret(RIGHT), None)
            .await;
        assert!(
            matches!(result, Err(Error::Rejected(AuthError::UsernameTaken))),
            "{result:?}"
        );
        assert_eq!(fixture.sessions_created(), 0);
    }

    fn demo_log_in(row: User) -> (DemoLogIn, Arc<RecordingSessions>) {
        let sessions = Arc::new(RecordingSessions::default());
        let use_case = DemoLogIn {
            enabled: true,
            ids: Arc::new(FixedIds),
            users: Arc::new(OneUser(row)),
            sessions: sessions.clone(),
        };
        (use_case, sessions)
    }

    #[tokio::test]
    async fn demo_login_signs_into_a_demo_only_account() {
        let (use_case, sessions) =
            demo_log_in(user(DemoAccount::Viewer.username(), Credential::DemoOnly));
        let signed_in = use_case.run(DemoAccount::Viewer, None).await.unwrap();
        assert_eq!(signed_in.user.username, DemoAccount::Viewer.username());
        assert_eq!(sessions.0.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn demo_login_never_signs_into_an_account_with_a_password() {
        let row = user(
            DemoAccount::Streamer.username(),
            Credential::Password(PasswordHash::new("$argon2id$...".into())),
        );
        let (use_case, sessions) = demo_log_in(row);
        assert!(matches!(
            use_case.run(DemoAccount::Streamer, None).await,
            Err(Error::Internal(_))
        ));
        assert!(sessions.0.lock().unwrap().is_empty(), "no session created");
    }

    #[tokio::test]
    async fn seeding_fails_when_a_demo_name_belongs_to_a_password_account() {
        let row = user(
            DemoAccount::Streamer.username(),
            Credential::Password(PasswordHash::new("$argon2id$...".into())),
        );
        let (use_case, _) = demo_log_in(row);
        assert!(matches!(use_case.seed().await, Err(Error::Internal(_))));
    }
}
