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
    media::{
        IngestRouter, MediaCredentialVerifier, PublishTicketLedger, RedeemRejection, StreamPath,
        TicketId,
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
    /// up and checking the broadcast happen in one write, so a stopped
    /// broadcast or a replayed ticket is refused even under races.
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
}
