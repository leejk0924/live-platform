//! Going live and ending a broadcast. As with the other broadcast writes,
//! only the authenticated actor is passed to the repository as the owner.

use std::sync::Arc;

use domain::{
    auth::UserId,
    broadcast::{BroadcastError, BroadcastId, BroadcastStatus},
};

use crate::{
    broadcast::{Error, view_of},
    ports::{
        PortError,
        auth::UserRepository,
        broadcast::{BroadcastRepository, BroadcastView, TransitionRejection},
        media::{IngestRouter, PublishAuthorizer, PublishCredential, StreamTerminator},
    },
};

fn rejected(rejection: TransitionRejection, wrong_status: BroadcastError) -> Error {
    Error::Rejected(match rejection {
        TransitionRejection::NotFound => BroadcastError::NotFound,
        TransitionRejection::NotOwner => BroadcastError::NotOwner,
        TransitionRejection::InStatus(_) => wrong_status,
    })
}

/// Where to publish and the credential to publish with.
#[derive(Debug)]
pub struct PublishGrant {
    pub whip_url: String,
    pub credential: PublishCredential,
}

pub struct StartBroadcast {
    pub broadcasts: Arc<dyn BroadcastRepository>,
    pub ingest: Arc<dyn IngestRouter>,
    pub authorizer: Arc<dyn PublishAuthorizer>,
}

impl StartBroadcast {
    /// Takes a `READY` broadcast live. The owner of a broadcast that is
    /// already `LIVE` gets a fresh credential without another transition, so
    /// a reloaded page can publish again.
    pub async fn run(&self, actor: UserId, id: BroadcastId) -> Result<PublishGrant, Error> {
        match self.broadcasts.start(id, actor).await? {
            Ok(_) | Err(TransitionRejection::InStatus(BroadcastStatus::Live)) => {}
            Err(rejection) => return Err(rejected(rejection, BroadcastError::NotStartable)),
        }
        let endpoint = self.ingest.route(id);
        let credential = self.authorizer.authorize(actor, &endpoint.path).await?;
        Ok(PublishGrant {
            whip_url: endpoint.whip_url,
            credential,
        })
    }
}

#[derive(Debug)]
pub struct Stopped {
    pub broadcast: BroadcastView,
    /// The broadcast is `ENDED` either way; a publisher that could not be
    /// disconnected is left for the caller to report.
    pub kick_failure: Option<PortError>,
}

pub struct StopBroadcast {
    pub broadcasts: Arc<dyn BroadcastRepository>,
    pub users: Arc<dyn UserRepository>,
    pub ingest: Arc<dyn IngestRouter>,
    pub terminator: Arc<dyn StreamTerminator>,
}

