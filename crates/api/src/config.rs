use std::{
    env::VarError,
    net::{AddrParseError, SocketAddr},
    time::Duration,
};

use domain::secret::Secret;
use thiserror::Error;

const DEFAULT_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_LEASE_TTL_SECS: u64 = 30;
/// The heartbeat renews every TTL/3, so shorter TTLs leave no room for a slow renewal.
const MIN_LEASE_TTL_SECS: u64 = 3;

#[derive(Debug)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub database_url: Secret<String>,
    pub worker_lease_ttl: Duration,
}

/// Messages name the variable but never echo a secret variable's value.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid API_ADDR: {0}")]
    ListenAddr(String),
    #[error("DATABASE_URL is not set")]
    MissingDatabaseUrl,
    #[error("DATABASE_URL is not valid unicode")]
    DatabaseUrlNotUnicode,
    #[error("WORKER_LEASE_TTL_SECS must be an integer of at least {MIN_LEASE_TTL_SECS}, got {0:?}")]
    LeaseTtl(String),
}

impl Config {
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Result<String, VarError>,
    ) -> Result<Self, ConfigError> {
        let listen_addr = listen_addr(lookup("API_ADDR")).map_err(ConfigError::ListenAddr)?;
        let database_url = match lookup("DATABASE_URL") {
            Ok(url) if !url.is_empty() => Secret::new(url),
            Ok(_) | Err(VarError::NotPresent) => return Err(ConfigError::MissingDatabaseUrl),
            Err(VarError::NotUnicode(_)) => return Err(ConfigError::DatabaseUrlNotUnicode),
        };
        let worker_lease_ttl = match lookup("WORKER_LEASE_TTL_SECS") {
            Err(VarError::NotPresent) => Duration::from_secs(DEFAULT_LEASE_TTL_SECS),
            Err(err @ VarError::NotUnicode(_)) => {
                return Err(ConfigError::LeaseTtl(err.to_string()));
            }
            Ok(raw) => match raw.parse::<u64>() {
                Ok(secs) if secs >= MIN_LEASE_TTL_SECS => Duration::from_secs(secs),
                _ => return Err(ConfigError::LeaseTtl(raw)),
            },
        };
        Ok(Self {
            listen_addr,
            database_url,
            worker_lease_ttl,
        })
    }
}

/// Also used alone by the `healthcheck` subcommand, which needs nothing else.
pub fn listen_addr(var: Result<String, VarError>) -> Result<SocketAddr, String> {
    match var {
        Ok(raw) => raw.parse().map_err(|err: AddrParseError| err.to_string()),
        Err(VarError::NotPresent) => Ok(DEFAULT_ADDR.parse().expect("default address is valid")),
        Err(err @ VarError::NotUnicode(_)) => Err(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::test_support::LogBuffer;

    const URL: &str = "postgres://app:s3cr3t-pw@db:5432/app";

    fn from(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|key| vars.get(key).cloned().ok_or(VarError::NotPresent))
    }

    #[test]
    fn defaults_apply_when_only_database_url_is_set() {
        let config = from(&[("DATABASE_URL", URL)]).unwrap();
        assert_eq!(config.listen_addr, SocketAddr::from(([0, 0, 0, 0], 8080)));
        assert_eq!(config.database_url.expose(), URL);
        assert_eq!(config.worker_lease_ttl, Duration::from_secs(30));
    }

    #[test]
    fn database_url_is_required() {
        assert!(matches!(from(&[]), Err(ConfigError::MissingDatabaseUrl)));
        assert!(matches!(
            from(&[("DATABASE_URL", "")]),
            Err(ConfigError::MissingDatabaseUrl)
        ));
    }

    #[test]
    fn lease_ttl_must_be_at_least_three_seconds() {
        let ttl = |raw| from(&[("DATABASE_URL", URL), ("WORKER_LEASE_TTL_SECS", raw)]);
        assert_eq!(ttl("3").unwrap().worker_lease_ttl, Duration::from_secs(3));
        for bad in ["2", "0", "-5", "abc", ""] {
            assert!(
                matches!(ttl(bad), Err(ConfigError::LeaseTtl(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn errors_never_contain_the_database_url() {
        let err = from(&[("DATABASE_URL", URL), ("API_ADDR", "nope")]).unwrap_err();
        assert!(matches!(err, ConfigError::ListenAddr(_)));
        assert!(!err.to_string().contains("s3cr3t-pw"));
    }

    #[test]
    fn logging_the_config_redacts_the_database_url() {
        let config = from(&[("DATABASE_URL", URL)]).unwrap();
        let logs = LogBuffer::default();

        tracing::subscriber::with_default(logs.subscriber(), || tracing::info!(?config, "loaded"));

        let logged = logs.contents();
        assert!(
            logged.contains("REDACTED"),
            "config should be logged: {logged}"
        );
        assert!(!logged.contains("s3cr3t-pw"), "password leaked: {logged}");
    }

    #[test]
    fn listen_addr_defaults_only_when_unset() {
        assert_eq!(
            listen_addr(Err(VarError::NotPresent)),
            Ok(SocketAddr::from(([0, 0, 0, 0], 8080)))
        );
        assert!(listen_addr(Err(VarError::NotUnicode(std::ffi::OsString::new()))).is_err());
        assert!(listen_addr(Ok(String::new())).is_err());
        assert_eq!(
            listen_addr(Ok("127.0.0.1:9000".into())),
            Ok(SocketAddr::from(([127, 0, 0, 1], 9000)))
        );
    }

    #[test]
    fn listen_addr_rejects_garbage() {
        assert!(listen_addr(Ok("nope".into())).is_err());
    }
}
