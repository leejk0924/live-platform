//! Ed25519-signed JWTs. The API issues them and checks them again when
//! MediaMTX asks whether to admit someone; the public key is also served as a
//! JWKS for anyone else who wants to verify them.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use application::ports::{
    PortError,
    media::{
        MediaCredentialVerifier, PublishAuthorizer, PublishCredential, PublishTicket, StreamPath,
        TicketId,
    },
};
use async_trait::async_trait;
use base64ct::{Base64, Base64UrlUnpadded, Encoding};
use domain::{auth::UserId, secret::Secret};
use ed25519_dalek::{Signature, Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

pub const ISSUER: &str = "live-platform";

/// How long a publish token may wait before its one use. Once admitted, the
/// publish session lasts until the publisher leaves or is kicked.
pub const PUBLISH_TOKEN_TTL: Duration = Duration::from_secs(60);

/// The subject of the API's own Control API tokens.
const CONTROL_SUBJECT: &str = "live-platform-api";

/// Longer ids are not ours; the bound also keeps the ledger column small.
const MAX_JTI_LEN: usize = 64;

/// Long enough for one Control API call.
const API_TOKEN_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Error)]
#[error("PUBLISH_JWT_PRIVATE_KEY must be 32 random bytes in standard base64")]
pub struct InvalidPublishKey;

/// The Ed25519 key that signs publish and Control API tokens. `Debug` shows
/// only the key id.
pub struct PublishKey {
    signing: SigningKey,
    kid: String,
    x: String,
}

/// The public half as RFC 8037 parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicJwk {
    /// RFC 7638 thumbprint of the key.
    pub kid: String,
    /// The public key, base64url.
    pub x: String,
}

impl PublishKey {
    /// `raw` is a 32-byte Ed25519 seed in standard base64, as printed by
    /// `openssl rand -base64 32`.
    pub fn from_base64_seed(raw: &Secret<String>) -> Result<Self, InvalidPublishKey> {
        let mut seed = Zeroizing::new([0u8; 32]);
        let decoded =
            Base64::decode(raw.expose().trim(), seed.as_mut()).map_err(|_| InvalidPublishKey)?;
        if decoded.len() != seed.len() {
            return Err(InvalidPublishKey);
        }
        Ok(Self::from_seed(&seed))
    }

    /// A new key from the OS random number generator.
    pub fn generate() -> Self {
        let mut seed = Zeroizing::new([0u8; 32]);
        getrandom::fill(seed.as_mut()).expect("os rng is available");
        Self::from_seed(&seed)
    }

    fn from_seed(seed: &[u8; 32]) -> Self {
        let signing = SigningKey::from_bytes(seed);
        let x = Base64UrlUnpadded::encode_string(signing.verifying_key().as_bytes());
        // RFC 7638: the required members in lexicographic order, no spaces.
        let canonical = format!(r#"{{"crv":"Ed25519","kty":"OKP","x":"{x}"}}"#);
        let kid = Base64UrlUnpadded::encode_string(&Sha256::digest(canonical.as_bytes()));
        Self { signing, kid, x }
    }

    pub fn public_jwk(&self) -> PublicJwk {
        PublicJwk {
            kid: self.kid.clone(),
            x: self.x.clone(),
        }
    }

    /// A token for `subject` that grants `permission` until `ttl` from now.
    fn sign(
        &self,
        subject: String,
        permission: Permission<'_>,
        ttl: Duration,
    ) -> Result<String, PortError> {
        let iat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| PortError(format!("clock before 1970: {err}")))?
            .as_secs();
        let mut jti = [0u8; 16];
        getrandom::fill(&mut jti).map_err(|err| PortError(format!("os rng failed: {err}")))?;
        let header = Header {
            alg: "EdDSA",
            typ: "JWT",
            kid: &self.kid,
        };
        let claims = Claims {
            iss: ISSUER,
            sub: subject,
            iat,
            exp: iat + ttl.as_secs(),
            jti: Base64UrlUnpadded::encode_string(&jti),
            permissions: [permission],
        };
        let signing_input = format!("{}.{}", segment(&header)?, segment(&claims)?);
        let signature = self.signing.sign(signing_input.as_bytes());
        Ok(format!(
            "{signing_input}.{}",
            Base64UrlUnpadded::encode_string(&signature.to_bytes())
        ))
    }

    /// A short token for MediaMTX's Control API. MediaMTX passes it back to
    /// the API to be checked like a publish token.
    pub fn control_token(&self) -> Result<String, PortError> {
        self.sign(
            CONTROL_SUBJECT.into(),
            Permission {
                action: "api",
                path: None,
            },
            API_TOKEN_TTL,
        )
    }
}

