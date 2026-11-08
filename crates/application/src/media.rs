//! Deciding whether the media server may let someone in. The media server
//! asks once per publish attempt and once per call to its control interface;
//! watching never asks.

use std::sync::Arc;

use domain::{
    broadcast::{BroadcastId, BroadcastStatus},
    secret::Secret,
};

use crate::ports::{
    PortError,
    broadcast::PublisherPresence,
    media::{
        IngestRouter, MediaCredentialVerifier, PublishTicketLedger, RedeemRejection, StreamPath,
        StreamTerminator, TicketId,
    },
};

/// What the media server wants to allow.
#[derive(Debug)]
pub enum MediaAccess {
    Publish {
        path: StreamPath,
        token: Option<Secret<String>>,
    },
    /// A call to the media server's control interface.
    Control { token: Option<Secret<String>> },
    /// Anything else, such as reading, which the media server is configured
    /// never to ask about.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admitted {
    /// The publish ticket that was used up, if this was a publish.
    pub ticket: Option<(TicketId, BroadcastId)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Denial {
    NoCredential,
    InvalidCredential,
    /// The credential is for another path.
    WrongPath,
    /// The path names no broadcast.
    UnknownPath,
    BroadcastNotFound,
    NotOwner,
    Replayed,
    NotPublishable(BroadcastStatus),
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied {
    pub reason: Denial,
    /// For the logs: the ticket that was presented, when it was valid.
    pub ticket: Option<TicketId>,
}

impl From<Denial> for Denied {
    fn from(reason: Denial) -> Self {
        Self {
            reason,
            ticket: None,
        }
    }
}

pub struct AdmitMediaAccess {
    pub verifier: Arc<dyn MediaCredentialVerifier>,
    pub ingest: Arc<dyn IngestRouter>,
    pub tickets: Arc<dyn PublishTicketLedger>,
}

impl AdmitMediaAccess {
    /// A publisher gets in only with an unused ticket for this path, issued to
    /// the owner of a broadcast that is live or interrupted. Using the ticket
    /// up and checking the broadcast happen in one write, so a replayed ticket
    /// is refused even under races. A stop that commits after that write
    /// cannot keep this publisher out; [`TrackPublisher`] disconnects it when
    /// its stream comes up, and the reaper on its next pass.
    pub async fn run(&self, access: MediaAccess) -> Result<Result<Admitted, Denied>, PortError> {
        match access {
            MediaAccess::Publish { path, token } => self.publish(&path, token).await,
            MediaAccess::Control { token } => Ok(match token {
                None => Err(Denial::NoCredential.into()),
                Some(token) if self.verifier.is_control_credential(&token) => {
                    Ok(Admitted { ticket: None })
                }
                Some(_) => Err(Denial::InvalidCredential.into()),
            }),
            MediaAccess::Other => Ok(Err(Denial::Unsupported.into())),
        }
    }

    async fn publish(
        &self,
        path: &StreamPath,
        token: Option<Secret<String>>,
    ) -> Result<Result<Admitted, Denied>, PortError> {
        let Some(token) = token else {
            return Ok(Err(Denial::NoCredential.into()));
        };
        let Some(ticket) = self.verifier.publish_ticket(&token) else {
            return Ok(Err(Denial::InvalidCredential.into()));
        };
        let denied = |reason| {
            Ok(Err(Denied {
                reason,
                ticket: Some(ticket.id.clone()),
            }))
        };
        if ticket.path != *path {
            return denied(Denial::WrongPath);
        }
        let Some(broadcast) = self.ingest.broadcast_of(path) else {
            return denied(Denial::UnknownPath);
        };
        match self.tickets.redeem(&ticket, broadcast).await? {
            Ok(()) => Ok(Ok(Admitted {
                ticket: Some((ticket.id, broadcast)),
            })),
            Err(rejection) => denied(match rejection {
                RedeemRejection::NotFound => Denial::BroadcastNotFound,
                RedeemRejection::NotOwner => Denial::NotOwner,
                RedeemRejection::AlreadyUsed => Denial::Replayed,
                RedeemRejection::InStatus(status) => Denial::NotPublishable(status),
            }),
        }
    }
}

/// What the media server reports about a stream path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublisherSignal {
    /// A publisher is sending and the stream can be watched.
    Arrived,
    /// The stream is gone: the publisher left, dropped or was kicked.
    Left,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    /// The path names no broadcast.
    UnknownPath,
    /// The broadcast's status changed.
    Changed(BroadcastId),
    /// The signal did not apply in the broadcast's status, such as a late
    /// report about an ended broadcast.
    Unchanged(BroadcastId),
    /// A stream came up for a broadcast that is not active, so its publisher
    /// was told to leave.
    NotActive {
        broadcast: BroadcastId,
        kick: Result<(), PortError>,
    },
}

/// Follows the media server's reports. They are hints that can arrive late,
/// twice or out of order; each becomes a conditional write that applies only
/// in the status it is meant for, so none can move an ended broadcast.
pub struct TrackPublisher {
    pub presence: Arc<dyn PublisherPresence>,
    pub ingest: Arc<dyn IngestRouter>,
    pub terminator: Arc<dyn StreamTerminator>,
}

impl TrackPublisher {
    pub async fn run(
        &self,
        path: &StreamPath,
        signal: PublisherSignal,
    ) -> Result<Observed, PortError> {
        let Some(broadcast) = self.ingest.broadcast_of(path) else {
            return Ok(Observed::UnknownPath);
        };
        let changed = match signal {
            PublisherSignal::Arrived => self.presence.resume(broadcast).await?,
            PublisherSignal::Left => self.presence.interrupt(broadcast).await?,
        };
        if changed {
            return Ok(Observed::Changed(broadcast));
        }
        if signal == PublisherSignal::Arrived && !self.presence.is_active(broadcast).await? {
            return Ok(Observed::NotActive {
                broadcast,
                kick: self.terminator.kick(path).await,
            });
        }
        Ok(Observed::Unchanged(broadcast))
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Mutex, time::SystemTime};

