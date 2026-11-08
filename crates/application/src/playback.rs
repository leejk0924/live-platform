//! Watching a broadcast: where a viewer fetches its stream. Anyone may ask,
//! signed in or not.

use std::sync::Arc;

use domain::broadcast::{BroadcastError, BroadcastId};

use crate::{
    broadcast::Error,
    ports::{
        broadcast::BroadcastQuery,
        media::{IngestRouter, PlaybackResolver, PlaybackSource},
    },
};

pub struct ResolvePlayback {
    pub query: Arc<dyn BroadcastQuery>,
    pub ingest: Arc<dyn IngestRouter>,
    pub resolver: Arc<dyn PlaybackResolver>,
}

impl ResolvePlayback {
    /// The sources of a `LIVE` or `INTERRUPTED` broadcast, preferred first.
    /// An interrupted one keeps its sources so players can wait out the
    /// grace period instead of giving up.
    pub async fn run(&self, id: BroadcastId) -> Result<Vec<PlaybackSource>, Error> {
        let view = self.query.get(id).await?.ok_or(BroadcastError::NotFound)?;
        if !view.broadcast.status.is_active() {
            return Err(BroadcastError::NotLive.into());
        }
        Ok(self.resolver.resolve(&self.ingest.route(id).path))
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::SystemTime};

    use async_trait::async_trait;
    use domain::{
        auth::Username,
        broadcast::{Broadcast, BroadcastDescription, BroadcastStatus, BroadcastTitle, Version},
        id::SnowflakeId,
    };

    use super::*;
    use crate::ports::{
        PortError,
        broadcast::{BroadcastView, ListFilter},
        media::{IngestEndpoint, PlaybackKind, StreamPath},
    };

    const BROADCAST: i64 = 7;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    /// Holds one broadcast in `status`, or none.
    struct Stored(Option<BroadcastStatus>);

    #[async_trait]
    impl BroadcastQuery for Stored {
        async fn list(
            &self,
            _: ListFilter,
            _: Option<BroadcastId>,
            _: u32,
        ) -> Result<Vec<BroadcastView>, PortError> {
            unreachable!("playback never lists")
        }

        async fn get(&self, wanted: BroadcastId) -> Result<Option<BroadcastView>, PortError> {
            Ok(self
                .0
                .filter(|_| wanted == id(BROADCAST))
                .map(|status| BroadcastView {
                    broadcast: Broadcast {
                        id: id(BROADCAST),
                        owner_id: id(1),
                        title: BroadcastTitle::parse("t").unwrap(),
                        description: BroadcastDescription::parse("").unwrap(),
                        status,
                        version: Version::FIRST,
                        created_at: SystemTime::UNIX_EPOCH,
                    },
                    owner_name: Username::parse("alice").unwrap(),
                }))
        }
    }

    struct Router;

    impl IngestRouter for Router {
        fn route(&self, broadcast: BroadcastId) -> IngestEndpoint {
            IngestEndpoint {
                path: StreamPath::new(format!("routed/{broadcast}")),
                whip_url: String::new(),
            }
        }

        fn broadcast_of(&self, _: &StreamPath) -> Option<BroadcastId> {
            unreachable!("playback never maps paths back")
        }
    }

    /// Answers with one source naming the path it was asked about.
    #[derive(Default)]
    struct Resolver(Mutex<Vec<StreamPath>>);

    impl PlaybackResolver for Resolver {
        fn resolve(&self, path: &StreamPath) -> Vec<PlaybackSource> {
            self.0.lock().unwrap().push(path.clone());
            vec![PlaybackSource {
                kind: PlaybackKind::Whep,
                url: path.as_str().to_owned(),
            }]
        }
    }

    async fn resolve(
        stored: Option<BroadcastStatus>,
        asked: i64,
    ) -> (Result<Vec<PlaybackSource>, Error>, Vec<StreamPath>) {
        let resolver = Arc::new(Resolver::default());
        let result = ResolvePlayback {
            query: Arc::new(Stored(stored)),
            ingest: Arc::new(Router),
            resolver: resolver.clone(),
        }
        .run(id(asked))
        .await;
        let asked_paths = resolver.0.lock().unwrap().clone();
        (result, asked_paths)
    }

    #[tokio::test]
    async fn an_on_air_broadcast_resolves_its_routed_path() {
        for status in BroadcastStatus::ACTIVE {
            let (result, asked) = resolve(Some(status), BROADCAST).await;
            let routed = format!("routed/{BROADCAST}");
            assert_eq!(
                result.unwrap(),
                [PlaybackSource {
                    kind: PlaybackKind::Whep,
                    url: routed.clone(),
                }],
                "{status:?}"
            );
            assert_eq!(asked, [StreamPath::new(routed)], "{status:?}");
        }
    }

    #[tokio::test]
    async fn a_ready_or_ended_broadcast_is_not_live() {
        for status in [BroadcastStatus::Ready, BroadcastStatus::Ended] {
            let (result, asked) = resolve(Some(status), BROADCAST).await;
            assert!(
                matches!(result, Err(Error::Rejected(BroadcastError::NotLive))),
                "{status:?}: {result:?}"
            );
            assert!(asked.is_empty(), "{status:?}");
        }
    }

    #[tokio::test]
    async fn a_missing_broadcast_is_not_found() {
        for (stored, asked_id) in [(None, BROADCAST), (Some(BroadcastStatus::Live), 8)] {
            let (result, asked) = resolve(stored, asked_id).await;
            assert!(
                matches!(result, Err(Error::Rejected(BroadcastError::NotFound))),
                "{result:?}"
            );
            assert!(asked.is_empty());
        }
    }
}