impl PublishKey {
    /// The claims of `token` if this key signed it, it names this issuer and
    /// it has not expired. Signature and header are checked before any claim
    /// is trusted.
    fn verify(&self, token: &str) -> Option<VerifiedClaims> {
        let (signing_input, signature) = token.rsplit_once('.')?;
        let (header, claims) = signing_input.split_once('.')?;
        if claims.contains('.') {
            return None;
        }
        let header: ReceivedHeader = serde_json::from_slice(&decode(header)?).ok()?;
        if header.alg != "EdDSA" || header.kid != self.kid {
            return None;
        }
        let signature = Signature::from_slice(&decode(signature)?).ok()?;
        self.signing
            .verifying_key()
            .verify_strict(signing_input.as_bytes(), &signature)
            .ok()?;
        let claims: ReceivedClaims = serde_json::from_slice(&decode(claims)?).ok()?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let valid = claims.iss == ISSUER
            && claims.exp > now
            && !claims.jti.is_empty()
            && claims.jti.len() <= MAX_JTI_LEN;
        let [permission] = claims.permissions;
        valid.then_some(VerifiedClaims {
            sub: claims.sub,
            exp: claims.exp,
            jti: claims.jti,
            action: permission.action,
            path: permission.path,
        })
    }
}

fn decode(segment: &str) -> Option<Vec<u8>> {
    Base64UrlUnpadded::decode_vec(segment).ok()
}

#[derive(Deserialize)]
struct ReceivedHeader {
    alg: String,
    kid: String,
}

#[derive(Deserialize)]
struct ReceivedClaims {
    iss: String,
    sub: String,
    exp: u64,
    jti: String,
    #[serde(rename = "mediamtx_permissions")]
    permissions: [ReceivedPermission; 1],
}

#[derive(Deserialize)]
struct ReceivedPermission {
    action: String,
    path: Option<String>,
}

struct VerifiedClaims {
    sub: String,
    exp: u64,
    jti: String,
    action: String,
    path: Option<String>,
}

impl fmt::Debug for PublishKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PublishKey")
            .field("kid", &self.kid)
            .finish_non_exhaustive()
    }
}

fn segment(value: &impl Serialize) -> Result<String, PortError> {
    let json = serde_json::to_vec(value).map_err(|err| PortError(err.to_string()))?;
    Ok(Base64UrlUnpadded::encode_string(&json))
}

#[derive(Serialize)]
struct Header<'a> {
    alg: &'static str,
    typ: &'static str,
    kid: &'a str,
}

#[derive(Serialize)]
struct Claims<'a> {
    iss: &'static str,
    sub: String,
    iat: u64,
    exp: u64,
    jti: String,
    /// MediaMTX's `authJWTClaimKey`.
    #[serde(rename = "mediamtx_permissions")]
    permissions: [Permission<'a>; 1],
}

#[derive(Serialize)]
struct Permission<'a> {
    action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<&'a str>,
}

/// Issues publish tokens, and checks them and the API's own Control API
/// tokens when MediaMTX asks.
pub struct JwtPublishAuthorizer {
    key: Arc<PublishKey>,
}

impl JwtPublishAuthorizer {
    pub fn new(key: Arc<PublishKey>) -> Self {
        Self { key }
    }
}

#[async_trait]
impl PublishAuthorizer for JwtPublishAuthorizer {
    async fn authorize(
        &self,
        publisher: UserId,
        path: &StreamPath,
    ) -> Result<PublishCredential, PortError> {
        let token = self.key.sign(
            publisher.to_string(),
            Permission {
                action: "publish",
                path: Some(path.as_str()),
            },
            PUBLISH_TOKEN_TTL,
        )?;
        Ok(PublishCredential::Bearer {
            token: Secret::new(token),
            expires_in: PUBLISH_TOKEN_TTL,
        })
    }
}

impl MediaCredentialVerifier for JwtPublishAuthorizer {
    fn publish_ticket(&self, token: &Secret<String>) -> Option<PublishTicket> {
        let claims = self.key.verify(token.expose())?;
        if claims.action != "publish" {
            return None;
        }
        Some(PublishTicket {
            id: TicketId::new(claims.jti),
            publisher: claims.sub.parse().ok()?,
            path: StreamPath::new(claims.path?),
            expires_at: UNIX_EPOCH + Duration::from_secs(claims.exp),
        })
    }

    fn is_control_credential(&self, token: &Secret<String>) -> bool {
        self.key.verify(token.expose()).is_some_and(|claims| {
            claims.action == "api" && claims.path.is_none() && claims.sub == CONTROL_SUBJECT
        })
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signature, VerifyingKey};
    use serde_json::{Value, json};

