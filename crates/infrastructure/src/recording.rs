//! Recording adapters: the spool MediaMTX writes segments into, the stores
//! that keep them, and the URL signers that let viewers fetch them.

mod local;
mod s3;

pub use local::{FsRecordingStorage, FsSegmentSpool};
pub use s3::{S3RecordingStorage, S3Settings};
