use std::{
    env::VarError,
    net::{AddrParseError, SocketAddr},
    time::Duration,
};

use domain::secret::Secret;
use infrastructure::media::PublishKey;
use thiserror::Error;

const DEFAULT_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_MEDIAMTX_API_URL: &str = "http://mediamtx:9997";
const DEFAULT_LEASE_TTL_SECS: u64 = 30;
/// The heartbeat renews every TTL/3, so shorter TTLs leave no room for a slow renewal.
const MIN_LEASE_TTL_SECS: u64 = 3;
/// `openssl rand -hex 32` prints 64.
const MIN_MEDIA_SECRET_LEN: usize = 32;

#[derive(Debug)]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub database_url: Secret<String>,
    pub worker_lease_ttl: Duration,
    pub allowed_origins: Vec<Origin>,
    pub demo_login_enabled: bool,
    pub publish_key: PublishKey,
    /// MediaMTX Control API base URL, reachable only inside the compose
    /// network.
    pub mediamtx_api_url: String,
    /// What MediaMTX presents on `/internal/*`. Letters and digits only, so
    /// it can sit in the URL MediaMTX calls without escaping.
    pub media_secret: Secret<String>,
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
    #[error("PUBLISH_JWT_PRIVATE_KEY is not set")]
    MissingPublishKey,
    #[error(
        "PUBLISH_JWT_PRIVATE_KEY must be 32 random bytes in standard base64 (openssl rand -base64 32)"
    )]
    InvalidPublishKey,
    #[error("MEDIAMTX_API_URL must be an http(s) URL without a trailing slash, got {0:?}")]
    MediaMtxApiUrl(String),
    /// Leaves the value out: it would carry the credentials.
    #[error("MEDIAMTX_API_URL must not contain user:password@; the API signs its own token")]
    MediaMtxApiUrlCredentials,
    #[error("MEDIA_HOOK_SECRET is not set")]
    MissingMediaSecret,
    #[error(
        "MEDIA_HOOK_SECRET must be at least {MIN_MEDIA_SECRET_LEN} letters or digits (openssl rand -hex 32)"
    )]
    InvalidMediaSecret,
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
        let publish_key = match lookup("PUBLISH_JWT_PRIVATE_KEY") {
            Ok(raw) if !raw.is_empty() => PublishKey::from_base64_seed(&Secret::new(raw))
                .map_err(|_| ConfigError::InvalidPublishKey)?,
            Ok(_) | Err(VarError::NotPresent) => return Err(ConfigError::MissingPublishKey),
            Err(VarError::NotUnicode(_)) => return Err(ConfigError::InvalidPublishKey),
        };
        let mediamtx_api_url = match lookup("MEDIAMTX_API_URL") {
            Err(VarError::NotPresent) => DEFAULT_MEDIAMTX_API_URL.to_owned(),
            Ok(raw) if has_userinfo(&raw) => return Err(ConfigError::MediaMtxApiUrlCredentials),
            Ok(raw)
                if (raw.starts_with("http://") || raw.starts_with("https://"))
                    && !raw.ends_with('/') =>
            {
                raw
            }
            Ok(raw) => return Err(ConfigError::MediaMtxApiUrl(raw)),
            Err(err) => return Err(ConfigError::MediaMtxApiUrl(err.to_string())),
        };
        let media_secret = match lookup("MEDIA_HOOK_SECRET") {
            Ok(raw)
                if raw.len() >= MIN_MEDIA_SECRET_LEN
                    && raw.bytes().all(|b| b.is_ascii_alphanumeric()) =>
            {
                Secret::new(raw)
            }
            Ok(raw) if raw.is_empty() => return Err(ConfigError::MissingMediaSecret),
            Err(VarError::NotPresent) => return Err(ConfigError::MissingMediaSecret),
            Ok(_) | Err(VarError::NotUnicode(_)) => return Err(ConfigError::InvalidMediaSecret),
        };
        Ok(Self {
            listen_addr,
            database_url,
            worker_lease_ttl,
            allowed_origins,
            demo_login_enabled,
            publish_key,
            mediamtx_api_url,
            media_secret,
        })
    }
}