    use super::*;

    fn decode(segment: &str) -> Vec<u8> {
        Base64UrlUnpadded::decode_vec(segment).unwrap()
    }

    fn parts(token: &str) -> (Value, Value, Signature, String) {
        let pieces: Vec<&str> = token.split('.').collect();
        assert_eq!(pieces.len(), 3, "{token}");
        let header = serde_json::from_slice(&decode(pieces[0])).unwrap();
        let claims = serde_json::from_slice(&decode(pieces[1])).unwrap();
        let signature = Signature::from_slice(&decode(pieces[2])).unwrap();
        (
            header,
            claims,
            signature,
            format!("{}.{}", pieces[0], pieces[1]),
        )
    }

    fn verifying_key(jwk: &PublicJwk) -> VerifyingKey {
        let bytes: [u8; 32] = decode(&jwk.x).try_into().unwrap();
        VerifyingKey::from_bytes(&bytes).unwrap()
    }

    #[test]
    fn kid_is_the_rfc_7638_thumbprint() {
        // RFC 8037 appendix A.1/A.3: this seed's public key and thumbprint.
        let seed = Secret::new("nWGxne/9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A=".to_owned());
        let jwk = PublishKey::from_base64_seed(&seed).unwrap().public_jwk();
        assert_eq!(jwk.x, "11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo");
        assert_eq!(jwk.kid, "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k");
    }

    #[test]
    fn the_seed_must_be_32_bytes_of_base64() {
        for bad in [
            "",
            "change-me",
            "AAAA",
            &Base64::encode_string(&[7u8; 31]),
            &Base64::encode_string(&[7u8; 33]),
        ] {
            assert!(
                PublishKey::from_base64_seed(&Secret::new(bad.to_owned())).is_err(),
                "{bad:?}"
            );
        }
        let ok = Secret::new(format!(" {}\n", Base64::encode_string(&[7u8; 32])));
        assert!(PublishKey::from_base64_seed(&ok).is_ok());
    }

    #[test]
    fn debug_shows_only_the_key_id() {
        let seed = Base64::encode_string(&[9u8; 32]);
        let key = PublishKey::from_base64_seed(&Secret::new(seed.clone())).unwrap();
        let shown = format!("{key:?}");
        assert!(shown.contains(&key.public_jwk().kid), "{shown}");
        assert!(!shown.contains(&seed), "{shown}");
    }

    #[tokio::test]
    async fn publish_token_is_signed_and_scoped_to_one_path() {
        let key = Arc::new(PublishKey::generate());
        let authorizer = JwtPublishAuthorizer::new(key.clone());
        let path = StreamPath::new("live/42".into());
        let publisher = UserId::try_from(7).unwrap();

        let PublishCredential::Bearer { token, expires_in } =
            authorizer.authorize(publisher, &path).await.unwrap();
        let (header, claims, signature, signed) = parts(token.expose());

        let jwk = key.public_jwk();
        verifying_key(&jwk)
            .verify_strict(signed.as_bytes(), &signature)
            .unwrap();
        assert_eq!(
            header,
            json!({"alg": "EdDSA", "typ": "JWT", "kid": jwk.kid})
        );
        assert_eq!(claims["iss"], ISSUER);
        assert_eq!(claims["sub"], "7");
        assert_eq!(
            claims["mediamtx_permissions"],
            json!([{"action": "publish", "path": "live/42"}])
        );
        let lifetime = claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap();
        assert_eq!(lifetime, PUBLISH_TOKEN_TTL.as_secs());
        assert_eq!(expires_in, PUBLISH_TOKEN_TTL);
        assert_eq!(decode(claims["jti"].as_str().unwrap()).len(), 16);

        let PublishCredential::Bearer { token: again, .. } =
            authorizer.authorize(publisher, &path).await.unwrap();
        assert_ne!(parts(again.expose()).1["jti"], claims["jti"]);
    }

    #[test]
    fn control_token_grants_only_the_control_api() {
        let key = PublishKey::generate();
        let (_, claims, _, _) = parts(&key.control_token().unwrap());
        assert_eq!(claims["mediamtx_permissions"], json!([{"action": "api"}]));
        let lifetime = claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap();
        assert_eq!(lifetime, API_TOKEN_TTL.as_secs());
    }

