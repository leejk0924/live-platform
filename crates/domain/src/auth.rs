//! Accounts, credentials and sessions.

use thiserror::Error;

use crate::{
    error::{DomainError, ErrorKind},
    id::SnowflakeId,
    secret::Secret,
};

pub type UserId = SnowflakeId;

pub const USERNAME_MIN_CHARS: usize = 3;
pub const USERNAME_MAX_CHARS: usize = 20;
pub const PASSWORD_MIN_CHARS: usize = 10;
pub const PASSWORD_MAX_CHARS: usize = 128;

/// Kept for the demo accounts, so signup can never create a look-alike.
const RESERVED_PREFIX: &str = "demo-";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Username(String);

impl Username {
    /// Accepts 3 to 20 characters of `a-z`, `0-9`, `_` and `-`.
    pub fn parse(raw: &str) -> Result<Self, AuthError> {
        let valid_len = (USERNAME_MIN_CHARS..=USERNAME_MAX_CHARS).contains(&raw.len());
        let valid_chars = raw
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'));
        if valid_len && valid_chars {
            Ok(Self(raw.to_owned()))
        } else {
            Err(AuthError::InvalidUsername)
        }
    }

    /// Reserved names belong to [`DemoAccount`]s and cannot be signed up for.
    pub fn is_reserved(&self) -> bool {
        self.0.starts_with(RESERVED_PREFIX)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A plaintext password that satisfies the length policy. It never leaves a
/// [`Secret`].
#[derive(Debug, Clone)]
pub struct Password(Secret<String>);

impl Password {
    pub fn parse(raw: Secret<String>) -> Result<Self, AuthError> {
        let chars = raw.expose().chars().count();
        if chars < PASSWORD_MIN_CHARS {
            Err(AuthError::PasswordTooShort)
        } else if chars > PASSWORD_MAX_CHARS {
            Err(AuthError::PasswordTooLong)
        } else {
            Ok(Self(raw))
        }
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

/// A password hash in PHC string format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

impl PasswordHash {
    pub fn new(phc: String) -> Self {
        Self(phc)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// How an account proves who it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Credential {
    Password(PasswordHash),
    /// Signed in only through demo login; no password ever matches.
    DemoOnly,
}

/// What an account may do. Only a streamer can create broadcasts; both roles
/// can watch, comment and donate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Streamer,
    Viewer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub username: Username,
    pub credential: Credential,
    pub role: Role,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoAccount {
    Streamer,
    Viewer,
}

impl DemoAccount {
    pub const ALL: [Self; 2] = [Self::Streamer, Self::Viewer];

    pub fn username(self) -> Username {
        let name = match self {
            Self::Streamer => "demo-streamer",
            Self::Viewer => "demo-viewer",
        };
        Username(name.to_owned())
    }

    /// The role the demo account is seeded with.
    pub fn role(self) -> Role {
        match self {
            Self::Streamer => Role::Streamer,
            Self::Viewer => Role::Viewer,
        }
    }
}

/// The opaque value a client presents to prove its session.
#[derive(Debug, Clone)]
pub struct SessionToken(Secret<String>);

impl SessionToken {
    pub fn new(raw: Secret<String>) -> Self {
        Self(raw)
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum AuthError {
    #[error(
        "username must be 3 to 20 characters of a-z, 0-9, _ or - and must not start with demo-"
    )]
    InvalidUsername,
    #[error("password must be at least 10 characters")]
    PasswordTooShort,
    #[error("password must be at most 128 characters")]
    PasswordTooLong,
    #[error("password appears in a list of breached passwords")]
    PasswordBreached,
    #[error("username is already taken")]
    UsernameTaken,
    /// One variant for unknown users, wrong passwords and demo-only
    /// accounts, so the answer never reveals which one it was.
    #[error("invalid username or password")]
    InvalidCredentials,
    #[error("authentication required")]
    Unauthenticated,
    #[error("resource not found")]
    DemoLoginDisabled,
}

impl DomainError for AuthError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidUsername
            | Self::PasswordTooShort
            | Self::PasswordTooLong
            | Self::PasswordBreached => ErrorKind::InvalidInput,
            Self::UsernameTaken => ErrorKind::Conflict,
            Self::InvalidCredentials | Self::Unauthenticated => ErrorKind::Unauthenticated,
            Self::DemoLoginDisabled => ErrorKind::NotFound,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::InvalidUsername => "INVALID_USERNAME",
            Self::PasswordTooShort => "PASSWORD_TOO_SHORT",
            Self::PasswordTooLong => "PASSWORD_TOO_LONG",
            Self::PasswordBreached => "PASSWORD_BREACHED",
            Self::UsernameTaken => "USERNAME_TAKEN",
            Self::InvalidCredentials => "INVALID_CREDENTIALS",
            Self::Unauthenticated => "UNAUTHENTICATED",
            // Indistinguishable from a route that does not exist.
            Self::DemoLoginDisabled => "NOT_FOUND",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn password(raw: &str) -> Result<Password, AuthError> {
        Password::parse(Secret::new(raw.to_owned()))
    }

    #[test]
    fn username_accepts_the_allowed_alphabet_within_3_to_20() {
        for ok in ["abc", "a_b-c9", "aaaaaaaaaaaaaaaaaaaa", "0-_"] {
            assert_eq!(Username::parse(ok).unwrap().as_str(), ok);
        }
    }

    #[test]
    fn username_rejects_bad_length_or_characters() {
        for bad in [
            "",
            "ab",
            "aaaaaaaaaaaaaaaaaaaaa",
            "Abc",
            "a b",
            "abc!",
            "한글이름",
            "abc\n",
            "éabc",
        ] {
            assert_eq!(
                Username::parse(bad),
                Err(AuthError::InvalidUsername),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn demo_prefix_is_reserved_and_demo_accounts_use_it() {
        assert!(Username::parse("demo-x").unwrap().is_reserved());
        assert!(!Username::parse("demox").unwrap().is_reserved());
        assert!(!Username::parse("my-demo-x").unwrap().is_reserved());
        for account in DemoAccount::ALL {
            let name = account.username();
            assert!(name.is_reserved(), "{name:?}");
            assert_eq!(Username::parse(name.as_str()), Ok(name));
        }
    }

    #[test]
    fn password_length_counts_characters_not_bytes() {
        assert_eq!(
            password("123456789").unwrap_err(),
            AuthError::PasswordTooShort
        );
        assert!(password("1234567890").is_ok());
        assert!(password(&"가".repeat(10)).is_ok(), "30 bytes, 10 chars");
        assert_eq!(
            password(&"가".repeat(9)).unwrap_err(),
            AuthError::PasswordTooShort
        );
        assert!(password(&"가".repeat(128)).is_ok());
        assert_eq!(
            password(&"a".repeat(129)).unwrap_err(),
            AuthError::PasswordTooLong
        );
    }

    #[test]
    fn password_debug_is_redacted() {
        let pw = password("hunter2hunter2").unwrap();
        assert!(!format!("{pw:?}").contains("hunter2"));
    }

    #[test]
    fn errors_have_stable_codes_and_kinds() {
        assert_eq!(AuthError::UsernameTaken.kind(), ErrorKind::Conflict);
        assert_eq!(
            AuthError::InvalidCredentials.kind(),
            ErrorKind::Unauthenticated
        );
        assert_eq!(AuthError::DemoLoginDisabled.code(), "NOT_FOUND");
        assert_eq!(AuthError::PasswordBreached.code(), "PASSWORD_BREACHED");
    }
}
