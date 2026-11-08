//! Ed25519-signed JWTs that MediaMTX verifies against the public key the API
//! serves as a JWKS.

use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use application::ports::{
    PortError,
    media::{PublishAuthorizer, PublishCredential, StreamPath},
};
use async_trait::async_trait;
use base64ct::{Base64, Base64UrlUnpadded, Encoding};
use domain::{auth::UserId, secret::Secret};
use ed25519_dalek::{Signer, SigningKey};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

/// Must equal `authJWTIssuer` in `deploy/mediamtx/mediamtx.yml`.
pub const ISSUER: &str = "live-platform";

/// MediaMTX disconnects a WebRTC publisher when its token's `exp` passes, so
/// this is how long one publish session can last, not only how long the token
/// may wait before use.
pub const PUBLISH_TOKEN_TTL: Duration = Duration::from_secs(12 * 60 * 60);

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

    /// A fresh random key, for tests.
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

    /// A short token for MediaMTX's Control API, which sits behind the same
    /// JWT check as publishing.
    pub(super) fn api_token(&self) -> Result<String, PortError> {
        self.sign(
            "live-platform-api".into(),
            Permission {
                action: "api",
                path: None,
            },
            API_TOKEN_TTL,
        )
    }
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

/// Issues publish tokens that MediaMTX checks itself, so publishing needs no
/// call back to the API.
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
    fn api_token_grants_only_the_control_api() {
        let key = PublishKey::generate();
        let (_, claims, _, _) = parts(&key.api_token().unwrap());
        assert_eq!(claims["mediamtx_permissions"], json!([{"action": "api"}]));
        let lifetime = claims["exp"].as_u64().unwrap() - claims["iat"].as_u64().unwrap();
        assert_eq!(lifetime, API_TOKEN_TTL.as_secs());
    }
}