impl StopBroadcast {
    /// Ends the broadcast, then disconnects its publisher. The kick runs only
    /// after the status change is stored, so a publisher that reconnects in
    /// between finds the broadcast already ended.
    pub async fn run(&self, actor: UserId, id: BroadcastId) -> Result<Stopped, Error> {
        let broadcast = self
            .broadcasts
            .stop(id, actor)
            .await?
            .map_err(|rejection| rejected(rejection, BroadcastError::NotLive))?;
        let kick_failure = self
            .terminator
            .kick(&self.ingest.route(id).path)
            .await
            .err();
        Ok(Stopped {
            broadcast: view_of(self.users.as_ref(), broadcast).await?,
            kick_failure,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, SystemTime},
    };

    use async_trait::async_trait;
    use domain::{
        auth::{Credential, User, Username},
        broadcast::{Broadcast, BroadcastDescription, BroadcastTitle, Version},
        id::SnowflakeId,
        secret::Secret,
    };

    use super::*;
    use crate::ports::{
        auth::Inserted,
        broadcast::{NewBroadcast, WriteRejection},
        media::{IngestEndpoint, StreamPath},
    };

    type Transition = Result<Broadcast, TransitionRejection>;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    const OWNER: i64 = 1;
    const BROADCAST: i64 = 7;

    fn broadcast(status: BroadcastStatus) -> Broadcast {
        Broadcast {
            id: id(BROADCAST),
            owner_id: id(OWNER),
            title: BroadcastTitle::parse("t").unwrap(),
            description: BroadcastDescription::parse("").unwrap(),
            status,
            version: Version::FIRST,
            created_at: SystemTime::UNIX_EPOCH,
        }
    }

    /// Answers `start` and `stop` with a fixed outcome and records the calls.
    struct Scripted {
        outcome: Transition,
        calls: Mutex<Vec<(BroadcastId, UserId)>>,
    }

    #[async_trait]
    impl BroadcastRepository for Scripted {
        async fn insert(&self, _: &NewBroadcast) -> Result<Broadcast, PortError> {
            unreachable!()
        }

        async fn update(
            &self,
            _: BroadcastId,
            _: UserId,
            _: Version,
            _: &BroadcastTitle,
            _: &BroadcastDescription,
        ) -> Result<Result<Broadcast, WriteRejection>, PortError> {
            unreachable!()
        }

        async fn soft_delete(
            &self,
            _: BroadcastId,
            _: UserId,
        ) -> Result<Result<(), WriteRejection>, PortError> {
            unreachable!()
        }

        async fn start(&self, id: BroadcastId, owner: UserId) -> Result<Transition, PortError> {
            self.calls.lock().unwrap().push((id, owner));
            Ok(self.outcome.clone())
        }

        async fn stop(&self, id: BroadcastId, owner: UserId) -> Result<Transition, PortError> {
            self.calls.lock().unwrap().push((id, owner));
            Ok(self.outcome.clone())
        }
    }

    struct Owner;

    #[async_trait]
    impl UserRepository for Owner {
        async fn insert(&self, _: &User) -> Result<Inserted, PortError> {
            unreachable!()
        }

        async fn find_by_username(&self, _: &Username) -> Result<Option<User>, PortError> {
            unreachable!()
        }

        async fn find_by_id(&self, user: UserId) -> Result<Option<User>, PortError> {
            Ok(Some(User {
                id: user,
                username: Username::parse("owner").unwrap(),
                credential: Credential::DemoOnly,
            }))
        }
    }

    struct LivePath;

    impl IngestRouter for LivePath {
        fn route(&self, broadcast: BroadcastId) -> IngestEndpoint {
            IngestEndpoint {
                path: StreamPath::new(format!("live/{broadcast}")),
                whip_url: format!("/whip/{broadcast}"),
            }
        }
    }

    #[derive(Default)]
    struct CountingAuthorizer(Mutex<Vec<(UserId, StreamPath)>>);

    #[async_trait]
    impl PublishAuthorizer for CountingAuthorizer {
        async fn authorize(
            &self,
            publisher: UserId,
            path: &StreamPath,
        ) -> Result<PublishCredential, PortError> {
            self.0.lock().unwrap().push((publisher, path.clone()));
            Ok(PublishCredential::Bearer {
                token: Secret::new("token".into()),
                expires_in: Duration::from_secs(60),
            })
        }
    }

    #[derive(Default)]
    struct CountingTerminator {
        kicks: AtomicUsize,
        fail: bool,
    }

    #[async_trait]
    impl StreamTerminator for CountingTerminator {
        async fn kick(&self, path: &StreamPath) -> Result<(), PortError> {
            assert_eq!(path.as_str(), format!("live/{BROADCAST}"));
            self.kicks.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(PortError("media server unreachable".into()));
            }
            Ok(())
        }
    }

    async fn start(outcome: Transition) -> (Result<PublishGrant, Error>, Arc<CountingAuthorizer>) {
        let authorizer = Arc::new(CountingAuthorizer::default());
        let repository = Arc::new(Scripted {
            outcome,
            calls: Mutex::default(),
        });
        let result = StartBroadcast {
            broadcasts: repository.clone(),
            ingest: Arc::new(LivePath),
            authorizer: authorizer.clone(),
        }
        .run(id(OWNER), id(BROADCAST))
        .await;
        assert_eq!(
            *repository.calls.lock().unwrap(),
            [(id(BROADCAST), id(OWNER))],
            "the actor is the owner the write checks"
        );
        (result, authorizer)
    }

