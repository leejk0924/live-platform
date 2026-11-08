use std::{
    io,
    sync::{Arc, Mutex},
};

use application::ports::{
    PortError,
    auth::{Inserted, PasswordHasher, SessionStore, UserRepository},
    broadcast::{
        BroadcastQuery, BroadcastRepository, BroadcastView, ListFilter, NewBroadcast,
        PublisherPresence, TransitionRejection, WriteRejection,
    },
    comment::{
        ChatPolicyStore, ChatTarget, CommentQuery, CommentRepository, CommentView,
        CommentWriteRejection, NewComment, PolicyRejection,
    },
    donation::{
        DonationOrder, DonationQuery, DonationRepository, DonationSummary, DonationView,
        NewDonation, Settlement,
    },
    event_bus::{EventBus, EventSubscription, RealtimeEvent},
    id::{IdGenerationError, IdGenerator},
    media::{
        IngestEndpoint, IngestRouter, MediaCredentialVerifier, PlaybackResolver, PlaybackSource,
        PublishAuthorizer, PublishCredential, PublishTicket, PublishTicketLedger, RedeemRejection,
        StreamPath, StreamTerminator,
    },
    payment::PaymentGateway,
    rate_limit::{RateKey, RateLimiter},
    recording::{
        ClaimedRecording, NewRecording, RecordingFilter, RecordingQuery, RecordingRepository,
        RecordingView, SegmentSpool, SpooledSegment, UrlSigner,
    },
};
use async_trait::async_trait;
use domain::{
    auth::{Password, PasswordHash, SessionToken, User, UserId, Username},
    broadcast::{
        Broadcast, BroadcastDescription, BroadcastId, BroadcastStatus, BroadcastTitle, Version,
    },
    comment::{ChatPolicy, CommentBody, CommentId, Revision},
    donation::{DonationId, PaymentKey},
    id::SnowflakeId,
    recording::{RecordingId, RecordingKey, StoreRetry},
    secret::Secret,
};
use infrastructure::{auth::BreachedPasswordList, media::PublishKey};
use tracing::Subscriber;
use tracing_subscriber::fmt::MakeWriter;

use crate::{
    AppState,
    auth::AuthPorts,
    broadcast::BroadcastPorts,
    comment::CommentPorts,
    config::Origin,
    donation::{DonationPorts, PaymentPorts},
    recording::RecordingPorts,
};

/// Stands in for every port a test does not exercise; each call fails.
struct Unavailable;

fn unavailable<T>() -> Result<T, PortError> {
    Err(PortError("not available in this test".into()))
}

impl IdGenerator for Unavailable {
    fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
        Err(IdGenerationError::LeaseExpired)
    }
}

#[async_trait]
impl UserRepository for Unavailable {
    async fn insert(&self, _: &User) -> Result<Inserted, PortError> {
        unavailable()
    }

    async fn find_by_username(&self, _: &Username) -> Result<Option<User>, PortError> {
        unavailable()
    }

    async fn find_by_id(&self, _: UserId) -> Result<Option<User>, PortError> {
        unavailable()
    }
}

#[async_trait]
impl SessionStore for Unavailable {
    async fn create(&self, _: UserId) -> Result<SessionToken, PortError> {
        unavailable()
    }

    async fn resolve(&self, _: &SessionToken) -> Result<Option<UserId>, PortError> {
        unavailable()
    }

    async fn revoke(&self, _: &SessionToken) -> Result<(), PortError> {
        unavailable()
    }
}

#[async_trait]
impl PasswordHasher for Unavailable {
    async fn hash(&self, _: &Password) -> Result<PasswordHash, PortError> {
        unavailable()
    }

    async fn verify(&self, _: &Password, _: Option<&PasswordHash>) -> Result<bool, PortError> {
        unavailable()
    }
}

#[async_trait]
impl BroadcastRepository for Unavailable {
    async fn insert(&self, _: &NewBroadcast) -> Result<Broadcast, PortError> {
        unavailable()
    }

    async fn update(
        &self,
        _: BroadcastId,
        _: UserId,
        _: Version,
        _: &BroadcastTitle,
        _: &BroadcastDescription,
    ) -> Result<Result<Broadcast, WriteRejection>, PortError> {
        unavailable()
    }

    async fn soft_delete(
        &self,
        _: BroadcastId,
        _: UserId,
    ) -> Result<Result<(), WriteRejection>, PortError> {
        unavailable()
    }

    async fn start(
        &self,
        _: BroadcastId,
        _: UserId,
    ) -> Result<Result<Broadcast, TransitionRejection>, PortError> {
        unavailable()
    }

