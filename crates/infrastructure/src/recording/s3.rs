use std::{sync::Arc, time::Duration};

use application::{
    ports::{
        PortError,
        recording::{RecordingStorage, SpooledSegment},
    },
    recording::STORE_LEASE,
};
use async_trait::async_trait;
use domain::{recording::RecordingKey, secret::Secret};
use reqwest::{
    Url,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};

use super::FsSegmentSpool;

/// Where recordings go in an S3-compatible service. Static credentials only;
/// credentials from an instance role are not supported yet.
#[derive(Debug)]
pub struct S3Settings {
    /// Such as `https://s3.ap-northeast-2.amazonaws.com`.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    pub access_key_id: Secret<String>,
    pub secret_access_key: Secret<String>,
}

/// Uploads each spooled segment with one presigned `PUT`, streaming the file
/// so a segment of hundreds of megabytes never sits in memory.
pub struct S3RecordingStorage {
    spool: Arc<FsSegmentSpool>,
    bucket: Bucket,
    credentials: Credentials,
    http: reqwest::Client,
}

impl S3RecordingStorage {
    /// Addresses the bucket path-style (`<endpoint>/<bucket>/<key>`), which
    /// every S3-compatible service accepts.
    pub fn new(spool: Arc<FsSegmentSpool>, settings: S3Settings) -> Result<Self, PortError> {
        let endpoint = Url::parse(&settings.endpoint)
            .map_err(|err| PortError(format!("s3 endpoint: {err}")))?;
        let bucket = Bucket::new(endpoint, UrlStyle::Path, settings.bucket, settings.region)
            .map_err(|err| PortError(format!("s3 bucket: {err}")))?;
        let credentials = Credentials::new(
            settings.access_key_id.expose().clone(),
            settings.secret_access_key.expose().clone(),
        );
        // An upload that outlives its claim may be stored again by another
        // storer, so it is cut off at the lease.
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(STORE_LEASE)
            .build()
            .map_err(|err| PortError(format!("http client: {err}")))?;
        Ok(Self {
            spool,
            bucket,
            credentials,
            http,
        })
    }
}

#[async_trait]
impl RecordingStorage for S3RecordingStorage {
    async fn put(&self, segment: &SpooledSegment, key: &RecordingKey) -> Result<u64, PortError> {
        let error = |what: &str, err: &dyn std::fmt::Display| {
            PortError(format!(
                "upload {} as {}: {what}: {err}",
                segment.as_str(),
                key.as_str()
            ))
        };
        let file = tokio::fs::File::open(self.spool.file(segment))
            .await
            .map_err(|err| error("open", &err))?;
        let size = file
            .metadata()
            .await
            .map_err(|err| error("metadata", &err))?
            .len();
        let url = self
            .bucket
            .put_object(Some(&self.credentials), key.as_str())
            .sign(STORE_LEASE);
        let response = self
            .http
            .put(url)
            // Without it the streamed body goes chunked, which S3 refuses.
            .header(CONTENT_LENGTH, size)
            .header(CONTENT_TYPE, "video/mp4")
            .body(reqwest::Body::from(file))
            .send()
            .await
            // The URL carries the access key id and a live signature.
            .map_err(|err| error("request", &err.without_url()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(error("status", &status));
        }
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use domain::id::SnowflakeId;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    use super::*;

    const ACCESS_KEY_ID: &str = "AKIDEXAMPLE";
    const SECRET_ACCESS_KEY: &str = "test-only-secret-access-key";

    /// What the fake S3 received.
    #[derive(Debug)]
    struct Received {
        request_line: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Received {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }
    }

    /// Accepts one HTTP/1.1 request, answers `status` and reports it. Built
    /// on a bare socket so the test sees exactly what went over the wire.
    async fn fake_s3(status: u16) -> (String, oneshot::Receiver<Received>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let head_end = loop {
                let mut chunk = [0u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "connection closed before the headers ended");
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8(buffer[..head_end].to_vec()).unwrap();
            let mut lines = head.split("\r\n");
            let request_line = lines.next().unwrap().to_owned();
            let headers: Vec<(String, String)> = lines
                .filter_map(|line| line.split_once(": "))
                .map(|(n, v)| (n.to_owned(), v.to_owned()))
                .collect();
            let length: usize = headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                .map_or(0, |(_, v)| v.parse().unwrap());
            let mut body = buffer[head_end..].to_vec();
            while body.len() < length {
                let mut chunk = [0u8; 65536];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "connection closed before the body ended");
                body.extend_from_slice(&chunk[..read]);
            }
            socket
                .write_all(format!("HTTP/1.1 {status} X\r\ncontent-length: 0\r\n\r\n").as_bytes())
                .await
                .unwrap();
            let _ = tx.send(Received {
                request_line,
                headers,
                body,
            });
        });
        (endpoint, rx)
    }

