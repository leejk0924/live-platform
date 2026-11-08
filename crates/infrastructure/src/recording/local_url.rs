use std::time::{Duration, SystemTime};

use application::ports::recording::UrlSigner;
use base64ct::{Base64UrlUnpadded, Encoding};
use domain::{
    recording::{RecordingId, RecordingKey},
    secret::Secret,
};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

/// Signs URLs to the API's own file route, for recordings kept by
/// [`super::FsRecordingStorage`]. The key lives only in this process, so a
/// restart invalidates every URL issued before it; the player asks for a new
/// one when playback fails.
pub struct LocalUrlSigner {
    key: Secret<[u8; 32]>,
}

impl LocalUrlSigner {
    pub fn generate() -> Self {
        let mut key = [0u8; 32];
        getrandom::fill(&mut key).expect("os rng is available");
        Self {
            key: Secret::new(key),
        }
    }

    fn mac(&self, recording: RecordingId, expires: u64) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.expose())
            .expect("HMAC takes a key of any length");
        mac.update(format!("{recording}.{expires}").as_bytes());
        mac
    }

    /// Whether `signature` is this signer's for `recording` until `expires`
    /// (Unix seconds), and `now` is before then. The comparison takes the
    /// same time however many bytes match.
    pub fn verify(
        &self,
        recording: RecordingId,
        expires: u64,
        signature: &str,
        now: SystemTime,
    ) -> bool {
        let unexpired = now < SystemTime::UNIX_EPOCH + Duration::from_secs(expires);
        let Ok(signature) = Base64UrlUnpadded::decode_vec(signature) else {
            return false;
        };
        let valid = self
            .mac(recording, expires)
            .verify_slice(&signature)
            .is_ok();
        unexpired && valid
    }
}

impl UrlSigner for LocalUrlSigner {
    fn sign(&self, recording: RecordingId, _: &RecordingKey, expires_at: SystemTime) -> String {
        let expires = expires_at
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let signature =
            Base64UrlUnpadded::encode_string(&self.mac(recording, expires).finalize().into_bytes());
        format!("/api/recordings/{recording}/file?expires={expires}&signature={signature}")
    }
}

#[cfg(test)]
mod tests {
    use domain::id::SnowflakeId;

    use super::*;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    /// Splits a signed URL into its recording, expiry and signature.
    fn parts(url: &str) -> (String, u64, String) {
        let rest = url.strip_prefix("/api/recordings/").unwrap();
        let (recording, query) = rest.split_once("/file?").unwrap();
        let (expires, signature) = query
            .strip_prefix("expires=")
            .unwrap()
            .split_once("&signature=")
            .unwrap();
        (
            recording.to_owned(),
            expires.parse().unwrap(),
            signature.to_owned(),
        )
    }

    fn key() -> RecordingKey {
        RecordingKey::new(id(42), id(7))
    }

    #[test]
    fn a_signed_url_verifies_until_it_expires() {
        let signer = LocalUrlSigner::generate();
        let url = signer.sign(id(7), &key(), at(1_000));

        let (recording, expires, signature) = parts(&url);
        assert_eq!(recording, "7");
        assert_eq!(expires, 1_000);
        assert!(signer.verify(id(7), expires, &signature, at(999)));
        assert!(!signer.verify(id(7), expires, &signature, at(1_000)));
    }

    #[test]
    fn a_url_changed_in_any_part_does_not_verify() {
        let signer = LocalUrlSigner::generate();
        let (_, expires, signature) = parts(&signer.sign(id(7), &key(), at(1_000)));
        let now = at(0);

        assert!(!signer.verify(id(8), expires, &signature, now), "recording");
        assert!(
            !signer.verify(id(7), expires + 1, &signature, now),
            "expiry"
        );
        let mut tampered = signature.clone().into_bytes();
        tampered[0] = if tampered[0] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert!(!signer.verify(id(7), expires, &tampered, now), "signature");
        assert!(!signer.verify(id(7), expires, "", now), "empty");
        assert!(
            !signer.verify(id(7), expires, "not base64!", now),
            "garbage"
        );
        assert!(
            !LocalUrlSigner::generate().verify(id(7), expires, &signature, now),
            "another process's key"
        );
    }
}
