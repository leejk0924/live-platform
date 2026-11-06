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
    pub allowed_origins: Vec<Origin>,
    pub demo_login_enabled: bool,
}

/// A browser origin, `scheme://host[:port]`, in the form browsers send it in
/// the `Origin` header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin(String);

impl Origin {
    pub fn parse(raw: &str) -> Option<Self> {
        let (scheme, authority) = raw.split_once("://")?;
        if !matches!(scheme, "http" | "https") {
            return None;
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let host_ok = !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
        // Browsers write the port as plain digits and omit the scheme's
        // default, so any other spelling would never match an `Origin` header.
        let default_port = if scheme == "https" { 443 } else { 80 };
        let port_ok = port.is_none_or(|p| {
            p.bytes().all(|b| b.is_ascii_digit())
                && !p.starts_with('0')
                && p.parse::<u16>().is_ok_and(|p| p != default_port)
        });
        (host_ok && port_ok).then(|| Self(raw.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
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
    #[error("ALLOWED_ORIGINS is not set")]
    MissingAllowedOrigins,
    #[error(
        "ALLOWED_ORIGINS must be a comma separated list of origins like https://example.com, got {0:?}"
    )]
    AllowedOrigins(String),
    #[error("DEMO_LOGIN_ENABLED must be true or false, got {0:?}")]
    DemoLoginEnabled(String),
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
        let allowed_origins = match lookup("ALLOWED_ORIGINS") {
            Ok(raw) => allowed_origins(&raw).ok_or(ConfigError::AllowedOrigins(raw))?,
            Err(VarError::NotPresent) => return Err(ConfigError::MissingAllowedOrigins),
            Err(err) => return Err(ConfigError::AllowedOrigins(err.to_string())),
        };
        let demo_login_enabled = match lookup("DEMO_LOGIN_ENABLED") {
            Err(VarError::NotPresent) => false,
            Ok(raw) if raw == "true" => true,
            Ok(raw) if raw == "false" => false,
            Ok(raw) => return Err(ConfigError::DemoLoginEnabled(raw)),
            Err(err) => return Err(ConfigError::DemoLoginEnabled(err.to_string())),
        };
        Ok(Self {
            listen_addr,
            database_url,
            worker_lease_ttl,
            allowed_origins,
            demo_login_enabled,
        })
    }
}

/// At least one origin; blanks around commas are ignored.
fn allowed_origins(raw: &str) -> Option<Vec<Origin>> {
    let origins = raw
        .split(',')
        .map(|entry| Origin::parse(entry.trim()))
        .collect::<Option<Vec<_>>>()?;
    (!origins.is_empty()).then_some(origins)
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
    const ORIGIN: &str = "https://localhost";

    /// `DATABASE_URL` and `ALLOWED_ORIGINS` are filled in unless `vars` sets them.
    fn from(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let required = [("DATABASE_URL", URL), ("ALLOWED_ORIGINS", ORIGIN)];
        from_exact(&[&required[..], vars].concat())
    }

    fn from_exact(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Config::from_lookup(|key| vars.get(key).cloned().ok_or(VarError::NotPresent))
    }

    #[test]
    fn defaults_apply_when_only_required_variables_are_set() {
        let config = from(&[]).unwrap();
        assert_eq!(config.listen_addr, SocketAddr::from(([0, 0, 0, 0], 8080)));
        assert_eq!(config.database_url.expose(), URL);
        assert_eq!(config.worker_lease_ttl, Duration::from_secs(30));
        assert_eq!(config.allowed_origins, [Origin::parse(ORIGIN).unwrap()]);
        assert!(!config.demo_login_enabled);
    }

    #[test]
    fn database_url_is_required() {
        assert!(matches!(
            from_exact(&[("ALLOWED_ORIGINS", ORIGIN)]),
            Err(ConfigError::MissingDatabaseUrl)
        ));
        assert!(matches!(
            from(&[("DATABASE_URL", "")]),
            Err(ConfigError::MissingDatabaseUrl)
        ));
    }

    #[test]
    fn lease_ttl_must_be_at_least_three_seconds() {
        let ttl = |raw| from(&[("WORKER_LEASE_TTL_SECS", raw)]);
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
        let err = from(&[("API_ADDR", "nope")]).unwrap_err();
        assert!(matches!(err, ConfigError::ListenAddr(_)));
        assert!(!err.to_string().contains("s3cr3t-pw"));
    }

    #[test]
    fn logging_the_config_redacts_the_database_url() {
        let config = from(&[]).unwrap();
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
    fn allowed_origins_is_a_required_list_of_origins() {
        let origins = |raw| from(&[("ALLOWED_ORIGINS", raw)]).map(|c| c.allowed_origins);
        assert_eq!(
            origins(" https://a.example , http://localhost:5173").unwrap(),
            [
                Origin::parse("https://a.example").unwrap(),
                Origin::parse("http://localhost:5173").unwrap()
            ]
        );
        assert!(matches!(
            from_exact(&[("DATABASE_URL", URL)]),
            Err(ConfigError::MissingAllowedOrigins)
        ));
        assert_eq!(
            origins("http://a.example:443,https://a.example:80")
                .unwrap()
                .len(),
            2,
            "a port is only implied by its own scheme"
        );
        for bad in [
            "",
            ",",
            "https://a.example,",
            "localhost",
            "https://",
            "ftp://a.example",
            "https://a.example/",
            "https://a.example/path",
            "https://A.example",
            "https://a.example:0",
            "https://a.example:99999",
            "https://a.example:",
            "https://a.example:+443",
            "https://a.example:0443",
            "https://a.example:443",
            "http://a.example:80",
            "https://user@a.example",
            "*",
        ] {
            assert!(
                matches!(origins(bad), Err(ConfigError::AllowedOrigins(_))),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn demo_login_flag_accepts_only_true_or_false() {
        let demo = |raw| from(&[("DEMO_LOGIN_ENABLED", raw)]).map(|c| c.demo_login_enabled);
        assert!(demo("true").unwrap());
        assert!(!demo("false").unwrap());
        for bad in ["", "1", "TRUE", "yes", " true"] {
            assert!(
                matches!(demo(bad), Err(ConfigError::DemoLoginEnabled(_))),
                "{bad:?} should be rejected"
            );
        }
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