    fn spool_with(bytes: &[u8]) -> Arc<FsSegmentSpool> {
        let mut suffix = [0u8; 8];
        getrandom::fill(&mut suffix).unwrap();
        let root: PathBuf =
            std::env::temp_dir().join(format!("lp-s3-{}", u64::from_le_bytes(suffix)));
        std::fs::create_dir_all(root.join("live/42")).unwrap();
        std::fs::write(root.join("live/42/a.mp4"), bytes).unwrap();
        Arc::new(FsSegmentSpool::new(root))
    }

    fn storage(spool: Arc<FsSegmentSpool>, endpoint: String) -> S3RecordingStorage {
        S3RecordingStorage::new(
            spool,
            S3Settings {
                endpoint,
                region: "ap-northeast-2".into(),
                bucket: "recordings-bucket".into(),
                access_key_id: Secret::new(ACCESS_KEY_ID.into()),
                secret_access_key: Secret::new(SECRET_ACCESS_KEY.into()),
            },
        )
        .unwrap()
    }

    fn key() -> RecordingKey {
        let id = |raw| SnowflakeId::try_from(raw).unwrap();
        RecordingKey::new(id(42), id(7))
    }

    #[tokio::test]
    async fn a_segment_is_put_presigned_with_its_bytes_and_length() {
        let bytes: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let (endpoint, received) = fake_s3(200).await;
        let storage = storage(spool_with(&bytes), endpoint);

        let size = storage
            .put(&SpooledSegment::new("live/42/a.mp4".into()), &key())
            .await
            .unwrap();

        let received = received.await.unwrap();
        assert_eq!(size, bytes.len() as u64);
        let target = received
            .request_line
            .strip_prefix("PUT /recordings-bucket/recordings/42/7.mp4?")
            .unwrap_or_else(|| panic!("{}", received.request_line));
        assert!(target.contains("X-Amz-Signature="), "{target}");
        assert!(
            target.contains(&format!("X-Amz-Credential={ACCESS_KEY_ID}%2F")),
            "{target}"
        );
        assert!(!target.contains(SECRET_ACCESS_KEY));
        assert_eq!(
            received.header("content-length"),
            Some(bytes.len().to_string().as_str())
        );
        assert_eq!(received.header("transfer-encoding"), None);
        assert_eq!(received.header("content-type"), Some("video/mp4"));
        assert!(received.body == bytes, "body differs from the file");
    }

    #[tokio::test]
    async fn a_refused_put_is_an_error_without_the_signature() {
        let (endpoint, _received) = fake_s3(403).await;
        let storage = storage(spool_with(b"x"), endpoint);

        let err = storage
            .put(&SpooledSegment::new("live/42/a.mp4".into()), &key())
            .await
            .unwrap_err();

        assert!(err.0.contains("403"), "{err}");
        assert!(!err.0.contains("X-Amz-Signature"), "{err}");
    }

    #[tokio::test]
    async fn an_unreachable_service_is_an_error_without_the_url() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let storage = storage(spool_with(b"x"), endpoint);

        let err = storage
            .put(&SpooledSegment::new("live/42/a.mp4".into()), &key())
            .await
            .unwrap_err();

        assert!(!err.0.contains(ACCESS_KEY_ID), "{err}");
        assert!(!err.0.contains("X-Amz"), "{err}");
    }
}