    async fn stop(
        &self,
        _: BroadcastId,
        _: UserId,
    ) -> Result<Result<Broadcast, TransitionRejection>, PortError> {
        unavailable()
    }
}

#[async_trait]
impl PublisherPresence for Unavailable {
    async fn interrupt(&self, _: BroadcastId) -> Result<bool, PortError> {
        unavailable()
    }

    async fn resume(&self, _: BroadcastId) -> Result<bool, PortError> {
        unavailable()
    }

    async fn is_active(&self, _: BroadcastId) -> Result<bool, PortError> {
        unavailable()
    }
}

#[async_trait]
impl BroadcastQuery for Unavailable {
    async fn list(
        &self,
        _: ListFilter,
        _: Option<BroadcastId>,
        _: u32,
    ) -> Result<Vec<BroadcastView>, PortError> {
        unavailable()
    }

    async fn get(&self, _: BroadcastId) -> Result<Option<BroadcastView>, PortError> {
        unavailable()
    }
}

impl IngestRouter for Unavailable {
    fn route(&self, _: BroadcastId) -> IngestEndpoint {
        unreachable!("not available in this test")
    }

    fn broadcast_of(&self, _: &StreamPath) -> Option<BroadcastId> {
        unreachable!("not available in this test")
    }
}

impl PlaybackResolver for Unavailable {
    fn resolve(&self, _: &StreamPath) -> Vec<PlaybackSource> {
        unreachable!("not available in this test")
    }
}

impl MediaCredentialVerifier for Unavailable {
    fn publish_ticket(&self, _: &Secret<String>) -> Option<PublishTicket> {
        unreachable!("not available in this test")
    }

    fn is_control_credential(&self, _: &Secret<String>) -> bool {
        unreachable!("not available in this test")
    }
}

#[async_trait]
impl PublishTicketLedger for Unavailable {
    async fn redeem(
        &self,
        _: &PublishTicket,
        _: BroadcastId,
    ) -> Result<Result<(), RedeemRejection>, PortError> {
        unavailable()
    }
}

#[async_trait]
impl PublishAuthorizer for Unavailable {
    async fn authorize(&self, _: UserId, _: &StreamPath) -> Result<PublishCredential, PortError> {
        unavailable()
    }
}

#[async_trait]
impl StreamTerminator for Unavailable {
    async fn kick(&self, _: &StreamPath) -> Result<(), PortError> {
        unavailable()
    }
}

#[async_trait]
impl CommentRepository for Unavailable {
    async fn insert(&self, _: &NewComment, _: bool) -> Result<Option<CommentView>, PortError> {
        unavailable()
    }

    async fn edit(
        &self,
        _: BroadcastId,
        _: CommentId,
        _: UserId,
        _: Version,
        _: &CommentBody,
    ) -> Result<Result<CommentView, CommentWriteRejection>, PortError> {
        unavailable()
    }

    async fn soft_delete(
        &self,
        _: BroadcastId,
        _: CommentId,
        _: UserId,
    ) -> Result<Result<CommentView, CommentWriteRejection>, PortError> {
        unavailable()
    }
}

#[async_trait]
impl CommentQuery for Unavailable {
    async fn list(
        &self,
        _: BroadcastId,
        _: Option<CommentId>,
        _: u32,
    ) -> Result<Vec<CommentView>, PortError> {
        unavailable()
    }

    async fn changes_since(
        &self,
        _: BroadcastId,
        _: Revision,
        _: u32,
    ) -> Result<Vec<CommentView>, PortError> {
        unavailable()
    }

    async fn latest_revision(&self, _: BroadcastId) -> Result<Revision, PortError> {
        unavailable()
    }
}

#[async_trait]
impl ChatPolicyStore for Unavailable {
    async fn target(&self, _: BroadcastId) -> Result<Option<ChatTarget>, PortError> {
        unavailable()
    }

    async fn set(
        &self,
        _: BroadcastId,
        _: UserId,
        _: ChatPolicy,
    ) -> Result<Result<(), PolicyRejection>, PortError> {
        unavailable()
    }
}

#[async_trait]
impl DonationRepository for Unavailable {
    async fn create(&self, _: &NewDonation) -> Result<bool, PortError> {
        unavailable()
    }

    async fn order(&self, _: DonationId) -> Result<Option<DonationOrder>, PortError> {
        unavailable()
    }

    async fn settle(
        &self,
        _: DonationId,
        _: &PaymentKey,
        _: std::time::SystemTime,
    ) -> Result<Settlement, PortError> {
        unavailable()
    }
}

