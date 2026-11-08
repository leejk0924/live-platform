use std::{fmt, time::SystemTime};

use application::ports::recording::UrlSigner;
use base64ct::{Base64, Encoding};
use domain::{
    recording::{RecordingId, RecordingKey},
    secret::Secret,
};
use p256::{
    SecretKey,
    ecdsa::{Signature, SigningKey, signature::Signer},
    pkcs8::DecodePrivateKey,
};
use thiserror::Error;

#[derive(Debug, Error)]
#[error("CLOUDFRONT_PRIVATE_KEY must be an ECDSA P-256 private key in PEM")]
pub struct InvalidCloudFrontKey;

/// The ECDSA P-256 key of a CloudFront key pair. `Debug` never shows it.
/// Boxed so a config holding it stays small (clippy::large_enum_variant).
pub struct CloudFrontKey(Box<SigningKey>);

impl CloudFrontKey {
    /// Takes PKCS#8 (`BEGIN PRIVATE KEY`) or SEC1 (`BEGIN EC PRIVATE KEY`,
    /// as `openssl ecparam -genkey` prints it).
    pub fn from_pem(pem: &Secret<String>) -> Result<Self, InvalidCloudFrontKey> {
        let pem = pem.expose().trim();
        SigningKey::from_pkcs8_pem(pem)
            .or_else(|_| SecretKey::from_sec1_pem(pem).map(SigningKey::from))
            .map(|key| Self(Box::new(key)))
            .map_err(|_| InvalidCloudFrontKey)
    }
}

impl fmt::Debug for CloudFrontKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CloudFrontKey([REDACTED])")
    }
}

/// Signs CloudFront URLs with a canned policy, for recordings kept by
/// [`super::S3RecordingStorage`]. The URL names `Hash-Algorithm=SHA256`
/// because CloudFront assumes SHA-1 without it.
pub struct CloudFrontUrlSigner {
    /// Such as `d111111abcdef8.cloudfront.net`.
    domain: String,
    key_pair_id: String,
    key: CloudFrontKey,
}

impl CloudFrontUrlSigner {
    pub fn new(domain: String, key_pair_id: String, key: CloudFrontKey) -> Self {
        Self {
            domain,
            key_pair_id,
            key,
        }
    }
}

impl UrlSigner for CloudFrontUrlSigner {
    fn sign(&self, _: RecordingId, key: &RecordingKey, expires_at: SystemTime) -> String {
        let expires = expires_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let resource = format!("https://{}/{}", self.domain, key.as_str());
        let policy = canned_policy(&resource, expires);
        let signature: Signature = self.key.0.sign(policy.as_bytes());
        let signature = Base64::encode_string(signature.to_der().as_bytes())
            .replace('+', "-")
            .replace('=', "_")
            .replace('/', "~");
        format!(
            "{resource}?Expires={expires}&Signature={signature}&Key-Pair-Id={}&Hash-Algorithm=SHA256",
            self.key_pair_id
        )
    }
}

/// CloudFront hashes these exact bytes, so the layout must not change.
fn canned_policy(resource: &str, expires: u64) -> String {
    format!(
        r#"{{"Statement":[{{"Resource":"{resource}","Condition":{{"DateLessThan":{{"AWS:EpochTime":{expires}}}}}}}]}}"#
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use domain::id::SnowflakeId;
    use p256::{
        ecdsa::{VerifyingKey, signature::Verifier},
        elliptic_curve::Generate,
        pkcs8::{EncodePrivateKey, LineEnding},
    };

    use super::*;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    fn param<'a>(url: &'a str, name: &str) -> &'a str {
        let (_, query) = url.split_once('?').unwrap();
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
            .unwrap_or_else(|| panic!("{name} in {url}"))
    }

    /// Undoes CloudFront's character replacement, then base64.
    fn decode_signature(signature: &str) -> Signature {
        let standard = signature
            .replace('-', "+")
            .replace('_', "=")
            .replace('~', "/");
        Signature::from_der(&Base64::decode_vec(&standard).unwrap()).unwrap()
    }

    #[test]
    fn signs_the_canned_policy_with_sha256() {
        let signing = SigningKey::generate();
        let verifying = VerifyingKey::from(&signing);
        let signer = CloudFrontUrlSigner::new(
            "d111111abcdef8.cloudfront.net".into(),
            "K2JCJMDEHXQW5F".into(),
            CloudFrontKey(Box::new(signing)),
        );
        let key = RecordingKey::new(id(42), id(7));
        let expires_at = SystemTime::UNIX_EPOCH + Duration::from_secs(1_767_225_600);

        let url = signer.sign(id(7), &key, expires_at);

        let resource = "https://d111111abcdef8.cloudfront.net/recordings/42/7.mp4";
        assert!(url.starts_with(&format!("{resource}?")), "{url}");
        assert_eq!(param(&url, "Expires"), "1767225600");
        assert_eq!(param(&url, "Key-Pair-Id"), "K2JCJMDEHXQW5F");
        assert_eq!(param(&url, "Hash-Algorithm"), "SHA256");
        let signature = param(&url, "Signature");
        assert!(!signature.contains(['+', '=', '/']), "{signature}");
        let policy = format!(
            r#"{{"Statement":[{{"Resource":"{resource}","Condition":{{"DateLessThan":{{"AWS:EpochTime":1767225600}}}}}}]}}"#
        );
        assert_eq!(canned_policy(resource, 1_767_225_600), policy);
        verifying
            .verify(policy.as_bytes(), &decode_signature(signature))
            .expect("signature over the canned policy");
    }

    #[test]
    fn loads_pkcs8_and_sec1_pem() {
        let signing = SigningKey::generate();
        let pkcs8 = signing.to_pkcs8_pem(LineEnding::LF).unwrap();
        let sec1 = SecretKey::from(&signing)
            .to_sec1_pem(LineEnding::LF)
            .unwrap();
        assert!(sec1.contains("BEGIN EC PRIVATE KEY"));

        for pem in [pkcs8.to_string(), sec1.to_string()] {
            let loaded = CloudFrontKey::from_pem(&Secret::new(pem)).unwrap();
            assert_eq!(loaded.0.verifying_key(), signing.verifying_key());
        }
    }

    #[test]
    fn rejects_anything_else_without_echoing_it() {
        for bad in [
            "",
            "hunter2",
            "-----BEGIN CERTIFICATE-----\nhunter2\n-----END CERTIFICATE-----",
        ] {
            let err = CloudFrontKey::from_pem(&Secret::new(bad.into())).unwrap_err();
            assert!(!err.to_string().contains("hunter2"));
        }
        let key = CloudFrontKey(Box::new(SigningKey::generate()));
        assert_eq!(format!("{key:?}"), "CloudFrontKey([REDACTED])");
    }
}