    use async_trait::async_trait;
    use domain::{auth::UserId, id::SnowflakeId};

    use super::*;
    use crate::ports::media::{IngestEndpoint, PublishTicket};

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    const PUBLISHER: i64 = 1;
    const BROADCAST: i64 = 7;

    fn path(raw: &str) -> StreamPath {
        StreamPath::new(raw.into())
    }

    /// `publish:<path>` is a publish ticket for that path, `control` is the
    /// control credential, anything else is invalid.
    struct Verifier;

    impl MediaCredentialVerifier for Verifier {
        fn publish_ticket(&self, token: &Secret<String>) -> Option<PublishTicket> {
            let target = token.expose().strip_prefix("publish:")?;
            Some(PublishTicket {
                id: TicketId::new(format!("jti-{target}")),
                publisher: id(PUBLISHER),
                path: path(target),
                expires_at: SystemTime::UNIX_EPOCH,
            })
        }

        fn is_control_credential(&self, token: &Secret<String>) -> bool {
            token.expose() == "control"
        }
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

    struct Ledger {
        outcome: Result<(), RedeemRejection>,
        redeemed: Mutex<Vec<(TicketId, UserId, BroadcastId)>>,
    }

    #[async_trait]
    impl PublishTicketLedger for Ledger {
        async fn redeem(
            &self,
            ticket: &PublishTicket,
            broadcast: BroadcastId,
        ) -> Result<Result<(), RedeemRejection>, PortError> {
            self.redeemed
                .lock()
                .unwrap()
                .push((ticket.id.clone(), ticket.publisher, broadcast));
            Ok(self.outcome)
        }
    }

    async fn admit(
        outcome: Result<(), RedeemRejection>,
        access: MediaAccess,
    ) -> (Result<Admitted, Denied>, usize) {
        let ledger = Arc::new(Ledger {
            outcome,
            redeemed: Mutex::default(),
        });
        let result = AdmitMediaAccess {
            verifier: Arc::new(Verifier),
            ingest: Arc::new(Live),
            tickets: ledger.clone(),
        }
        .run(access)
        .await
        .unwrap();
        let redeemed = ledger.redeemed.lock().unwrap().len();
        (result, redeemed)
    }

    fn publish(target: &str, token: Option<&str>) -> MediaAccess {
        MediaAccess::Publish {
            path: path(target),
            token: token.map(|t| Secret::new(t.to_owned())),
        }
    }

    #[tokio::test]
    async fn a_ticket_for_the_path_is_redeemed_for_its_broadcast() {
        let target = format!("live/{BROADCAST}");
        let (result, redeemed) =
            admit(Ok(()), publish(&target, Some(&format!("publish:{target}")))).await;
        assert_eq!(
            result,
            Ok(Admitted {
                ticket: Some((TicketId::new(format!("jti-{target}")), id(BROADCAST)))
            })
        );
        assert_eq!(redeemed, 1);
    }

    #[tokio::test]
    async fn the_ledger_decides_replays_and_broadcast_state() {
        let target = format!("live/{BROADCAST}");
        for (rejection, reason) in [
            (RedeemRejection::NotFound, Denial::BroadcastNotFound),
            (RedeemRejection::NotOwner, Denial::NotOwner),
            (RedeemRejection::AlreadyUsed, Denial::Replayed),
            (
                RedeemRejection::InStatus(BroadcastStatus::Ended),
                Denial::NotPublishable(BroadcastStatus::Ended),
            ),
        ] {
            let (result, _) = admit(
                Err(rejection),
                publish(&target, Some(&format!("publish:{target}"))),
            )
            .await;
            let denied = result.unwrap_err();
            assert_eq!(denied.reason, reason, "{rejection:?}");
            assert_eq!(
                denied.ticket,
                Some(TicketId::new(format!("jti-{target}"))),
                "{rejection:?}"
            );
        }
    }

