//! Live event delivery. [`EventHub`] fans events out to the subscriptions
//! of this process; [`PgEventBus`] carries them between processes over
//! Postgres `NOTIFY`, so a follower connected to any API instance receives
//! what was announced on any other. Comment writes announce themselves
//! with [`notify`] inside their own transaction.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

use application::ports::{
    comment::CommentView,
    event_bus::{Delivery, EventBus, EventKind, EventSubscription, RealtimeEvent},
};
use async_trait::async_trait;
use domain::{
    auth::Username,
    broadcast::{BroadcastId, Version},
    comment::{ChatPolicy, Comment, CommentBody, Revision},
    id::SnowflakeId,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgExecutor, PgPool, postgres::PgListener};
use tokio::{
    sync::{broadcast, oneshot},
    task::JoinHandle,
};

/// The `NOTIFY` channel every instance listens on.
pub const CHANNEL: &str = "broadcast_events";

/// How many deliveries one subscription may fall behind before it is told
/// it missed some.
const SUBSCRIPTION_BUFFER: usize = 1024;

/// How long the listener waits before trying again after the database
/// refused it.
const RECONNECT_DELAY: Duration = Duration::from_secs(1);

struct Topic {
    sender: broadcast::Sender<Delivery>,
}

/// Fans each broadcast's events out to its subscriptions in this process.
/// A broadcast's entry exists only while it has a subscription.
pub struct EventHub {
    /// `None` once closed: every subscription has ended and new ones end at
    /// once.
    topics: Mutex<Option<HashMap<BroadcastId, Topic>>>,
}

impl Default for EventHub {
    fn default() -> Self {
        Self {
            topics: Mutex::new(Some(HashMap::new())),
        }
    }
}

impl EventHub {
    pub fn deliver(&self, event: RealtimeEvent) {
        let topics = self.topics.lock().expect("hub lock");
        if let Some(topic) = topics.as_ref().and_then(|t| t.get(&event.broadcast)) {
            // An error only means every receiver has just gone.
            let _ = topic.sender.send(Delivery::Event(Arc::new(event)));
        }
    }

    /// Tells the broadcast's subscriptions they may have missed events.
    pub fn miss(&self, broadcast: BroadcastId) {
        let topics = self.topics.lock().expect("hub lock");
        if let Some(topic) = topics.as_ref().and_then(|t| t.get(&broadcast)) {
            let _ = topic.sender.send(Delivery::Missed);
        }
    }

    /// Tells every subscription it may have missed events.
    pub fn miss_all(&self) {
        let topics = self.topics.lock().expect("hub lock");
        for topic in topics.iter().flat_map(HashMap::values) {
            let _ = topic.sender.send(Delivery::Missed);
        }
    }

    /// Ends every subscription, and every one taken from now on as it
    /// starts: nothing will be delivered any more.
    pub fn close(&self) {
        self.topics.lock().expect("hub lock").take();
    }

    pub fn subscribe(self: &Arc<Self>, broadcast: BroadcastId) -> Box<dyn EventSubscription> {
        let mut topics = self.topics.lock().expect("hub lock");
        let receiver = topics.as_mut().map(|topics| {
            topics
                .entry(broadcast)
                .or_insert_with(|| Topic {
                    sender: broadcast::channel(SUBSCRIPTION_BUFFER).0,
                })
                .sender
                .subscribe()
        });
        Box::new(HubSubscription {
            hub: Arc::clone(self),
            broadcast,
            receiver,
        })
    }

    /// How many broadcasts have a subscription.
    pub fn topic_count(&self) -> usize {
        self.topics
            .lock()
            .expect("hub lock")
            .as_ref()
            .map_or(0, HashMap::len)
    }
}

struct HubSubscription {
    hub: Arc<EventHub>,
    broadcast: BroadcastId,
    receiver: Option<broadcast::Receiver<Delivery>>,
}

