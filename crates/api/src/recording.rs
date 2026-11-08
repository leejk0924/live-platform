//! Recordings: the use cases the handlers and the hook share, and the
//! adapters the API serves files through when recordings are kept locally.

use std::sync::Arc;

use application::{
    ports::{
        id::IdGenerator,
        media::IngestRouter,
        recording::{RecordingQuery, RecordingRepository, SegmentSpool, UrlSigner},
    },
    recording::{GetRecording, ListRecordings, RegisterSegment, ResolveReplay},
};
use infrastructure::recording::{FsRecordingStorage, LocalUrlSigner};
use tokio::sync::Notify;

/// Recordings kept on this host, which the API serves itself through URLs
/// it signed.
pub struct LocalRecordings {
    pub signer: Arc<LocalUrlSigner>,
    pub storage: Arc<FsRecordingStorage>,
}

pub struct RecordingPorts {
    pub repository: Arc<dyn RecordingRepository>,
    pub query: Arc<dyn RecordingQuery>,
    pub spool: Arc<dyn SegmentSpool>,
    pub signer: Arc<dyn UrlSigner>,
    /// `None` when another service serves the files.
    pub local: Option<LocalRecordings>,
}

pub struct RecordingUseCases {
    pub register: RegisterSegment,
    pub list: ListRecordings,
    pub get: GetRecording,
    pub replay: ResolveReplay,
    /// Wakes the storer after a segment is registered, so it need not wait
    /// for its next tick.
    pub stored_wake: Arc<Notify>,
    pub local: Option<LocalRecordings>,
}

impl RecordingUseCases {
    pub fn new(
        ports: RecordingPorts,
        ingest: Arc<dyn IngestRouter>,
        ids: Arc<dyn IdGenerator>,
    ) -> Self {
        let RecordingPorts {
            repository,
            query,
            spool,
            signer,
            local,
        } = ports;
        Self {
            register: RegisterSegment {
                ingest,
                spool,
                recordings: repository,
                ids,
            },
            list: ListRecordings {
                query: query.clone(),
            },
            get: GetRecording {
                query: query.clone(),
            },
            replay: ResolveReplay { query, signer },
            stored_wake: Arc::new(Notify::new()),
            local,
        }
    }
}