/// Whether the authority part of `url` (after the scheme, up to the path)
/// has a `user:password@` prefix.
fn has_userinfo(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    authority.contains('@')
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
    /// RFC 8037 appendix A.1's private key.
    const KEY: &str = "nWGxne/9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A=";
    const MEDIA_SECRET: &str = "0123456789abcdef0123456789abcdef";

    /// `DATABASE_URL`, `ALLOWED_ORIGINS`, `PUBLISH_JWT_PRIVATE_KEY` and
    /// `MEDIA_HOOK_SECRET` are filled in unless `vars` sets them.
    fn from(vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let required = [
            ("DATABASE_URL", URL),
            ("ALLOWED_ORIGINS", ORIGIN),
            ("PUBLISH_JWT_PRIVATE_KEY", KEY),
            ("MEDIA_HOOK_SECRET", MEDIA_SECRET),
        ];
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
        assert_eq!(config.mediamtx_api_url, "http://mediamtx:9997");
    }

    #[test]
    fn publish_key_is_required_and_never_echoed() {
        assert!(matches!(
            from_exact(&[("DATABASE_URL", URL), ("ALLOWED_ORIGINS", ORIGIN)]),
            Err(ConfigError::MissingPublishKey)
        ));
        for bad in [
            "",
            "change-me",
            "nWGxne/9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A",
        ] {
            let err = from(&[("PUBLISH_JWT_PRIVATE_KEY", bad)]).unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::MissingPublishKey | ConfigError::InvalidPublishKey
                ),
                "{bad:?}"
            );
            assert!(bad.is_empty() || !err.to_string().contains(bad), "{err}");
        }
        let config = from(&[]).unwrap();
        assert_eq!(
            config.publish_key.public_jwk().kid,
            "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k"
        );
        assert!(!format!("{config:?}").contains(KEY));
    }

    #[test]
    fn media_secret_is_required_long_alphanumeric_and_never_echoed() {
        assert!(matches!(
            from_exact(&[
                ("DATABASE_URL", URL),
                ("ALLOWED_ORIGINS", ORIGIN),
                ("PUBLISH_JWT_PRIVATE_KEY", KEY),
            ]),
            Err(ConfigError::MissingMediaSecret)
        ));
        let short = "a".repeat(MIN_MEDIA_SECRET_LEN - 1);
        let with_symbol = format!("{}@x", "a".repeat(MIN_MEDIA_SECRET_LEN));
        for bad in [
            "",
            "change-me",
            short.as_str(),
            with_symbol.as_str(),
            "0123456789abcdef0123456789abcdef ",
        ] {
            let err = from(&[("MEDIA_HOOK_SECRET", bad)]).unwrap_err();
            assert!(
                matches!(
                    err,
                    ConfigError::MissingMediaSecret | ConfigError::InvalidMediaSecret
                ),
                "{bad:?}"
            );
            assert!(bad.is_empty() || !err.to_string().contains(bad), "{err}");
        }
        let config = from(&[]).unwrap();
        assert_eq!(config.media_secret.expose(), MEDIA_SECRET);
        assert!(!format!("{config:?}").contains(MEDIA_SECRET));
    }

    #[test]
    fn mediamtx_api_url_must_be_http_without_trailing_slash() {
        let url = |raw| from(&[("MEDIAMTX_API_URL", raw)]).map(|c| c.mediamtx_api_url);
        assert_eq!(
            url("http://127.0.0.1:9997").unwrap(),
            "http://127.0.0.1:9997"
        );
        for bad in ["", "mediamtx:9997", "http://mediamtx:9997/"] {
            assert!(
                matches!(url(bad), Err(ConfigError::MediaMtxApiUrl(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn mediamtx_api_url_with_credentials_is_rejected_without_echoing_them() {
        for bad in [
            "http://admin:hunter2@mediamtx:9997",
            "https://hunter2@mediamtx:9997",
            "http://admin:hunter2@mediamtx:9997/v3",
        ] {
            let err = from(&[("MEDIAMTX_API_URL", bad)]).unwrap_err();
            assert!(!err.to_string().contains("hunter2"), "{err}");
        }
        assert_eq!(
            from(&[("MEDIAMTX_API_URL", "http://mediamtx:9997/a@b")])
                .unwrap()
                .mediamtx_api_url,
            "http://mediamtx:9997/a@b",
            "an @ after the host is not userinfo"
        );
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