#[async_trait]
impl EventSubscription for HubSubscription {
    async fn next(&mut self) -> Option<Delivery> {
        let receiver = self.receiver.as_mut()?;
        match receiver.recv().await {
            Ok(delivery) => Some(delivery),
            Err(broadcast::error::RecvError::Lagged(_)) => Some(Delivery::Missed),
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

impl Drop for HubSubscription {
    fn drop(&mut self) {
        let mut topics = self.hub.topics.lock().expect("hub lock");
        drop(self.receiver.take());
        let Some(topics) = topics.as_mut() else {
            return;
        };
        if topics
            .get(&self.broadcast)
            .is_some_and(|topic| topic.sender.receiver_count() == 0)
        {
            topics.remove(&self.broadcast);
        }
    }
}

/// Delivers within this process only: for a single instance and for tests.
#[derive(Default)]
pub struct LocalEventBus {
    hub: Arc<EventHub>,
}

impl LocalEventBus {
    pub fn hub(&self) -> &Arc<EventHub> {
        &self.hub
    }
}

#[async_trait]
impl EventBus for LocalEventBus {
    async fn publish(&self, event: RealtimeEvent) {
        self.hub.deliver(event);
    }

    fn subscribe(&self, broadcast: BroadcastId) -> Box<dyn EventSubscription> {
        self.hub.subscribe(broadcast)
    }
}

/// Announces `event` on [`CHANNEL`]. Postgres delivers it to every
/// listener, the announcing instance's own included, when the transaction
/// that ran this commits, and in commit order; never if it rolls back.
pub(crate) async fn notify(
    executor: impl PgExecutor<'_>,
    event: &RealtimeEvent,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(CHANNEL)
        .bind(encode(event))
        .execute(executor)
        .await
        .map(drop)
}

/// Publishes with [`notify`] and receives through one `LISTEN` connection
/// per process.
pub struct PgEventBus {
    pool: PgPool,
    hub: Arc<EventHub>,
}

/// Stops the listener of a [`PgEventBus`]. Dropping it stops the listener
/// too; so does closing the pool. Either way every subscription ends.
pub struct EventListenerHandle {
    stop: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl EventListenerHandle {
    /// Stops listening and gives the connection back to the pool. Open
    /// subscriptions then end, as do ones taken later.
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        if let Err(err) = self.task.await {
            tracing::warn!(%err, "event listener task failed");
        }
    }
}

impl PgEventBus {
    /// Listens on [`CHANNEL`] before returning, so every event published
    /// afterwards, here or on another instance, reaches this one's
    /// subscriptions. Holds one connection of `pool` until shut down.
    pub async fn start(pool: PgPool) -> Result<(Arc<Self>, EventListenerHandle), sqlx::Error> {
        let mut listener = PgListener::connect_with(&pool).await?;
        listener.listen(CHANNEL).await?;
        let hub = Arc::new(EventHub::default());
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn({
            let (pool, hub) = (pool.clone(), Arc::clone(&hub));
            async move {
                listen(pool, listener, &hub, stopped).await;
                hub.close();
            }
        });
        Ok((
            Arc::new(Self { pool, hub }),
            EventListenerHandle { stop, task },
        ))
    }

    pub fn hub(&self) -> &Arc<EventHub> {
        &self.hub
    }
}

async fn listen(
    pool: PgPool,
    mut listener: PgListener,
    hub: &EventHub,
    mut stopped: oneshot::Receiver<()>,
) {
    loop {
        let received = tokio::select! {
            biased;
            _ = &mut stopped => return,
            received = listener.try_recv() => received,
        };
        match received {
            Ok(Some(notification)) => match decode(notification.payload()) {
                Ok(event) => hub.deliver(event),
                Err(err) => tracing::warn!(%err, "undecodable broadcast event dropped"),
            },
            // The pool waits for the listener's connection before it can
            // close, so give it back.
            Err(sqlx::Error::PoolClosed) => return,
            lost => {
                if let Err(err) = lost {
                    tracing::warn!(%err, "event listener lost the database");
                }
                // Dropped first: a dead connection may linger in it.
                drop(listener);
                let Some(again) = listen_again(&pool, &mut stopped).await else {
                    return;
                };
                listener = again;
                // Only now: an event published before LISTEN took effect
                // is in the store by the time a follower catches up.
                tracing::warn!("event listener reconnected; followers catch up");
                hub.miss_all();
            }
        }
    }
}

/// A fresh connection listening on [`CHANNEL`], retried every
/// [`RECONNECT_DELAY`]; `None` once stopped or the pool closes.
async fn listen_again(pool: &PgPool, stopped: &mut oneshot::Receiver<()>) -> Option<PgListener> {
    loop {
        let attempt = async {
            let mut listener = PgListener::connect_with(pool).await?;
            listener.listen(CHANNEL).await?;
            Ok::<_, sqlx::Error>(listener)
        };
        let result = tokio::select! {
            biased;
            _ = &mut *stopped => return None,
            result = attempt => result,
        };
        match result {
            Ok(listener) => return Some(listener),
            Err(sqlx::Error::PoolClosed) => return None,
            Err(err) => tracing::warn!(%err, "event listener cannot listen again; retrying"),
        }
        tokio::select! {
            _ = &mut *stopped => return None,
            () = tokio::time::sleep(RECONNECT_DELAY) => {}
        }
    }
}

#[async_trait]
impl EventBus for PgEventBus {
    async fn publish(&self, event: RealtimeEvent) {
        let broadcast = event.broadcast;
        if let Err(err) = notify(&self.pool, &event).await {
            tracing::warn!(%err, %broadcast, "broadcast event not published");
            // Followers here catch up from the store; those on other
            // instances only at their next miss.
            self.hub.miss(broadcast);
        }
    }

    fn subscribe(&self, broadcast: BroadcastId) -> Box<dyn EventSubscription> {
        self.hub.subscribe(broadcast)
    }
}

/// The `NOTIFY` payload. Ids are strings like everywhere else in JSON;
/// times are microseconds since the epoch, the database's precision.
#[derive(Serialize, Deserialize)]
struct Wire {
    broadcast: String,
    priority: bool,
    #[serde(flatten)]
    kind: WireKind,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum WireKind {
    Comment(WireComment),
    ChatPolicy { slow_mode_secs: u32 },
}

#[derive(Serialize, Deserialize)]
struct WireComment {
    id: String,
    author_id: String,
    author_name: String,
    body: Option<String>,
    version: i32,
    created_at_us: i64,
    edited: bool,
    revision: i64,
}

fn encode(event: &RealtimeEvent) -> String {
    let kind = match &event.kind {
        EventKind::Comment(view) => {
            let c = &view.comment;
            WireKind::Comment(WireComment {
                id: c.id.to_string(),
                author_id: c.author_id.to_string(),
                author_name: view.author_name.as_str().to_owned(),
                body: c.body.as_ref().map(|body| body.as_str().to_owned()),
                version: c.version.get(),
                created_at_us: c.created_at.duration_since(UNIX_EPOCH).map_or(0, |since| {
                    i64::try_from(since.as_micros()).unwrap_or(i64::MAX)
                }),
                edited: c.edited,
                revision: view.revision.get(),
            })
        }
        EventKind::ChatPolicy(policy) => WireKind::ChatPolicy {
            slow_mode_secs: policy.slow_mode_secs(),
        },
    };
    let wire = Wire {
        broadcast: event.broadcast.to_string(),
        priority: event.priority,
        kind,
    };
    serde_json::to_string(&wire).expect("strings and numbers always serialize")
}

fn decode(payload: &str) -> Result<RealtimeEvent, String> {
    let wire: Wire = serde_json::from_str(payload).map_err(|err| err.to_string())?;
    let id = |raw: &str| {
        raw.parse::<SnowflakeId>()
            .map_err(|_| format!("bad id {raw}"))
    };
    let broadcast = id(&wire.broadcast)?;
    let kind = match wire.kind {
        WireKind::Comment(c) => EventKind::Comment(CommentView {
            comment: Comment {
                id: id(&c.id)?,
                broadcast_id: broadcast,
                author_id: id(&c.author_id)?,
                body: c
                    .body
                    .map(|body| CommentBody::parse(&body).map_err(|err| err.to_string()))
                    .transpose()?,
                version: Version::new(c.version).ok_or("bad version")?,
                created_at: UNIX_EPOCH
                    + Duration::from_micros(u64::try_from(c.created_at_us).unwrap_or(0)),
                edited: c.edited,
            },
            author_name: Username::parse(&c.author_name).map_err(|err| err.to_string())?,
            revision: Revision::new(c.revision).ok_or("bad revision")?,
        }),
        WireKind::ChatPolicy { slow_mode_secs } => EventKind::ChatPolicy(
            ChatPolicy::from_secs(slow_mode_secs).map_err(|err| err.to_string())?,
        ),
    };
    Ok(RealtimeEvent {
        broadcast,
        priority: wire.priority,
        kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    fn comment_event(broadcast: i64, revision: i64, body: Option<&str>) -> RealtimeEvent {
        RealtimeEvent {
            broadcast: id(broadcast),
            priority: false,
            kind: EventKind::Comment(CommentView {
                comment: Comment {
                    id: id(9_007_199_254_740_993),
                    broadcast_id: id(broadcast),
                    author_id: id(3),
                    body: body.map(|b| CommentBody::parse(b).unwrap()),
                    version: Version::new(2).unwrap(),
                    created_at: UNIX_EPOCH + Duration::from_micros(1_700_000_000_123_456),
                    edited: true,
                },
                author_name: Username::parse("viewer").unwrap(),
                revision: Revision::new(revision).unwrap(),
            }),
        }
    }

    #[test]
    fn events_survive_the_wire() {
        for event in [
            comment_event(5, 11, Some("안녕 \"하세요\" \\ 😀")),
            comment_event(5, 12, None),
            RealtimeEvent {
                broadcast: id(5),
                priority: true,
                kind: EventKind::ChatPolicy(ChatPolicy::from_secs(30).unwrap()),
            },
        ] {
            assert_eq!(decode(&encode(&event)).unwrap(), event);
        }
        assert!(decode("{}").is_err());
    }

    /// The JSON of a body grows most with characters that take four UTF-8
    /// bytes or that JSON escapes; control characters never get this far.
    #[test]
    fn the_largest_comment_payload_is_under_the_notify_limit() {
        for worst in ["\"", "\\", "😀"] {
            let body = worst.repeat(domain::comment::COMMENT_MAX_CHARS);
            let mut event = comment_event(i64::MAX, i64::MAX, Some(&body));
            if let EventKind::Comment(view) = &mut event.kind {
                view.author_name = Username::parse(&"w".repeat(20)).unwrap();
                view.comment.version = Version::new(i32::MAX).unwrap();
            }
            let bytes = encode(&event).len();
            assert!((1000..8000).contains(&bytes), "{worst}: {bytes}");
        }
    }

    async fn next(subscription: &mut Box<dyn EventSubscription>) -> Delivery {
        tokio::time::timeout(Duration::from_secs(1), subscription.next())
            .await
            .expect("a delivery")
            .expect("the hub is open")
    }

    fn revision_of(delivery: Delivery) -> i64 {
        match delivery {
            Delivery::Event(event) => match &event.kind {
                EventKind::Comment(view) => view.revision.get(),
                EventKind::ChatPolicy(_) => panic!("expected a comment"),
            },
            Delivery::Missed => panic!("expected an event"),
        }
    }

    #[tokio::test]
    async fn hub_delivers_to_the_broadcasts_subscribers_only() {
        let hub = Arc::new(EventHub::default());
        let mut a1 = hub.subscribe(id(1));
        let mut a2 = hub.subscribe(id(1));
        let mut b = hub.subscribe(id(2));
        hub.deliver(comment_event(1, 10, Some("x")));
        hub.deliver(comment_event(2, 20, Some("y")));

        assert_eq!(revision_of(next(&mut a1).await), 10);
        assert_eq!(revision_of(next(&mut a2).await), 10);
        assert_eq!(revision_of(next(&mut b).await), 20);
    }

    #[tokio::test]
    async fn a_lagging_subscriber_is_told_it_missed_events() {
        let hub = Arc::new(EventHub::default());
        let mut slow = hub.subscribe(id(1));
        for n in 1..=(SUBSCRIPTION_BUFFER as i64 + 1) {
            hub.deliver(comment_event(1, n, Some("x")));
        }
        assert!(matches!(next(&mut slow).await, Delivery::Missed));
    }

    #[tokio::test]
    async fn miss_all_reaches_every_subscriber() {
        let hub = Arc::new(EventHub::default());
        let mut a = hub.subscribe(id(1));
        let mut b = hub.subscribe(id(2));
        hub.miss_all();
        assert!(matches!(next(&mut a).await, Delivery::Missed));
        assert!(matches!(next(&mut b).await, Delivery::Missed));
    }

    #[test]
    fn a_broadcast_is_forgotten_when_its_last_subscription_drops() {
        let hub = Arc::new(EventHub::default());
        let a = hub.subscribe(id(1));
        let b = hub.subscribe(id(1));
        assert_eq!(hub.topic_count(), 1);
        drop(a);
        assert_eq!(hub.topic_count(), 1);
        drop(b);
        assert_eq!(hub.topic_count(), 0);
    }
}