    /// A token signed by `key` with exactly this header and these claims.
    fn forge(key: &PublishKey, header: Value, claims: Value) -> Secret<String> {
        let input = format!(
            "{}.{}",
            Base64UrlUnpadded::encode_string(header.to_string().as_bytes()),
            Base64UrlUnpadded::encode_string(claims.to_string().as_bytes())
        );
        let signature = key.signing.sign(input.as_bytes());
        Secret::new(format!(
            "{input}.{}",
            Base64UrlUnpadded::encode_string(&signature.to_bytes())
        ))
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn valid_claims() -> Value {
        json!({
            "iss": ISSUER, "sub": "7", "iat": now(), "exp": now() + 60, "jti": "abc",
            "mediamtx_permissions": [{"action": "publish", "path": "live/42"}],
        })
    }

    #[tokio::test]
    async fn an_issued_publish_token_is_a_ticket_for_its_path_and_publisher() {
        let key = Arc::new(PublishKey::generate());
        let authorizer = JwtPublishAuthorizer::new(key.clone());
        let path = StreamPath::new("live/42".into());
        let PublishCredential::Bearer { token, .. } = authorizer
            .authorize(UserId::try_from(7).unwrap(), &path)
            .await
            .unwrap();
        let (_, claims, _, _) = parts(token.expose());

        let ticket = authorizer.publish_ticket(&token).expect("valid ticket");
        assert_eq!(ticket.path, path);
        assert_eq!(ticket.publisher, UserId::try_from(7).unwrap());
        assert_eq!(ticket.id.as_str(), claims["jti"].as_str().unwrap());
        assert_eq!(
            ticket.expires_at,
            UNIX_EPOCH + Duration::from_secs(claims["exp"].as_u64().unwrap())
        );
        assert!(!authorizer.is_control_credential(&token));
    }

    #[test]
    fn the_control_token_is_only_a_control_credential() {
        let key = Arc::new(PublishKey::generate());
        let authorizer = JwtPublishAuthorizer::new(key.clone());
        let control = Secret::new(key.control_token().unwrap());
        assert!(authorizer.is_control_credential(&control));
        assert!(authorizer.publish_ticket(&control).is_none());
    }

    #[test]
    fn tokens_that_were_not_issued_here_or_are_stale_are_refused() {
        let key = PublishKey::generate();
        let authorizer = JwtPublishAuthorizer::new(Arc::new(PublishKey::generate()));
        let ours =
            JwtPublishAuthorizer::new(Arc::new(PublishKey::from_seed(&key.signing.to_bytes())));
        let header = json!({"alg": "EdDSA", "typ": "JWT", "kid": key.kid});
        assert!(
            ours.publish_ticket(&forge(&key, header.clone(), valid_claims()))
                .is_some(),
            "the baseline forgery is valid"
        );
        assert!(
            authorizer
                .publish_ticket(&forge(&key, header.clone(), valid_claims()))
                .is_none(),
            "signed by another key"
        );

        let with = |field: &str, value: Value| {
            let mut claims = valid_claims();
            claims[field] = value;
            forge(&key, header.clone(), claims)
        };
        for (what, token) in [
            ("expired", with("exp", json!(now()))),
            ("other issuer", with("iss", json!("someone-else"))),
            ("empty jti", with("jti", json!(""))),
            ("long jti", with("jti", json!("x".repeat(65)))),
            ("subject not a user id", with("sub", json!("alice"))),
            (
                "two permissions",
                with(
                    "mediamtx_permissions",
                    json!([{"action": "publish", "path": "live/42"}, {"action": "api"}]),
                ),
            ),
            (
                "no path",
                with("mediamtx_permissions", json!([{"action": "publish"}])),
            ),
            (
                "read permission",
                with(
                    "mediamtx_permissions",
                    json!([{"action": "read", "path": "live/42"}]),
                ),
            ),
            (
                "other kid",
                forge(&key, json!({"alg": "EdDSA", "kid": "nope"}), valid_claims()),
            ),
            (
                "alg none",
                forge(&key, json!({"alg": "none", "kid": key.kid}), valid_claims()),
            ),
        ] {
            assert!(ours.publish_ticket(&token).is_none(), "{what}");
        }

        let valid = forge(&key, header, valid_claims());
        let (input, signature) = valid.expose().rsplit_once('.').unwrap();
        let mut flipped = decode(signature);
        flipped[0] ^= 1;
        for (what, token) in [
            (
                "tampered signature",
                format!("{input}.{}", Base64UrlUnpadded::encode_string(&flipped)),
            ),
            ("extra segment", format!("{}.x", valid.expose())),
            ("two segments", input.to_owned()),
            ("garbage", "a.b.c".to_owned()),
            ("empty", String::new()),
        ] {
            assert!(ours.publish_ticket(&Secret::new(token)).is_none(), "{what}");
        }
    }
}
