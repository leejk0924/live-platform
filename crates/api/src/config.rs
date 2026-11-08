use std::{
    env::VarError,
    net::{AddrParseError, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use domain::secret::Secret;
use infrastructure::{
    media::PublishKey,
    recording::{CloudFrontKey, S3Settings},
};
use thiserror::Error;

const DEFAULT_ADDR: &str = "0.0.0.0:8080";
const DEFAULT_MEDIAMTX_API_URL: &str = "http://mediamtx:9997";
const DEFAULT_LEASE_TTL_SECS: u64 = 30;
/// The heartbeat renews every TTL/3, so shorter TTLs leave no room for a slow renewal.
const MIN_LEASE_TTL_SECS: u64 = 3;
/// `openssl rand -hex 32` prints 64.
const MIN_MEDIA_SECRET_LEN: usize = 32;
/// Where MediaMTX records, mounted at the same path in the API container.
const DEFAULT_RECORDING_SPOOL_DIR: &str = "/recordings";
const DEFAULT_RECORDING_STORE_DIR: &str = "/recording-store";

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
    pub recordings: RecordingConfig,
    /// `None` turns donations off.
    pub payments: Option<TossConfig>,
}

/// Toss Payments test keys. The client key is handed to the browser, but
/// stays out of the logs all the same.
#[derive(Debug)]
pub struct TossConfig {
    pub client_key: Secret<String>,
    pub secret_key: Secret<String>,
}

#[derive(Debug)]
pub struct RecordingConfig {
    /// Where MediaMTX leaves finished segments.
    pub spool_dir: PathBuf,
    pub storage: RecordingStorageConfig,
}

#[derive(Debug)]
pub enum RecordingStorageConfig {
    /// Kept in a directory on this host and served by the API.
    Local { store_dir: PathBuf },
    /// Kept in an S3 bucket and served by CloudFront.
    S3 {
        s3: S3Settings,
        cloudfront: CloudFrontSettings,
    },
}

/// How viewers reach recordings in S3: a CloudFront distribution that only
/// accepts URLs signed with this key pair.
#[derive(Debug)]
pub struct CloudFrontSettings {
    /// Such as `d111111abcdef8.cloudfront.net`.
    pub domain: String,
    pub key_pair_id: String,
    pub private_key: CloudFrontKey,
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
    #[error("{0} must not be empty")]
    EmptyPath(&'static str),
    #[error("{0} must be valid UTF-8")]
    PathNotUnicode(&'static str),
    #[error("RECORDING_STORAGE must be local or s3, got {0:?}")]
    RecordingStorage(String),
    /// Leaves the value out: it may be a secret.
    #[error("{0} is required when RECORDING_STORAGE=s3")]
    MissingS3Setting(&'static str),
    #[error("S3_ENDPOINT must be an http(s) URL without a trailing slash, got {0:?}")]
    S3Endpoint(String),
    /// Leaves the value out: it would carry the credentials.
    #[error("S3_ENDPOINT must not contain user:password@; set S3_ACCESS_KEY_ID instead")]
    S3EndpointCredentials,
    #[error("CLOUDFRONT_DOMAIN must be a host name like d111111abcdef8.cloudfront.net, got {0:?}")]
    CloudFrontDomain(String),
    #[error("CLOUDFRONT_PRIVATE_KEY must be an ECDSA P-256 private key in PEM")]
    CloudFrontPrivateKey,
    #[error(
        "{0} is required when the other Toss key is set; leave both empty to turn donations off"
    )]
    MissingTossKey(&'static str),
    /// Leaves the value out: it is a key.
    #[error("{0}: only Toss test keys (test_...) are accepted")]
    TossKeyNotTest(&'static str),
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
        let recordings = recording_config(&lookup)?;
        let payments = toss_config(&lookup)?;
        Ok(Self {
            listen_addr,
            database_url,
            worker_lease_ttl,
            allowed_origins,
            demo_login_enabled,
            publish_key,
            mediamtx_api_url,
            media_secret,
            recordings,
            payments,
        })
    }
}

/// Both keys or neither; empty counts as unset, since compose passes them
/// as `${VAR:-}`.
fn toss_config(
    lookup: &impl Fn(&str) -> Result<String, VarError>,
) -> Result<Option<TossConfig>, ConfigError> {
    let read = |var: &'static str| match lookup(var) {
        Ok(raw) if raw.is_empty() => Ok(None),
        Ok(raw) if raw.starts_with("test_") => Ok(Some(Secret::new(raw))),
        Err(VarError::NotPresent) => Ok(None),
        Ok(_) | Err(VarError::NotUnicode(_)) => Err(ConfigError::TossKeyNotTest(var)),
    };
    match (read("TOSS_CLIENT_KEY")?, read("TOSS_SECRET_KEY")?) {
        (None, None) => Ok(None),
        (Some(_), None) => Err(ConfigError::MissingTossKey("TOSS_SECRET_KEY")),
        (None, Some(_)) => Err(ConfigError::MissingTossKey("TOSS_CLIENT_KEY")),
        (Some(client_key), Some(secret_key)) => Ok(Some(TossConfig {
            client_key,
            secret_key,
        })),
    }
}

fn dir_or(
    lookup: &impl Fn(&str) -> Result<String, VarError>,
    var: &'static str,
    default: &str,
) -> Result<PathBuf, ConfigError> {
    match lookup(var) {
        Err(VarError::NotPresent) => Ok(PathBuf::from(default)),
        Ok(raw) if !raw.is_empty() => Ok(PathBuf::from(raw)),
        Ok(_) => Err(ConfigError::EmptyPath(var)),
        Err(VarError::NotUnicode(_)) => Err(ConfigError::PathNotUnicode(var)),
    }
}

fn recording_config(
    lookup: &impl Fn(&str) -> Result<String, VarError>,
) -> Result<RecordingConfig, ConfigError> {
    let spool_dir = dir_or(lookup, "RECORDING_SPOOL_DIR", DEFAULT_RECORDING_SPOOL_DIR)?;
    let storage = match lookup("RECORDING_STORAGE") {
        Err(VarError::NotPresent) => "local".to_owned(),
        Ok(raw) if raw.is_empty() => "local".to_owned(),
        Ok(raw) => raw,
        Err(err) => return Err(ConfigError::RecordingStorage(err.to_string())),
    };
    let storage = match storage.as_str() {
        "local" => RecordingStorageConfig::Local {
            store_dir: dir_or(lookup, "RECORDING_STORE_DIR", DEFAULT_RECORDING_STORE_DIR)?,
        },
        "s3" => s3_config(lookup)?,
        _ => return Err(ConfigError::RecordingStorage(storage)),
    };
    Ok(RecordingConfig { spool_dir, storage })
}

fn s3_config(
    lookup: &impl Fn(&str) -> Result<String, VarError>,
) -> Result<RecordingStorageConfig, ConfigError> {
    let required = |var: &'static str| match lookup(var) {
        Ok(raw) if !raw.is_empty() => Ok(raw),
        _ => Err(ConfigError::MissingS3Setting(var)),
    };
    let endpoint = required("S3_ENDPOINT")?;
    if has_userinfo(&endpoint) {
        return Err(ConfigError::S3EndpointCredentials);
    }
    if !(endpoint.starts_with("http://") || endpoint.starts_with("https://"))
        || endpoint.ends_with('/')
    {
        return Err(ConfigError::S3Endpoint(endpoint));
    }
    let domain = required("CLOUDFRONT_DOMAIN")?;
    let host_ok = domain
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
    if !host_ok || !domain.contains('.') {
        return Err(ConfigError::CloudFrontDomain(domain));
    }
    let private_key = CloudFrontKey::from_pem(&Secret::new(required("CLOUDFRONT_PRIVATE_KEY")?))
        .map_err(|_| ConfigError::CloudFrontPrivateKey)?;
    Ok(RecordingStorageConfig::S3 {
        s3: S3Settings {
            endpoint,
            region: required("S3_REGION")?,
            bucket: required("S3_BUCKET")?,
            access_key_id: Secret::new(required("S3_ACCESS_KEY_ID")?),
            secret_access_key: Secret::new(required("S3_SECRET_ACCESS_KEY")?),
        },
        cloudfront: CloudFrontSettings {
            domain,
            key_pair_id: required("CLOUDFRONT_KEY_PAIR_ID")?,
            private_key,
        },
    })
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
    use std::{collections::HashMap, sync::LazyLock};

    use p256::{
        ecdsa::SigningKey,
        elliptic_curve::Generate,
        pkcs8::{EncodePrivateKey, LineEnding},
    };

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
    fn recordings_default_to_local_directories() {
        let config = from(&[]).unwrap();
        assert_eq!(config.recordings.spool_dir, PathBuf::from("/recordings"));
        assert!(matches!(
            config.recordings.storage,
            RecordingStorageConfig::Local { ref store_dir } if store_dir == &PathBuf::from("/recording-store")
        ));
        let config = from(&[
            ("RECORDING_SPOOL_DIR", "/spool"),
            ("RECORDING_STORAGE", "local"),
            ("RECORDING_STORE_DIR", "/store"),
        ])
        .unwrap();
        assert_eq!(config.recordings.spool_dir, PathBuf::from("/spool"));
        assert!(matches!(
            config.recordings.storage,
            RecordingStorageConfig::Local { ref store_dir } if store_dir == &PathBuf::from("/store")
        ));
        for (var, value) in [
            ("RECORDING_SPOOL_DIR", ""),
            ("RECORDING_STORE_DIR", ""),
            ("RECORDING_STORAGE", "S3"),
        ] {
            assert!(from(&[(var, value)]).is_err(), "{var}={value:?}");
        }
    }

    #[test]
    fn a_recording_directory_that_is_not_utf8_says_so() {
        for var in ["RECORDING_SPOOL_DIR", "RECORDING_STORE_DIR"] {
            let error = Config::from_lookup(|key| match key {
                "DATABASE_URL" => Ok(URL.into()),
                "ALLOWED_ORIGINS" => Ok(ORIGIN.into()),
                "PUBLISH_JWT_PRIVATE_KEY" => Ok(KEY.into()),
                "MEDIA_HOOK_SECRET" => Ok(MEDIA_SECRET.into()),
                _ if key == var => Err(VarError::NotUnicode("\u{fffd}".into())),
                _ => Err(VarError::NotPresent),
            })
            .unwrap_err()
            .to_string();
            assert_eq!(error, format!("{var} must be valid UTF-8"));
        }
    }

    /// Compose passes every recording setting as `${VAR:-}`, so one left out
    /// of `.env` arrives empty.
    #[test]
    fn empty_recording_settings_mean_local_storage() {
        let empty = [
            "RECORDING_STORAGE",
            "S3_ENDPOINT",
            "S3_REGION",
            "S3_BUCKET",
            "S3_ACCESS_KEY_ID",
            "S3_SECRET_ACCESS_KEY",
            "CLOUDFRONT_DOMAIN",
            "CLOUDFRONT_KEY_PAIR_ID",
            "CLOUDFRONT_PRIVATE_KEY",
        ]
        .map(|var| (var, ""));
        let config = from(&empty).unwrap();
        assert!(matches!(
            config.recordings.storage,
            RecordingStorageConfig::Local { ref store_dir } if store_dir == &PathBuf::from("/recording-store")
        ));
    }

    const S3_SECRET: &str = "s3-secret-access-key-value";
    /// A fresh PKCS#8 key, so no private key sits in the repository.
    static PEM: LazyLock<String> = LazyLock::new(|| {
        SigningKey::generate()
            .to_pkcs8_pem(LineEnding::LF)
            .unwrap()
            .to_string()
    });

    /// The first base64 line of [`PEM`].
    fn pem_body() -> &'static str {
        PEM.lines().nth(1).unwrap()
    }

    fn s3(overrides: &[(&'static str, &'static str)]) -> Result<Config, ConfigError> {
        let mut vars = vec![
            ("RECORDING_STORAGE", "s3"),
            ("S3_ENDPOINT", "https://s3.ap-northeast-2.amazonaws.com"),
            ("S3_REGION", "ap-northeast-2"),
            ("S3_BUCKET", "lp-recordings"),
            ("S3_ACCESS_KEY_ID", "AKIDEXAMPLE"),
            ("S3_SECRET_ACCESS_KEY", S3_SECRET),
            ("CLOUDFRONT_DOMAIN", "d111111abcdef8.cloudfront.net"),
            ("CLOUDFRONT_KEY_PAIR_ID", "K2JCJMDEHXQW5F"),
            ("CLOUDFRONT_PRIVATE_KEY", PEM.as_str()),
        ];
        for (var, value) in overrides {
            vars.retain(|(v, _)| v != var);
            vars.push((var, value));
        }
        from(&vars)
    }

    #[test]
    fn s3_storage_takes_every_setting_and_never_logs_the_secrets() {
        let config = s3(&[]).unwrap();
        let RecordingStorageConfig::S3 { s3, cloudfront } = &config.recordings.storage else {
            panic!("{:?}", config.recordings.storage);
        };
        assert_eq!(s3.endpoint, "https://s3.ap-northeast-2.amazonaws.com");
        assert_eq!(s3.bucket, "lp-recordings");
        assert_eq!(s3.secret_access_key.expose(), S3_SECRET);
        assert_eq!(cloudfront.domain, "d111111abcdef8.cloudfront.net");
        let logged = format!("{config:?}");
        for secret in [S3_SECRET, "AKIDEXAMPLE", pem_body()] {
            assert!(!logged.contains(secret), "{secret} in {logged}");
        }
    }

    #[test]
    fn s3_storage_requires_every_setting() {
        for var in [
            "S3_ENDPOINT",
            "S3_REGION",
            "S3_BUCKET",
            "S3_ACCESS_KEY_ID",
            "S3_SECRET_ACCESS_KEY",
            "CLOUDFRONT_DOMAIN",
            "CLOUDFRONT_KEY_PAIR_ID",
            "CLOUDFRONT_PRIVATE_KEY",
        ] {
            assert!(
                matches!(s3(&[(var, "")]), Err(ConfigError::MissingS3Setting(v)) if v == var),
                "{var}"
            );
        }
    }

    #[test]
    fn s3_settings_are_checked_without_echoing_secrets() {
        for (var, bad) in [
            ("S3_ENDPOINT", "s3.amazonaws.com"),
            ("S3_ENDPOINT", "https://s3.amazonaws.com/"),
            ("S3_ENDPOINT", "https://key:hunter2@s3.amazonaws.com"),
            ("CLOUDFRONT_DOMAIN", "https://d1.cloudfront.net"),
            ("CLOUDFRONT_DOMAIN", "localhost"),
            ("CLOUDFRONT_PRIVATE_KEY", "hunter2"),
            (
                "CLOUDFRONT_PRIVATE_KEY",
                "-----BEGIN CERTIFICATE-----\nhunter2\n-----END CERTIFICATE-----",
            ),
        ] {
            let err = s3(&[(var, bad)]).unwrap_err();
            assert!(!err.to_string().contains("hunter2"), "{err}");
        }
    }

    const TOSS_CLIENT: &str = "test_ck_example_client_key";
    const TOSS_SECRET: &str = "test_sk_example_secret_key";

    #[test]
    fn donations_are_off_unless_both_toss_keys_are_set() {
        assert!(from(&[]).unwrap().payments.is_none());
        let empty = from(&[("TOSS_CLIENT_KEY", ""), ("TOSS_SECRET_KEY", "")]).unwrap();
        assert!(
            empty.payments.is_none(),
            "compose passes unset keys as empty"
        );

        let config = from(&[
            ("TOSS_CLIENT_KEY", TOSS_CLIENT),
            ("TOSS_SECRET_KEY", TOSS_SECRET),
        ])
        .unwrap();
        let toss = config.payments.as_ref().unwrap();
        assert_eq!(toss.client_key.expose(), TOSS_CLIENT);
        assert_eq!(toss.secret_key.expose(), TOSS_SECRET);
        let logged = format!("{config:?}");
        for key in [TOSS_CLIENT, TOSS_SECRET] {
            assert!(!logged.contains(key), "{key} in {logged}");
        }
    }

    #[test]
    fn toss_keys_are_required_together() {
        for (set, missing) in [
            ("TOSS_CLIENT_KEY", "TOSS_SECRET_KEY"),
            ("TOSS_SECRET_KEY", "TOSS_CLIENT_KEY"),
        ] {
            let err = from(&[(set, TOSS_SECRET)]).unwrap_err();
            assert!(
                matches!(err, ConfigError::MissingTossKey(var) if var == missing),
                "{err}"
            );
            assert!(!err.to_string().contains(TOSS_SECRET), "{err}");
        }
    }

    #[test]
    fn live_toss_keys_are_refused_without_echoing_them() {
        for (var, live) in [
            ("TOSS_CLIENT_KEY", "live_ck_x"),
            ("TOSS_SECRET_KEY", "live_sk_x"),
            ("TOSS_SECRET_KEY", "hunter2"),
            ("TOSS_SECRET_KEY", " test_sk_hunter2"),
        ] {
            let mut vars = vec![
                ("TOSS_CLIENT_KEY", TOSS_CLIENT),
                ("TOSS_SECRET_KEY", TOSS_SECRET),
            ];
            vars.retain(|(v, _)| *v != var);
            vars.push((var, live));
            let err = from(&vars).unwrap_err();
            assert!(
                matches!(err, ConfigError::TossKeyNotTest(v) if v == var),
                "{var}={live:?}: {err}"
            );
            assert_eq!(
                err.to_string(),
                format!("{var}: only Toss test keys (test_...) are accepted")
            );
        }
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
