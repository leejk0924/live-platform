//! Password hashing and the breached password list.

use std::{collections::HashSet, sync::Arc};

use application::ports::auth::{BreachedPasswords, PasswordHasher, PortError};
use argon2::{
    Argon2,
    password_hash::{Error as HashError, PasswordHasher as _, PasswordVerifier as _},
};
use async_trait::async_trait;
use domain::auth::{Password, PasswordHash};
use tokio::sync::Semaphore;

/// Argon2id with the OWASP minimum: 19 MiB memory, 2 passes, 1 lane. These
/// are also `argon2`'s defaults; the test below pins them.
pub struct Argon2Hasher {
    dummy: Arc<str>,
    permits: Arc<Semaphore>,
}

impl Argon2Hasher {
    /// Hashes the dummy password once, so it blocks for one hash.
    pub fn new() -> Result<Self, PortError> {
        let parallelism = std::thread::available_parallelism().map_or(1, |n| n.get());
        let dummy = hash_blocking(b"dummy password for missing accounts")?;
        Ok(Self {
            dummy: dummy.into(),
            permits: Arc::new(Semaphore::new(parallelism)),
        })
    }

    /// Runs `work` on the blocking pool, at most one per core at a time, so a
    /// login burst cannot starve the async workers or exhaust memory.
    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> Result<T, PortError> + Send + 'static,
    ) -> Result<T, PortError> {
        let _permit = self
            .permits
            .acquire()
            .await
            .map_err(|err| PortError(err.to_string()))?;
        tokio::task::spawn_blocking(work)
            .await
            .map_err(|err| PortError(format!("password hashing task failed: {err}")))?
    }
}

fn hash_blocking(password: &[u8]) -> Result<String, PortError> {
    Argon2::default()
        .hash_password(password)
        .map(|hash| hash.to_string())
        .map_err(|err| PortError(format!("password hashing failed: {err}")))
}

fn verify_blocking(password: &[u8], phc: &str) -> Result<bool, PortError> {
    match Argon2::default().verify_password(password, phc) {
        Ok(()) => Ok(true),
        Err(HashError::PasswordInvalid) => Ok(false),
        Err(err) => Err(PortError(format!("password verification failed: {err}"))),
    }
}

#[async_trait]
impl PasswordHasher for Argon2Hasher {
    async fn hash(&self, password: &Password) -> Result<PasswordHash, PortError> {
        let password = password.clone();
        let phc = self
            .run(move || hash_blocking(password.expose().as_bytes()))
            .await?;
        Ok(PasswordHash::new(phc))
    }

    async fn verify(
        &self,
        password: &Password,
        hash: Option<&PasswordHash>,
    ) -> Result<bool, PortError> {
        let password = password.clone();
        let (phc, real): (Arc<str>, bool) = match hash {
            Some(hash) => (hash.as_str().into(), true),
            None => (Arc::clone(&self.dummy), false),
        };
        let matched = self
            .run(move || verify_blocking(password.expose().as_bytes(), &phc))
            .await?;
        Ok(real && matched)
    }
}

/// Entries of SecLists' `10k-most-common.txt` that are long enough to pass
/// the length policy; shorter ones are rejected before this list is asked.
const BREACHED_LIST: &str = include_str!("../data/breached-passwords.txt");

pub struct BreachedPasswordList(HashSet<String>);

impl BreachedPasswordList {
    pub fn load() -> Self {
        Self(
            BREACHED_LIST
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_lowercase)
                .collect(),
        )
    }
}

impl BreachedPasswords for BreachedPasswordList {
    fn contains(&self, password: &Password) -> bool {
        self.0.contains(&password.expose().to_lowercase())
    }
}

#[cfg(test)]
mod tests {
    use domain::secret::Secret;

    use super::*;

    fn password(raw: &str) -> Password {
        Password::parse(Secret::new(raw.to_owned())).unwrap()
    }

    #[tokio::test]
    async fn hashes_with_owasp_argon2id_and_verifies() {
        let hasher = Argon2Hasher::new().unwrap();
        let hash = hasher.hash(&password("correct horse")).await.unwrap();

        assert!(
            hash.as_str().starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "{}",
            hash.as_str()
        );
        assert!(
            hasher
                .verify(&password("correct horse"), Some(&hash))
                .await
                .unwrap()
        );
        assert!(
            !hasher
                .verify(&password("wrong horse!"), Some(&hash))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn the_same_password_gets_a_fresh_salt() {
        let hasher = Argon2Hasher::new().unwrap();
        let a = hasher.hash(&password("correct horse")).await.unwrap();
        let b = hasher.hash(&password("correct horse")).await.unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn a_missing_hash_never_verifies_even_for_the_dummy_password() {
        let hasher = Argon2Hasher::new().unwrap();
        let dummy = password("dummy password for missing accounts");
        assert!(!hasher.verify(&dummy, None).await.unwrap());
    }

    #[tokio::test]
    async fn a_corrupt_stored_hash_is_an_error_not_a_mismatch() {
        let hasher = Argon2Hasher::new().unwrap();
        let corrupt = PasswordHash::new("not a phc string".into());
        assert!(
            hasher
                .verify(&password("correct horse"), Some(&corrupt))
                .await
                .is_err()
        );
    }

    #[test]
    fn breached_list_matches_case_insensitively() {
        let list = BreachedPasswordList::load();
        assert!(list.contains(&password("1234567890")));
        assert!(list.contains(&password("QwertyUiop")));
        assert!(!list.contains(&password("a fresh passphrase")));
        assert!(list.0.iter().all(|entry| entry.chars().count() >= 10));
    }
}
