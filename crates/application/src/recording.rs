//! Recording finished segments, storing them for replay, and handing
//! viewers a short-lived URL to one. Anyone may browse and replay, signed in
//! or not, the same as live playback.

use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use domain::{
    broadcast::BroadcastId,
    recording::{
        MAX_SEGMENT_DURATION, PAGE_SIZE_DEFAULT, PAGE_SIZE_MAX, RecordingError, RecordingId,
        RecordingKey, RecordingStatus, StoreRetry,
    },
};
use thiserror::Error;

use crate::ports::{
    PortError,
    id::{IdGenerationError, IdGenerator},
    media::{IngestRouter, StreamPath},
    recording::{
        NewRecording, RecordingFilter, RecordingQuery, RecordingRepository, RecordingStorage,
        RecordingView, SegmentSpool, UrlSigner,
    },
};

/// How long a storer holds a claimed recording before another may take it:
/// longer than copying a 30-minute segment should ever take.
pub const STORE_LEASE: Duration = Duration::from_secs(10 * 60);

/// How long a replay URL works, whatever the recording's length. The player
/// asks for a new one when playback fails, so a URL that expires mid-replay
/// costs one request, and a deleted broadcast's CDN URLs die soon.
pub const REPLAY_URL_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Rejected(#[from] RecordingError),
    #[error(transparent)]
    Internal(#[from] PortError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    Registered {
        recording: RecordingId,
        broadcast: BroadcastId,
    },
    /// The path names no broadcast.
    UnknownPath,
    /// The reported file is not a segment in that path's spool directory.
    NotInSpool,
    /// The reported duration is above [`MAX_SEGMENT_DURATION`].
    TooLong,
    /// Its broadcast is unknown or deleted, or the segment was registered
    /// before.
    Ignored(BroadcastId),
}

/// Takes the media server's report that a segment file is finished. The
/// report is a hint: a duplicate or one about a deleted broadcast changes
/// nothing.
pub struct RegisterSegment {
    pub ingest: Arc<dyn IngestRouter>,
    pub spool: Arc<dyn SegmentSpool>,
    pub recordings: Arc<dyn RecordingRepository>,
    pub ids: Arc<dyn IdGenerator>,
}