    async fn stop(
        outcome: Transition,
        terminator: CountingTerminator,
    ) -> (Result<Stopped, Error>, usize) {
        let terminator = Arc::new(terminator);
        let result = StopBroadcast {
            broadcasts: Arc::new(Scripted {
                outcome,
                calls: Mutex::default(),
            }),
            users: Arc::new(Owner),
            ingest: Arc::new(LivePath),
            terminator: terminator.clone(),
        }
        .run(id(OWNER), id(BROADCAST))
        .await;
        (result, terminator.kicks.load(Ordering::SeqCst))
    }

    fn rejection_code(result: Result<impl std::fmt::Debug, Error>) -> BroadcastError {
        match result {
            Err(Error::Rejected(err)) => err,
            other => panic!("expected a rejection, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_authorizes_the_routed_path_for_the_actor() {
        for outcome in [
            Ok(broadcast(BroadcastStatus::Live)),
            Err(TransitionRejection::InStatus(BroadcastStatus::Live)),
        ] {
            let (result, authorizer) = start(outcome.clone()).await;
            let grant = result.unwrap();
            assert_eq!(grant.whip_url, format!("/whip/{BROADCAST}"), "{outcome:?}");
            assert_eq!(
                *authorizer.0.lock().unwrap(),
                [(id(OWNER), StreamPath::new(format!("live/{BROADCAST}")))],
                "{outcome:?}"
            );
        }
    }

    #[tokio::test]
    async fn start_never_authorizes_when_the_transition_is_rejected() {
        for (rejection, expected) in [
            (TransitionRejection::NotFound, BroadcastError::NotFound),
            (TransitionRejection::NotOwner, BroadcastError::NotOwner),
            (
                TransitionRejection::InStatus(BroadcastStatus::Ended),
                BroadcastError::NotStartable,
            ),
            (
                TransitionRejection::InStatus(BroadcastStatus::Interrupted),
                BroadcastError::NotStartable,
            ),
        ] {
            let (result, authorizer) = start(Err(rejection)).await;
            assert_eq!(rejection_code(result), expected, "{rejection:?}");
            assert!(authorizer.0.lock().unwrap().is_empty(), "{rejection:?}");
        }
    }

    #[tokio::test]
    async fn stop_kicks_once_after_the_transition() {
        let (result, kicks) = stop(
            Ok(broadcast(BroadcastStatus::Ended)),
            CountingTerminator::default(),
        )
        .await;
        let stopped = result.unwrap();
        assert_eq!(stopped.broadcast.broadcast.status, BroadcastStatus::Ended);
        assert!(stopped.kick_failure.is_none());
        assert_eq!(kicks, 1);
    }

    #[tokio::test]
    async fn stop_never_kicks_when_the_transition_is_rejected() {
        for (rejection, expected) in [
            (TransitionRejection::NotFound, BroadcastError::NotFound),
            (TransitionRejection::NotOwner, BroadcastError::NotOwner),
            (
                TransitionRejection::InStatus(BroadcastStatus::Ready),
                BroadcastError::NotLive,
            ),
            (
                TransitionRejection::InStatus(BroadcastStatus::Ended),
                BroadcastError::NotLive,
            ),
        ] {
            let (result, kicks) = stop(Err(rejection), CountingTerminator::default()).await;
            assert_eq!(rejection_code(result), expected, "{rejection:?}");
            assert_eq!(kicks, 0, "{rejection:?}");
        }
    }

    #[tokio::test]
    async fn a_failed_kick_still_ends_the_broadcast() {
        let failing = CountingTerminator {
            fail: true,
            ..Default::default()
        };
        let (result, kicks) = stop(Ok(broadcast(BroadcastStatus::Ended)), failing).await;
        let stopped = result.unwrap();
        assert_eq!(stopped.broadcast.broadcast.status, BroadcastStatus::Ended);
        assert_eq!(
            stopped.kick_failure,
            Some(PortError("media server unreachable".into()))
        );
        assert_eq!(kicks, 1);
    }
}