    #[tokio::test]
    async fn bad_credentials_and_paths_never_reach_the_ledger() {
        for (access, reason) in [
            (publish("live/7", None), Denial::NoCredential),
            (
                publish("live/7", Some("garbage")),
                Denial::InvalidCredential,
            ),
            (
                publish("live/7", Some("control")),
                Denial::InvalidCredential,
            ),
            (publish("live/7", Some("publish:live/8")), Denial::WrongPath),
            (
                publish("other/7", Some("publish:other/7")),
                Denial::UnknownPath,
            ),
            (MediaAccess::Other, Denial::Unsupported),
            (MediaAccess::Control { token: None }, Denial::NoCredential),
            (
                MediaAccess::Control {
                    token: Some(Secret::new("publish:live/7".into())),
                },
                Denial::InvalidCredential,
            ),
        ] {
            let shown = format!("{access:?}");
            let (result, redeemed) = admit(Ok(()), access).await;
            assert_eq!(result.unwrap_err().reason, reason, "{shown}");
            assert_eq!(redeemed, 0, "{shown}");
        }
    }

    #[tokio::test]
    async fn the_control_credential_is_admitted_without_a_ticket() {
        let (result, redeemed) = admit(
            Ok(()),
            MediaAccess::Control {
                token: Some(Secret::new("control".into())),
            },
        )
        .await;
        assert_eq!(result, Ok(Admitted { ticket: None }));
        assert_eq!(redeemed, 0);
    }

    /// Interrupts change the status of an active broadcast, resumes never
    /// change it.
    #[derive(Default)]
    struct Presence {
        calls: Mutex<Vec<(&'static str, BroadcastId)>>,
        active: bool,
    }

    #[async_trait]
    impl PublisherPresence for Presence {
        async fn interrupt(&self, id: BroadcastId) -> Result<bool, PortError> {
            self.calls.lock().unwrap().push(("interrupt", id));
            Ok(self.active)
        }

        async fn resume(&self, id: BroadcastId) -> Result<bool, PortError> {
            self.calls.lock().unwrap().push(("resume", id));
            Ok(false)
        }

        async fn is_active(&self, _: BroadcastId) -> Result<bool, PortError> {
            Ok(self.active)
        }
    }

    #[derive(Default)]
    struct Kicks(Mutex<Vec<String>>);

    #[async_trait]
    impl StreamTerminator for Kicks {
        async fn kick(&self, path: &StreamPath) -> Result<(), PortError> {
            self.0.lock().unwrap().push(path.as_str().to_owned());
            Ok(())
        }
    }

    fn track(active: bool) -> (TrackPublisher, Arc<Presence>, Arc<Kicks>) {
        let presence = Arc::new(Presence {
            active,
            ..Presence::default()
        });
        let kicks = Arc::new(Kicks::default());
        let track = TrackPublisher {
            presence: presence.clone(),
            ingest: Arc::new(Live),
            terminator: kicks.clone(),
        };
        (track, presence, kicks)
    }

    #[tokio::test]
    async fn arrivals_resume_departures_interrupt_and_unknown_paths_do_nothing() {
        let (track, presence, kicks) = track(true);
        let live = path(&format!("live/{BROADCAST}"));
        assert_eq!(
            track.run(&live, PublisherSignal::Left).await.unwrap(),
            Observed::Changed(id(BROADCAST))
        );
        assert_eq!(
            track.run(&live, PublisherSignal::Arrived).await.unwrap(),
            Observed::Unchanged(id(BROADCAST))
        );
        assert_eq!(
            track
                .run(&path("other/7"), PublisherSignal::Left)
                .await
                .unwrap(),
            Observed::UnknownPath
        );
        assert_eq!(
            *presence.calls.lock().unwrap(),
            [("interrupt", id(BROADCAST)), ("resume", id(BROADCAST))]
        );
        assert!(kicks.0.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_stream_that_comes_up_for_a_broadcast_that_is_not_active_is_kicked() {
        let (track, _, kicks) = track(false);
        let live = path(&format!("live/{BROADCAST}"));
        assert_eq!(
            track.run(&live, PublisherSignal::Arrived).await.unwrap(),
            Observed::NotActive {
                broadcast: id(BROADCAST),
                kick: Ok(())
            }
        );
        assert_eq!(
            track.run(&live, PublisherSignal::Left).await.unwrap(),
            Observed::Unchanged(id(BROADCAST)),
            "a departure is never kicked"
        );
        assert_eq!(*kicks.0.lock().unwrap(), [live.as_str()]);
    }
}