#[async_trait]
impl DonationQuery for Unavailable {
    async fn list(
        &self,
        _: BroadcastId,
        _: Option<DonationId>,
        _: u32,
    ) -> Result<Vec<DonationView>, PortError> {
        unavailable()
    }

    async fn summary(&self, _: BroadcastId) -> Result<DonationSummary, PortError> {
        unavailable()
    }
}

impl RateLimiter for Unavailable {
    fn acquire(&self, _: &[(RateKey, std::time::Duration)]) -> Result<(), std::time::Duration> {
        unreachable!("not available in this test")
    }
}

#[async_trait]
impl EventBus for Unavailable {
    async fn publish(&self, _: RealtimeEvent) {}

    fn subscribe(&self, _: BroadcastId) -> Box<dyn EventSubscription> {
        unreachable!("not available in this test")
    }
}

#[async_trait]
impl RecordingRepository for Unavailable {
    async fn register(&self, _: &NewRecording) -> Result<bool, PortError> {
        unavailable()
    }

    async fn claim(&self, _: std::time::Duration) -> Result<Option<ClaimedRecording>, PortError> {
        unavailable()
    }

    async fn mark_stored(&self, _: RecordingId, _: u64) -> Result<bool, PortError> {
        unavailable()
    }

    async fn release_failed(
        &self,
        _: RecordingId,
        _: u32,
        _: StoreRetry,
    ) -> Result<bool, PortError> {
        unavailable()
    }
}

#[async_trait]
impl RecordingQuery for Unavailable {
    async fn list(
        &self,
        _: RecordingFilter,
        _: Option<RecordingId>,
        _: u32,
    ) -> Result<Vec<RecordingView>, PortError> {
        unavailable()
    }

    async fn get(&self, _: RecordingId) -> Result<Option<RecordingView>, PortError> {
        unavailable()
    }
}

#[async_trait]
impl SegmentSpool for Unavailable {
    fn segment(&self, _: &StreamPath, _: &str) -> Option<SpooledSegment> {
        unreachable!("not available in this test")
    }

    async fn discard(&self, _: &SpooledSegment) -> Result<(), PortError> {
        unavailable()
    }
}

impl UrlSigner for Unavailable {
    fn sign(&self, _: RecordingId, _: &RecordingKey, _: std::time::SystemTime) -> String {
        unreachable!("not available in this test")
    }
}

pub const TEST_ORIGIN: &str = "https://app.test";
pub const TEST_MEDIA_SECRET: &str = "0123456789abcdef0123456789abcdef";

fn unavailable_comments() -> CommentPorts {
    CommentPorts {
        repository: Arc::new(Unavailable),
        query: Arc::new(Unavailable),
        policies: Arc::new(Unavailable),
        limiter: Arc::new(Unavailable),
        donations: Arc::new(Unavailable),
        events: Arc::new(Unavailable),
    }
}

/// Donations are off: no Toss keys.
pub fn test_state() -> AppState {
    state_with(unavailable_comments(), None)
}

/// Payments go to `gateway`; every store is unavailable.
pub fn payments_state(gateway: Arc<dyn PaymentGateway>) -> AppState {
    let payments = PaymentPorts {
        gateway,
        client_key: Secret::new("test_ck_example".into()),
    };
    state_with(unavailable_comments(), Some(payments))
}

/// Every broadcast but 404 is live with no comments and no donations, and
/// events travel over `bus`.
pub fn chat_state(bus: Arc<dyn EventBus>) -> AppState {
    let comments = CommentPorts {
        repository: Arc::new(Unavailable),
        query: Arc::new(EmptyChat),
        policies: Arc::new(EmptyChat),
        limiter: Arc::new(Unavailable),
        donations: Arc::new(EmptyChat),
        events: bus,
    };
    state_with(comments, None)
}

struct EmptyChat;

#[async_trait]
impl CommentQuery for EmptyChat {
    async fn list(
        &self,
        _: BroadcastId,
        _: Option<CommentId>,
        _: u32,
    ) -> Result<Vec<CommentView>, PortError> {
        Ok(Vec::new())
    }

    async fn changes_since(
        &self,
        _: BroadcastId,
        _: Revision,
        _: u32,
    ) -> Result<Vec<CommentView>, PortError> {
        Ok(Vec::new())
    }

    async fn latest_revision(&self, _: BroadcastId) -> Result<Revision, PortError> {
        Ok(Revision::START)
    }
}

