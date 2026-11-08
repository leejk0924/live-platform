//! Recording adapters: the spool MediaMTX writes segments into, the stores
//! that keep them, and the URL signers that let viewers fetch them.

mod cloudfront;
mod local;
mod local_url;
mod s3;

pub use cloudfront::{CloudFrontKey, CloudFrontUrlSigner, InvalidCloudFrontKey};
pub use local::{FsRecordingStorage, FsSegmentSpool};
pub use local_url::LocalUrlSigner;
pub use s3::{S3RecordingStorage, S3Settings};