impl RegisterSegment {
    pub async fn run(
        &self,
        path: &StreamPath,
        reported: &str,
        duration: Duration,
    ) -> Result<Registration, PortError> {
        let Some(broadcast) = self.ingest.broadcast_of(path) else {
            return Ok(Registration::UnknownPath);
        };
        let Some(source) = self.spool.segment(path, reported) else {
            return Ok(Registration::NotInSpool);
        };
        if duration > MAX_SEGMENT_DURATION {
            return Ok(Registration::TooLong);
        }
        let recording = self
            .ids
            .next_id()
            .map_err(|err: IdGenerationError| PortError(err.to_string()))?;
        let new = NewRecording {
            id: recording,
            broadcast,
            source,
            key: RecordingKey::new(broadcast, recording),
            duration,
        };
        Ok(if self.recordings.register(&new).await? {
            Registration::Registered {
                recording,
                broadcast,
            }
        } else {
            Registration::Ignored(broadcast)
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreOutcome {
    Stored {
        recording: RecordingId,
        size_bytes: u64,
        /// The spool copy could not be removed; the media server's own
        /// retention deletes it later.
        discard: Result<(), PortError>,
    },
    /// Stored, but the recording was no longer pending, so another storer
    /// whose lease had run out finished it first. The spool copy is left
    /// alone.
    Superseded(RecordingId),
    Retrying {
        recording: RecordingId,
        attempts: u32,
        after: Duration,
        error: PortError,
    },
    GaveUp {
        recording: RecordingId,
        error: PortError,
    },
}

/// Copies pending recordings from the spool to storage, one per call.
pub struct StoreRecordings {
    pub recordings: Arc<dyn RecordingRepository>,
    pub spool: Arc<dyn SegmentSpool>,
    pub storage: Arc<dyn RecordingStorage>,
}

impl StoreRecordings {
    /// Stores one due recording, or returns `None` when none is due. A crash
    /// at any point leaves the recording pending with its spool copy, so the
    /// next claim after [`STORE_LEASE`] stores it again over the same key.
    pub async fn run_once(&self) -> Result<Option<StoreOutcome>, PortError> {
        let Some(claimed) = self.recordings.claim(STORE_LEASE).await? else {
            return Ok(None);
        };
        let recording = claimed.id;
        let error = match self.storage.put(&claimed.source, &claimed.key).await {
            Ok(size_bytes) => {
                if !self
                    .recordings
                    .mark_stored(recording, claimed.attempts, size_bytes)
                    .await?
                {
                    return Ok(Some(StoreOutcome::Superseded(recording)));
                }
                return Ok(Some(StoreOutcome::Stored {
                    recording,
                    size_bytes,
                    discard: self.spool.discard(&claimed.source).await,
                }));
            }
            Err(error) => error,
        };
        let retry = StoreRetry::after_attempt(claimed.attempts);
        self.recordings
            .release_failed(recording, claimed.attempts, retry)
            .await?;
        Ok(Some(match retry {
            StoreRetry::After(after) => StoreOutcome::Retrying {
                recording,
                attempts: claimed.attempts,
                after,
                error,
            },
            StoreRetry::GiveUp => StoreOutcome::GaveUp { recording, error },
        }))
    }
}

pub struct GetRecording {
    pub query: Arc<dyn RecordingQuery>,
}

impl GetRecording {
    pub async fn run(&self, id: RecordingId) -> Result<RecordingView, Error> {
        self.query
            .get(id)
            .await?
            .ok_or_else(|| RecordingError::NotFound.into())
    }
}

#[derive(Debug)]
pub struct Page {
    pub items: Vec<RecordingView>,
    /// Pass as `before` to get the next page; `None` on the last page.
    pub next_before: Option<RecordingId>,
}

pub struct ListRecordings {
    pub query: Arc<dyn RecordingQuery>,
}

impl ListRecordings {
    pub async fn run(
        &self,
        filter: RecordingFilter,
        before: Option<RecordingId>,
        size: Option<u32>,
    ) -> Result<Page, Error> {
        let size = size.unwrap_or(PAGE_SIZE_DEFAULT);
        if !(1..=PAGE_SIZE_MAX).contains(&size) {
            return Err(RecordingError::InvalidPageSize.into());
        }
        // One extra row tells whether a next page exists.
        let mut items = self.query.list(filter, before, size + 1).await?;
        let has_more = items.len() > size as usize;
        items.truncate(size as usize);
        let next_before = if has_more {
            items.last().map(|view| view.id)
        } else {
            None
        };
        Ok(Page { items, next_before })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    pub url: String,
    pub expires_at: SystemTime,
}

pub struct ResolveReplay {
    pub query: Arc<dyn RecordingQuery>,
    pub signer: Arc<dyn UrlSigner>,
}

impl ResolveReplay {
    /// A URL to a ready recording that stays valid for [`REPLAY_URL_TTL`]
    /// from `now`.
    pub async fn run(&self, id: RecordingId, now: SystemTime) -> Result<Replay, Error> {
        let view = self.query.get(id).await?.ok_or(RecordingError::NotFound)?;
        if view.status != RecordingStatus::Ready {
            return Err(RecordingError::NotReady.into());
        }
        let expires_at = now + REPLAY_URL_TTL;
        Ok(Replay {
            url: self.signer.sign(id, &view.key, expires_at),
            expires_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use domain::{auth::Username, broadcast::BroadcastTitle, id::SnowflakeId};

    use super::*;
    use crate::ports::{
        media::IngestEndpoint,
        recording::{ClaimedRecording, SpooledSegment},
    };

    const BROADCAST: i64 = 7;
    const RECORDING: i64 = 100;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    fn path(raw: &str) -> StreamPath {
        StreamPath::new(raw.into())
    }

    struct Live;

    impl IngestRouter for Live {
        fn route(&self, broadcast: BroadcastId) -> IngestEndpoint {
            IngestEndpoint {
                path: path(&format!("live/{broadcast}")),
                whip_url: String::new(),
            }
        }

        fn broadcast_of(&self, path: &StreamPath) -> Option<BroadcastId> {
            path.as_str().strip_prefix("live/")?.parse().ok()
        }
    }

    /// Accepts `<path>/<name>.mp4` and records what it discarded.
    #[derive(Default)]
    struct Spool {
        discarded: Mutex<Vec<SpooledSegment>>,
        discard_fails: bool,
    }

    #[async_trait]
    impl SegmentSpool for Spool {
        fn segment(&self, path: &StreamPath, reported: &str) -> Option<SpooledSegment> {
            let name = reported.strip_prefix(&format!("/spool/{}/", path.as_str()))?;
            name.ends_with(".mp4")
                .then(|| SpooledSegment::new(format!("{}/{name}", path.as_str())))
        }

        async fn discard(&self, segment: &SpooledSegment) -> Result<(), PortError> {
            self.discarded.lock().unwrap().push(segment.clone());
            if self.discard_fails {
                return Err(PortError("permission denied".into()));
            }
            Ok(())
        }
    }

    struct Ids;

    impl IdGenerator for Ids {
        fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
            Ok(id(RECORDING))
        }
    }

    /// `accepts` is what `register` answers; `claimable` what every claim
    /// returns.
    #[derive(Default)]
    struct Recordings {
        accepts: bool,
        claimable: Option<ClaimedRecording>,
        still_pending: bool,
        stored: Mutex<Vec<(RecordingId, u32, u64)>>,
        released: Mutex<Vec<(RecordingId, u32, StoreRetry)>>,
        inserted: Mutex<Vec<NewRecording>>,
    }

    #[async_trait]
    impl RecordingRepository for Recordings {
        async fn register(&self, new: &NewRecording) -> Result<bool, PortError> {
            self.inserted.lock().unwrap().push(new.clone());
            Ok(self.accepts)
        }

        async fn claim(&self, lease: Duration) -> Result<Option<ClaimedRecording>, PortError> {
            assert_eq!(lease, STORE_LEASE);
            Ok(self.claimable.clone())
        }

        async fn mark_stored(
            &self,
            id: RecordingId,
            attempts: u32,
            size: u64,
        ) -> Result<bool, PortError> {
            self.stored.lock().unwrap().push((id, attempts, size));
            Ok(self.still_pending)
        }

        async fn release_failed(
            &self,
            id: RecordingId,
            attempts: u32,
            retry: StoreRetry,
        ) -> Result<bool, PortError> {
            self.released.lock().unwrap().push((id, attempts, retry));
            Ok(true)
        }
    }

    /// `Ok(size)` or the error every put returns.
    struct Storage(Result<u64, PortError>);

    #[async_trait]
    impl RecordingStorage for Storage {
        async fn put(&self, _: &SpooledSegment, key: &RecordingKey) -> Result<u64, PortError> {
            assert_eq!(key, &RecordingKey::new(id(BROADCAST), id(RECORDING)));
            self.0.clone()
        }
    }

    fn register(recordings: Arc<Recordings>) -> RegisterSegment {
        RegisterSegment {
            ingest: Arc::new(Live),
            spool: Arc::new(Spool::default()),
            recordings,
            ids: Arc::new(Ids),
        }
    }

    const SEGMENT: &str = "/spool/live/7/2026-11-08_19-32-15-610734.mp4";

    #[tokio::test]
    async fn a_finished_segment_is_registered_under_a_key_from_the_ids() {
        let recordings = Arc::new(Recordings {
            accepts: true,
            ..Recordings::default()
        });
        let registered = register(recordings.clone())
            .run(&path("live/7"), SEGMENT, Duration::from_millis(5_940))
            .await
            .unwrap();

        assert_eq!(
            registered,
            Registration::Registered {
                recording: id(RECORDING),
                broadcast: id(BROADCAST),
            }
        );
        assert_eq!(
            *recordings.inserted.lock().unwrap(),
            [NewRecording {
                id: id(RECORDING),
                broadcast: id(BROADCAST),
                source: SpooledSegment::new("live/7/2026-11-08_19-32-15-610734.mp4".into()),
                key: RecordingKey::new(id(BROADCAST), id(RECORDING)),
                duration: Duration::from_millis(5_940),
            }]
        );
    }

    #[tokio::test]
    async fn an_unknown_path_or_a_file_outside_the_spool_registers_nothing() {
        let recordings = Arc::new(Recordings {
            accepts: true,
            ..Recordings::default()
        });
        let use_case = register(recordings.clone());
        let ignored = [
            ("other/7", SEGMENT, Registration::UnknownPath),
            ("live/7", "/etc/passwd", Registration::NotInSpool),
            ("live/7", "/spool/live/8/a.mp4", Registration::NotInSpool),
        ];
        for (stream, reported, expected) in ignored {
            let outcome = use_case
                .run(&path(stream), reported, Duration::ZERO)
                .await
                .unwrap();
            assert_eq!(outcome, expected, "{stream} {reported}");
        }
        assert!(recordings.inserted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_segment_longer_than_any_real_one_is_refused() {
        let recordings = Arc::new(Recordings {
            accepts: true,
            ..Recordings::default()
        });
        let outcome = register(recordings.clone())
            .run(
                &path("live/7"),
                SEGMENT,
                Duration::from_secs(10_000_000_000),
            )
            .await
            .unwrap();
        assert_eq!(outcome, Registration::TooLong);
        assert!(recordings.inserted.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_segment_the_repository_refuses_is_ignored() {
        let recordings = Arc::new(Recordings::default());
        let outcome = register(recordings)
            .run(&path("live/7"), SEGMENT, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(outcome, Registration::Ignored(id(BROADCAST)));
    }

    fn claimed(attempts: u32) -> ClaimedRecording {
        ClaimedRecording {
            id: id(RECORDING),
            broadcast: id(BROADCAST),
            source: SpooledSegment::new("live/7/a.mp4".into()),
            key: RecordingKey::new(id(BROADCAST), id(RECORDING)),
            attempts,
        }
    }

    async fn store(
        recordings: Arc<Recordings>,
        spool: Arc<Spool>,
        put: Result<u64, PortError>,
    ) -> Option<StoreOutcome> {
        StoreRecordings {
            recordings,
            spool,
            storage: Arc::new(Storage(put)),
        }
        .run_once()
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn nothing_due_is_none() {
        let outcome = store(
            Arc::new(Recordings::default()),
            Arc::new(Spool::default()),
            Ok(1),
        )
        .await;
        assert_eq!(outcome, None);
    }

    #[tokio::test]
    async fn a_stored_recording_is_marked_ready_then_leaves_the_spool() {
        let recordings = Arc::new(Recordings {
            claimable: Some(claimed(1)),
            still_pending: true,
            ..Recordings::default()
        });
        let spool = Arc::new(Spool::default());
        let outcome = store(recordings.clone(), spool.clone(), Ok(4_096)).await;

        assert_eq!(
            outcome,
            Some(StoreOutcome::Stored {
                recording: id(RECORDING),
                size_bytes: 4_096,
                discard: Ok(()),
            })
        );
        assert_eq!(
            *recordings.stored.lock().unwrap(),
            [(id(RECORDING), 1, 4_096)]
        );
        assert_eq!(*spool.discarded.lock().unwrap(), [claimed(1).source]);
        assert!(recordings.released.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_discard_is_reported_and_the_recording_stays_ready() {
        let recordings = Arc::new(Recordings {
            claimable: Some(claimed(1)),
            still_pending: true,
            ..Recordings::default()
        });
        let spool = Arc::new(Spool {
            discard_fails: true,
            ..Spool::default()
        });
        let outcome = store(recordings.clone(), spool, Ok(1)).await;

        assert!(
            matches!(
                outcome,
                Some(StoreOutcome::Stored {
                    discard: Err(_),
                    ..
                })
            ),
            "{outcome:?}"
        );
        assert!(recordings.released.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_recording_another_storer_finished_keeps_its_spool_copy() {
        let recordings = Arc::new(Recordings {
            claimable: Some(claimed(2)),
            still_pending: false,
            ..Recordings::default()
        });
        let spool = Arc::new(Spool::default());
        let outcome = store(recordings, spool.clone(), Ok(1)).await;

        assert_eq!(outcome, Some(StoreOutcome::Superseded(id(RECORDING))));
        assert!(spool.discarded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_failed_put_backs_off_and_the_last_attempt_gives_up() {
        let error = PortError("storage unreachable".into());
        for (attempts, retry) in [
            (1, StoreRetry::after_attempt(1)),
            (domain::recording::MAX_STORE_ATTEMPTS, StoreRetry::GiveUp),
        ] {
            let recordings = Arc::new(Recordings {
                claimable: Some(claimed(attempts)),
                ..Recordings::default()
            });
            let spool = Arc::new(Spool::default());
            let outcome = store(recordings.clone(), spool.clone(), Err(error.clone())).await;

            let expected = match retry {
                StoreRetry::After(after) => StoreOutcome::Retrying {
                    recording: id(RECORDING),
                    attempts,
                    after,
                    error: error.clone(),
                },
                StoreRetry::GiveUp => StoreOutcome::GaveUp {
                    recording: id(RECORDING),
                    error: error.clone(),
                },
            };
            assert_eq!(outcome, Some(expected), "attempt {attempts}");
            assert_eq!(
                *recordings.released.lock().unwrap(),
                [(id(RECORDING), attempts, retry)]
            );
            assert!(recordings.stored.lock().unwrap().is_empty());
            assert!(spool.discarded.lock().unwrap().is_empty());
        }
    }

    fn view(raw_id: i64, status: RecordingStatus) -> RecordingView {
        RecordingView {
            id: id(raw_id),
            broadcast_id: id(BROADCAST),
            broadcast_title: BroadcastTitle::parse("t").unwrap(),
            owner_name: Username::parse("alice").unwrap(),
            status,
            duration: Duration::from_secs(600),
            size_bytes: (status == RecordingStatus::Ready).then_some(1),
            recorded_at: SystemTime::UNIX_EPOCH,
            key: RecordingKey::new(id(BROADCAST), id(raw_id)),
        }
    }

    /// Holds ids 1..=total in `status` and answers like the real query.
    struct Stored(i64, RecordingStatus);

    #[async_trait]
    impl RecordingQuery for Stored {
        async fn list(
            &self,
            _: RecordingFilter,
            before: Option<RecordingId>,
            limit: u32,
        ) -> Result<Vec<RecordingView>, PortError> {
            let top = before.map_or(self.0, |b| b.as_i64() - 1);
            Ok((1..=top)
                .rev()
                .take(limit as usize)
                .map(|raw| view(raw, self.1))
                .collect())
        }

        async fn get(&self, wanted: RecordingId) -> Result<Option<RecordingView>, PortError> {
            Ok((1..=self.0)
                .contains(&wanted.as_i64())
                .then(|| view(wanted.as_i64(), self.1)))
        }
    }

    #[tokio::test]
    async fn pages_link_to_the_next_until_the_last() {
        let list = ListRecordings {
            query: Arc::new(Stored(3, RecordingStatus::Ready)),
        };
        let first = list.run(RecordingFilter::All, None, Some(2)).await.unwrap();
        assert_eq!(first.items.len(), 2);
        assert_eq!(first.next_before, Some(id(2)));
        let last = list
            .run(RecordingFilter::All, first.next_before, Some(2))
            .await
            .unwrap();
        assert_eq!(last.items.len(), 1);
        assert_eq!(last.next_before, None);
        for bad in [0, PAGE_SIZE_MAX + 1] {
            assert!(matches!(
                list.run(RecordingFilter::All, None, Some(bad)).await,
                Err(Error::Rejected(RecordingError::InvalidPageSize))
            ));
        }
    }

    /// Writes the expiry it was asked for into the URL.
    struct Signer;

    impl UrlSigner for Signer {
        fn sign(
            &self,
            recording: RecordingId,
            key: &RecordingKey,
            expires_at: SystemTime,
        ) -> String {
            let secs = expires_at
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            format!("{recording}:{}:{secs}", key.as_str())
        }
    }

    fn replay(status: RecordingStatus) -> ResolveReplay {
        ResolveReplay {
            query: Arc::new(Stored(3, status)),
            signer: Arc::new(Signer),
        }
    }

    #[tokio::test]
    async fn a_replay_url_lasts_five_minutes_whatever_the_recording_length() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let replay = replay(RecordingStatus::Ready)
            .run(id(2), now)
            .await
            .unwrap();

        assert_eq!(replay.expires_at, now + Duration::from_secs(5 * 60));
        assert_eq!(replay.url, "2:recordings/7/2.mp4:1300");
    }

    #[tokio::test]
    async fn only_a_ready_recording_can_be_replayed() {
        let now = SystemTime::UNIX_EPOCH;
        for status in [RecordingStatus::Pending, RecordingStatus::Failed] {
            assert!(
                matches!(
                    replay(status).run(id(2), now).await,
                    Err(Error::Rejected(RecordingError::NotReady))
                ),
                "{status:?}"
            );
        }
        assert!(matches!(
            replay(RecordingStatus::Ready).run(id(9), now).await,
            Err(Error::Rejected(RecordingError::NotFound))
        ));
    }
}