#[async_trait]
impl DonationQuery for EmptyChat {
    async fn list(
        &self,
        _: BroadcastId,
        _: Option<DonationId>,
        _: u32,
    ) -> Result<Vec<DonationView>, PortError> {
        Ok(Vec::new())
    }

    async fn summary(&self, _: BroadcastId) -> Result<DonationSummary, PortError> {
        Ok(DonationSummary::default())
    }
}

#[async_trait]
impl ChatPolicyStore for EmptyChat {
    async fn target(&self, broadcast: BroadcastId) -> Result<Option<ChatTarget>, PortError> {
        Ok((broadcast.as_i64() != 404).then_some(ChatTarget {
            owner: SnowflakeId::try_from(1).unwrap(),
            status: BroadcastStatus::Live,
            policy: ChatPolicy::OFF,
        }))
    }

    async fn set(
        &self,
        _: BroadcastId,
        _: UserId,
        _: ChatPolicy,
    ) -> Result<Result<(), PolicyRejection>, PortError> {
        unavailable()
    }
}

/// One Server-Sent Event as a client parses it.
#[derive(Debug)]
pub struct SseEvent {
    pub event: String,
    pub id: Option<String>,
    pub data: String,
}

impl SseEvent {
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.data).unwrap()
    }
}

/// Reads an event stream response the way `EventSource` does, skipping
/// keep-alive comments.
pub struct SseReader {
    body: axum::body::BodyDataStream,
    buffer: String,
}

impl SseReader {
    pub fn new(response: axum::response::Response) -> Self {
        Self {
            body: response.into_body().into_data_stream(),
            buffer: String::new(),
        }
    }

    /// The next event, or `None` when the stream ends.
    pub async fn next(&mut self) -> Option<SseEvent> {
        use futures_util::StreamExt;
        loop {
            while let Some(end) = self.buffer.find(
                "

",
            ) {
                let block: String = self.buffer.drain(..end + 2).collect();
                let mut event = SseEvent {
                    event: "message".into(),
                    id: None,
                    data: String::new(),
                };
                let mut fields = 0;
                for line in block.lines() {
                    let Some((name, value)) = line.split_once(':') else {
                        continue;
                    };
                    let value = value.strip_prefix(' ').unwrap_or(value).to_owned();
                    fields += 1;
                    match name {
                        "event" => event.event = value,
                        "id" => event.id = Some(value),
                        "data" => event.data.push_str(&value),
                        _ => fields -= 1,
                    }
                }
                if fields > 0 {
                    return Some(event);
                }
            }
            let chunk = self.body.next().await?.unwrap();
            self.buffer.push_str(std::str::from_utf8(&chunk).unwrap());
        }
    }
}

fn state_with(comment_ports: CommentPorts, payments: Option<PaymentPorts>) -> AppState {
    let ports = AuthPorts {
        ids: Arc::new(Unavailable),
        users: Arc::new(Unavailable),
        sessions: Arc::new(Unavailable),
        hasher: Arc::new(Unavailable),
        breached: Arc::new(BreachedPasswordList::load()),
    };
    let broadcast_ports = BroadcastPorts {
        repository: Arc::new(Unavailable),
        query: Arc::new(Unavailable),
        ingest: Arc::new(Unavailable),
        playback: Arc::new(Unavailable),
        authorizer: Arc::new(Unavailable),
        terminator: Arc::new(Unavailable),
        verifier: Arc::new(Unavailable),
        tickets: Arc::new(Unavailable),
        presence: Arc::new(Unavailable),
    };
    let recording_ports = RecordingPorts {
        repository: Arc::new(Unavailable),
        query: Arc::new(Unavailable),
        spool: Arc::new(Unavailable),
        signer: Arc::new(Unavailable),
        local: None,
    };
    let donation_ports = DonationPorts {
        repository: Arc::new(Unavailable),
        query: Arc::new(Unavailable),
        limiter: Arc::new(Unavailable),
        payments,
    };
    AppState::new(
        ports,
        broadcast_ports,
        comment_ports,
        donation_ports,
        recording_ports,
        vec![Origin::parse(TEST_ORIGIN).unwrap()],
        false,
        PublishKey::generate().public_jwk(),
        Secret::new(TEST_MEDIA_SECRET.into()),
    )
}

/// Captures JSON log lines so tests can assert what reached the logs.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    pub fn subscriber(&self) -> impl Subscriber + Send + Sync + 'static {
        tracing_subscriber::fmt()
            .json()
            .with_writer(self.clone())
            .finish()
    }

    pub fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

impl io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for LogBuffer {
    type Writer = LogBuffer;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
