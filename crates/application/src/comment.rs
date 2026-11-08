//! Posting, editing, deleting and following comments. The author is always
//! the authenticated actor, so a request can never write as someone else or
//! change someone else's comment by naming them.

use std::{collections::BTreeSet, sync::Arc, time::Duration};

use domain::{
    auth::UserId,
    broadcast::{BroadcastId, Version},
    comment::{
        ChatPolicy, CommentBody, CommentError, CommentId, PAGE_SIZE_DEFAULT, PAGE_SIZE_MAX,
        POST_INTERVAL, Revision, accepts_comments,
    },
};
use thiserror::Error;

use crate::ports::{
    PortError,
    comment::{
        ChatPolicyStore, ChatTarget, CommentQuery, CommentRepository, CommentView,
        CommentWriteRejection, NewComment, PolicyRejection,
    },
    event_bus::{Delivery, EventBus, EventKind, EventSubscription, RealtimeEvent},
    id::{IdGenerationError, IdGenerator},
    rate_limit::{RateKey, RateLimiter},
};

/// The most changes a follower catches up on at once. A follower further
/// behind is told to reload instead.
pub const FEED_CATCH_UP_LIMIT: u32 = 500;

/// How many caught-up revisions a feed remembers to skip their live copies.
const CAUGHT_UP_KEPT: usize = 2 * FEED_CATCH_UP_LIMIT as usize;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Rejected(#[from] CommentError),
    #[error("posting too fast; retry in {0:?}")]
    RateLimited(Duration),
    #[error(transparent)]
    Internal(#[from] PortError),
}

impl From<IdGenerationError> for Error {
    fn from(err: IdGenerationError) -> Self {
        Self::Internal(PortError(err.to_string()))
    }
}

impl From<CommentWriteRejection> for Error {
    fn from(rejection: CommentWriteRejection) -> Self {
        Self::Rejected(match rejection {
            CommentWriteRejection::NotFound => CommentError::NotFound,
            CommentWriteRejection::NotAuthor => CommentError::NotAuthor,
            CommentWriteRejection::VersionMismatch => CommentError::VersionConflict,
        })
    }
}

impl From<PolicyRejection> for Error {
    fn from(rejection: PolicyRejection) -> Self {
        Self::Rejected(match rejection {
            PolicyRejection::NotFound => CommentError::BroadcastNotFound,
            PolicyRejection::NotOwner => CommentError::NotBroadcastOwner,
        })
    }
}

async fn target_of(
    policies: &dyn ChatPolicyStore,
    broadcast: BroadcastId,
) -> Result<ChatTarget, Error> {
    policies
        .target(broadcast)
        .await?
        .ok_or_else(|| CommentError::BroadcastNotFound.into())
}

async fn announce(events: &dyn EventBus, view: &CommentView, priority: bool) {
    events
        .publish(RealtimeEvent {
            broadcast: view.comment.broadcast_id,
            priority,
            kind: EventKind::Comment(view.clone()),
        })
        .await;
}

pub struct PostComment {
    pub ids: Arc<dyn IdGenerator>,
    pub comments: Arc<dyn CommentRepository>,
    pub policies: Arc<dyn ChatPolicyStore>,
    pub limiter: Arc<dyn RateLimiter>,
    pub events: Arc<dyn EventBus>,
}

impl PostComment {
    pub async fn run(
        &self,
        actor: UserId,
        broadcast: BroadcastId,
        body: &str,
    ) -> Result<CommentView, Error> {
        let body = CommentBody::parse(body)?;
        let target = target_of(self.policies.as_ref(), broadcast).await?;
        if !accepts_comments(target.status) {
            return Err(CommentError::Closed.into());
        }
        let is_owner = actor == target.owner;
        let mut slots = vec![(RateKey::User(actor), POST_INTERVAL)];
        if let Some(slow_mode) = target.policy.slow_mode_for(is_owner) {
            slots.push((RateKey::SlowMode(actor, broadcast), slow_mode));
        }
        self.limiter.acquire(&slots).map_err(Error::RateLimited)?;
        let new = NewComment {
            id: self.ids.next_id()?,
            broadcast_id: broadcast,
            author_id: actor,
            body,
        };
        // The broadcast ended between the read above and this write.
        let view = self
            .comments
            .insert(&new)
            .await?
            .ok_or(CommentError::Closed)?;
        announce(self.events.as_ref(), &view, is_owner).await;
        Ok(view)
    }
}

pub struct EditComment {
    pub comments: Arc<dyn CommentRepository>,
    pub events: Arc<dyn EventBus>,
}

impl EditComment {
    pub async fn run(
        &self,
        actor: UserId,
        broadcast: BroadcastId,
        id: CommentId,
        expected: Version,
        body: &str,
    ) -> Result<CommentView, Error> {
        let body = CommentBody::parse(body)?;
        let view = self
            .comments
            .edit(broadcast, id, actor, expected, &body)
            .await??;
        announce(self.events.as_ref(), &view, false).await;
        Ok(view)
    }
}

pub struct DeleteComment {
    pub comments: Arc<dyn CommentRepository>,
    pub events: Arc<dyn EventBus>,
}

impl DeleteComment {
    pub async fn run(
        &self,
        actor: UserId,
        broadcast: BroadcastId,
        id: CommentId,
    ) -> Result<(), Error> {
        let tombstone = self.comments.soft_delete(broadcast, id, actor).await??;
        announce(self.events.as_ref(), &tombstone, false).await;
        Ok(())
    }
}

#[derive(Debug)]
pub struct Page {
    pub items: Vec<CommentView>,
    /// Pass as `before` to get the next page; `None` on the last page.
    pub next_before: Option<CommentId>,
    /// Read before the items: following the feed after it misses nothing
    /// the page lacks. [`Revision::START`] before the first comment.
    pub revision: Revision,
}

pub struct ListComments {
    pub query: Arc<dyn CommentQuery>,
    pub policies: Arc<dyn ChatPolicyStore>,
}

impl ListComments {
    pub async fn run(
        &self,
        broadcast: BroadcastId,
        before: Option<CommentId>,
        size: Option<u32>,
    ) -> Result<Page, Error> {
        let size = size.unwrap_or(PAGE_SIZE_DEFAULT);
        if !(1..=PAGE_SIZE_MAX).contains(&size) {
            return Err(CommentError::InvalidPageSize.into());
        }
        target_of(self.policies.as_ref(), broadcast).await?;
        let revision = self.query.latest_revision(broadcast).await?;
        // One extra row tells whether a next page exists.
        let mut items = self.query.list(broadcast, before, size + 1).await?;
        let has_more = items.len() > size as usize;
        items.truncate(size as usize);
        let next_before = if has_more {
            items.last().map(|view| view.comment.id)
        } else {
            None
        };
        Ok(Page {
            items,
            next_before,
            revision,
        })
    }
}

pub struct GetChatPolicy {
    pub policies: Arc<dyn ChatPolicyStore>,
}

impl GetChatPolicy {
    pub async fn run(&self, broadcast: BroadcastId) -> Result<ChatPolicy, Error> {
        Ok(target_of(self.policies.as_ref(), broadcast).await?.policy)
    }
}

pub struct SetChatPolicy {
    pub policies: Arc<dyn ChatPolicyStore>,
    pub events: Arc<dyn EventBus>,
}

impl SetChatPolicy {
    pub async fn run(
        &self,
        actor: UserId,
        broadcast: BroadcastId,
        slow_mode_secs: u32,
    ) -> Result<ChatPolicy, Error> {
        let policy = ChatPolicy::from_secs(slow_mode_secs)?;
        self.policies.set(broadcast, actor, policy).await??;
        self.events
            .publish(RealtimeEvent {
                broadcast,
                priority: true,
                kind: EventKind::ChatPolicy(policy),
            })
            .await;
        Ok(policy)
    }
}

/// What a follower of a broadcast's comments receives next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeedItem {
    /// Changed comments, by ascending revision when they come from the
    /// store. A client keeps, per comment id, the one with the highest
    /// revision.
    Comments {
        changes: Vec<CommentView>,
        priority: bool,
    },
    Policy(ChatPolicy),
    /// More changed than one catch-up holds: reload the list. The feed goes
    /// on from `cursor`, the newest revision when the reset was decided.
    Reset {
        cursor: Revision,
    },
}

pub struct FollowComments {
    pub query: Arc<dyn CommentQuery>,
    pub policies: Arc<dyn ChatPolicyStore>,
    pub events: Arc<dyn EventBus>,
}

impl FollowComments {
    /// Follows from `after` (a revision the client already holds), or from
    /// now when `None`.
    pub async fn open(
        &self,
        broadcast: BroadcastId,
        after: Option<Revision>,
    ) -> Result<CommentFeed, Error> {
        target_of(self.policies.as_ref(), broadcast).await?;
        // Subscribe before reading the store, so nothing written in between
        // falls through the gap.
        let subscription = self.events.subscribe(broadcast);
        let latest = self.query.latest_revision(broadcast).await?;
        // A resume point the store never reached (another database, or a
        // forged header) would wait for revisions that already exist.
        let (cursor, behind, queued) = match after {
            Some(after) if after > latest => {
                (latest, false, Some(FeedItem::Reset { cursor: latest }))
            }
            Some(after) => (after, true, None),
            None => (latest, false, None),
        };
        Ok(CommentFeed {
            broadcast,
            query: Arc::clone(&self.query),
            policies: Arc::clone(&self.policies),
            subscription,
            cursor,
            behind,
            policy_due: true,
            queued,
            caught_up: BTreeSet::new(),
        })
    }
}

/// One follower's view of a broadcast's live events, made gap-free by
/// catching up from the store whenever the bus may have lost some.
pub struct CommentFeed {
    broadcast: BroadcastId,
    query: Arc<dyn CommentQuery>,
    policies: Arc<dyn ChatPolicyStore>,
    subscription: Box<dyn EventSubscription>,
    /// The newest revision delivered.
    cursor: Revision,
    /// Events may be missing after `cursor`; the next call catches up first.
    behind: bool,
    /// The policy is sent when the feed opens and after every catch-up,
    /// since a missed event may have changed it.
    policy_due: bool,
    /// Delivered before anything else.
    queued: Option<FeedItem>,
    /// Revisions catch-ups delivered whose live events are still to come
    /// and must not be delivered twice, at most [`CAUGHT_UP_KEPT`] of the
    /// newest. A set rather than "at most the cursor": each write publishes
    /// after its own commit, so a lower revision's event may arrive after a
    /// higher one's. A copy forgotten here is delivered twice, which a
    /// client keeping the highest revision per comment ignores.
    caught_up: BTreeSet<Revision>,
}

impl CommentFeed {
    /// The next item, or `None` once the bus has shut down. Dropping the
    /// future before it completes loses nothing.
    pub async fn next(&mut self) -> Result<Option<FeedItem>, PortError> {
        if let Some(item) = self.queued.take() {
            return Ok(Some(item));
        }
        loop {
            if self.behind {
                let item = self.catch_up().await?;
                self.behind = false;
                self.policy_due = true;
                if let Some(item) = item {
                    return Ok(Some(item));
                }
                continue;
            }
            if self.policy_due {
                let target = self.policies.target(self.broadcast).await?;
                self.policy_due = false;
                if let Some(target) = target {
                    return Ok(Some(FeedItem::Policy(target.policy)));
                }
                continue;
            }
            match self.subscription.next().await {
                None => return Ok(None),
                Some(Delivery::Missed) => self.behind = true,
                Some(Delivery::Event(event)) => {
                    if let Some(item) = self.accept(&event) {
                        return Ok(Some(item));
                    }
                }
            }
        }
    }

    fn accept(&mut self, event: &RealtimeEvent) -> Option<FeedItem> {
        match &event.kind {
            EventKind::ChatPolicy(policy) => Some(FeedItem::Policy(*policy)),
            EventKind::Comment(view) => {
                if self.caught_up.remove(&view.revision) {
                    return None;
                }
                self.cursor = self.cursor.max(view.revision);
                Some(FeedItem::Comments {
                    changes: vec![view.clone()],
                    priority: event.priority,
                })
            }
        }
    }

    async fn catch_up(&mut self) -> Result<Option<FeedItem>, PortError> {
        // One more than fits tells "exactly the limit" from "too many".
        let changes = self
            .query
            .changes_since(self.broadcast, self.cursor, FEED_CATCH_UP_LIMIT + 1)
            .await?;
        if changes.len() > FEED_CATCH_UP_LIMIT as usize {
            let cursor = self.query.latest_revision(self.broadcast).await?;
            self.cursor = cursor;
            self.caught_up.clear();
            return Ok(Some(FeedItem::Reset { cursor }));
        }
        let Some(last) = changes.last() else {
            return Ok(None);
        };
        self.cursor = self.cursor.max(last.revision);
        self.caught_up
            .extend(changes.iter().map(|view| view.revision));
        while self.caught_up.len() > CAUGHT_UP_KEPT {
            self.caught_up.pop_first();
        }
        Ok(Some(FeedItem::Comments {
            changes,
            priority: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::Mutex,
        time::{Duration, SystemTime},
    };

    use async_trait::async_trait;
    use domain::{
        auth::Username,
        broadcast::BroadcastStatus,
        comment::{Comment, CommentBody},
        id::SnowflakeId,
    };
    use tokio::sync::mpsc;

    use super::*;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    const BROADCAST: i64 = 100;
    const OWNER: i64 = 7;
    const VIEWER: i64 = 8;

    /// A comment whose id, revision and body are all `n`.
    fn view(n: i64) -> CommentView {
        CommentView {
            comment: Comment {
                id: id(n),
                broadcast_id: id(BROADCAST),
                author_id: id(VIEWER),
                body: Some(CommentBody::parse(&n.to_string()).unwrap()),
                version: Version::FIRST,
                created_at: SystemTime::UNIX_EPOCH,
                edited: false,
            },
            author_name: Username::parse("viewer").unwrap(),
            revision: Revision::new(n).unwrap(),
        }
    }

    fn revisions(item: &FeedItem) -> Vec<i64> {
        match item {
            FeedItem::Comments { changes, .. } => {
                changes.iter().map(|view| view.revision.get()).collect()
            }
            other => panic!("expected comments, got {other:?}"),
        }
    }

    /// Holds the comments with revisions `1..=stored` of one open broadcast.
    struct Store {
        stored: Mutex<i64>,
        policy: ChatPolicy,
        status: BroadcastStatus,
    }

    impl Store {
        fn with(stored: i64) -> Arc<Self> {
            Arc::new(Self {
                stored: Mutex::new(stored),
                policy: ChatPolicy::OFF,
                status: BroadcastStatus::Live,
            })
        }
    }

    #[async_trait]
    impl CommentQuery for Store {
        async fn list(
            &self,
            _: BroadcastId,
            before: Option<CommentId>,
            limit: u32,
        ) -> Result<Vec<CommentView>, PortError> {
            let top = before.map_or(*self.stored.lock().unwrap(), |b| b.as_i64() - 1);
            Ok((1..=top).rev().take(limit as usize).map(view).collect())
        }

        async fn changes_since(
            &self,
            _: BroadcastId,
            after: Revision,
            limit: u32,
        ) -> Result<Vec<CommentView>, PortError> {
            let from = after.get() + 1;
            let to = *self.stored.lock().unwrap();
            Ok((from..=to).take(limit as usize).map(view).collect())
        }

        async fn latest_revision(&self, _: BroadcastId) -> Result<Revision, PortError> {
            Ok(Revision::new(*self.stored.lock().unwrap()).unwrap())
        }
    }

    #[async_trait]
    impl ChatPolicyStore for Store {
        async fn target(&self, broadcast: BroadcastId) -> Result<Option<ChatTarget>, PortError> {
            Ok((broadcast == id(BROADCAST)).then_some(ChatTarget {
                owner: id(OWNER),
                status: self.status,
                policy: self.policy,
            }))
        }

        async fn set(
            &self,
            _: BroadcastId,
            _: UserId,
            _: ChatPolicy,
        ) -> Result<Result<(), PolicyRejection>, PortError> {
            unreachable!()
        }
    }

    /// A bus the test drives by hand: what it sends is what the one
    /// subscriber receives.
    struct Bus {
        subscriber: Mutex<Option<mpsc::UnboundedReceiver<Delivery>>>,
        published: Mutex<Vec<RealtimeEvent>>,
    }

    impl Bus {
        fn new() -> (Arc<Self>, mpsc::UnboundedSender<Delivery>) {
            let (tx, rx) = mpsc::unbounded_channel();
            let bus = Arc::new(Self {
                subscriber: Mutex::new(Some(rx)),
                published: Mutex::default(),
            });
            (bus, tx)
        }
    }

    struct Subscription(mpsc::UnboundedReceiver<Delivery>);

    #[async_trait]
    impl EventSubscription for Subscription {
        async fn next(&mut self) -> Option<Delivery> {
            self.0.recv().await
        }
    }

    #[async_trait]
    impl EventBus for Bus {
        async fn publish(&self, event: RealtimeEvent) {
            self.published.lock().unwrap().push(event);
        }

        fn subscribe(&self, _: BroadcastId) -> Box<dyn EventSubscription> {
            Box::new(Subscription(
                self.subscriber
                    .lock()
                    .unwrap()
                    .take()
                    .expect("one subscriber"),
            ))
        }
    }

    fn live(n: i64) -> Delivery {
        Delivery::Event(Arc::new(RealtimeEvent {
            broadcast: id(BROADCAST),
            priority: false,
            kind: EventKind::Comment(view(n)),
        }))
    }

    async fn open(
        store: &Arc<Store>,
        bus: &Arc<Bus>,
        after: Option<i64>,
    ) -> Result<CommentFeed, Error> {
        FollowComments {
            query: store.clone(),
            policies: store.clone(),
            events: bus.clone(),
        }
        .open(id(BROADCAST), after.and_then(Revision::new))
        .await
    }

    /// The next item, skipping the re-sent policy of a store whose chat
    /// is unrestricted; the policy tests use another.
    async fn next(feed: &mut CommentFeed) -> FeedItem {
        loop {
            let item = tokio::time::timeout(Duration::from_secs(1), feed.next())
                .await
                .expect("an item")
                .unwrap()
                .expect("the bus is open");
            if item != FeedItem::Policy(ChatPolicy::OFF) {
                return item;
            }
        }
    }

    #[tokio::test]
    async fn backlog_comes_first_and_its_live_events_are_not_repeated() {
        let store = Store::with(5);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, Some(3)).await.unwrap();
        for n in [4, 5, 6] {
            tx.send(live(n)).unwrap();
        }

        assert_eq!(revisions(&next(&mut feed).await), [4, 5]);
        assert_eq!(revisions(&next(&mut feed).await), [6]);
    }

    #[tokio::test]
    async fn a_late_lower_revision_is_still_delivered() {
        let store = Store::with(5);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, None).await.unwrap();
        tx.send(live(7)).unwrap();
        tx.send(live(6)).unwrap();

        assert_eq!(revisions(&next(&mut feed).await), [7]);
        assert_eq!(revisions(&next(&mut feed).await), [6]);
    }

    #[tokio::test]
    async fn missed_events_are_caught_up_from_the_cursor() {
        let store = Store::with(2);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, None).await.unwrap();
        tx.send(live(3)).unwrap();
        assert_eq!(revisions(&next(&mut feed).await), [3]);

        *store.stored.lock().unwrap() = 6;
        tx.send(Delivery::Missed).unwrap();
        tx.send(live(6)).unwrap();
        tx.send(live(7)).unwrap();

        assert_eq!(revisions(&next(&mut feed).await), [4, 5, 6]);
        assert_eq!(revisions(&next(&mut feed).await), [7]);
    }

    #[tokio::test]
    async fn live_copies_of_an_earlier_catch_up_are_not_repeated() {
        let store = Store::with(3);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, Some(1)).await.unwrap();
        assert_eq!(revisions(&next(&mut feed).await), [2, 3]);

        *store.stored.lock().unwrap() = 5;
        tx.send(Delivery::Missed).unwrap();
        assert_eq!(revisions(&next(&mut feed).await), [4, 5]);

        for n in [2, 3, 4, 5, 6] {
            tx.send(live(n)).unwrap();
        }
        assert_eq!(revisions(&next(&mut feed).await), [6]);
    }

    #[tokio::test]
    async fn a_follower_from_an_empty_chat_catches_up_from_the_start() {
        let store = Store::with(0);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, None).await.unwrap();

        *store.stored.lock().unwrap() = 2;
        tx.send(Delivery::Missed).unwrap();

        assert_eq!(revisions(&next(&mut feed).await), [1, 2]);
    }

    #[tokio::test]
    async fn too_far_behind_is_a_reset_to_the_newest_revision() {
        let limit = i64::from(FEED_CATCH_UP_LIMIT);
        let store = Store::with(limit + 10);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, Some(1)).await.unwrap();

        let newest = Revision::new(limit + 10).unwrap();
        assert_eq!(next(&mut feed).await, FeedItem::Reset { cursor: newest });

        *store.stored.lock().unwrap() = limit + 12;
        tx.send(Delivery::Missed).unwrap();
        assert_eq!(
            revisions(&next(&mut feed).await),
            [limit + 11, limit + 12],
            "catches up from the reset point"
        );
    }

    #[tokio::test]
    async fn exactly_the_limit_is_caught_up_and_one_more_is_a_reset() {
        let limit = i64::from(FEED_CATCH_UP_LIMIT);
        let (bus, _tx) = Bus::new();
        let mut feed = open(&Store::with(limit), &bus, Some(0)).await.unwrap();
        assert_eq!(
            revisions(&next(&mut feed).await).len(),
            FEED_CATCH_UP_LIMIT as usize
        );

        let (bus, _tx) = Bus::new();
        let mut feed = open(&Store::with(limit + 1), &bus, Some(0)).await.unwrap();
        assert_eq!(
            next(&mut feed).await,
            FeedItem::Reset {
                cursor: Revision::new(limit + 1).unwrap()
            }
        );
    }

    #[tokio::test]
    async fn a_resume_point_past_the_newest_revision_is_a_reset_to_it() {
        let store = Store::with(5);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, Some(9)).await.unwrap();
        assert_eq!(
            next(&mut feed).await,
            FeedItem::Reset {
                cursor: Revision::new(5).unwrap()
            }
        );

        *store.stored.lock().unwrap() = 6;
        tx.send(Delivery::Missed).unwrap();
        assert_eq!(
            revisions(&next(&mut feed).await),
            [6],
            "followed from there"
        );
    }

    #[tokio::test]
    async fn the_policy_is_sent_on_open_and_after_every_miss() {
        let slow = ChatPolicy::from_secs(10).unwrap();
        let store = Arc::new(Store {
            stored: Mutex::new(2),
            policy: slow,
            status: BroadcastStatus::Live,
        });
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, Some(1)).await.unwrap();
        assert_eq!(revisions(&next(&mut feed).await), [2]);
        assert_eq!(next(&mut feed).await, FeedItem::Policy(slow));

        tx.send(Delivery::Missed).unwrap();
        assert_eq!(
            next(&mut feed).await,
            FeedItem::Policy(slow),
            "also when nothing was missed"
        );
    }

    #[tokio::test]
    async fn policy_events_pass_through_and_a_closed_bus_ends_the_feed() {
        let store = Store::with(0);
        let (bus, tx) = Bus::new();
        let mut feed = open(&store, &bus, None).await.unwrap();
        let slow = ChatPolicy::from_secs(10).unwrap();
        tx.send(Delivery::Event(Arc::new(RealtimeEvent {
            broadcast: id(BROADCAST),
            priority: true,
            kind: EventKind::ChatPolicy(slow),
        })))
        .unwrap();
        drop(tx);

        assert_eq!(next(&mut feed).await, FeedItem::Policy(slow));
        assert_eq!(feed.next().await.unwrap(), None);
    }

    #[tokio::test]
    async fn following_an_unknown_broadcast_is_not_found() {
        let store = Store::with(0);
        let (bus, _tx) = Bus::new();
        let result = FollowComments {
            query: store.clone(),
            policies: store,
            events: bus,
        }
        .open(id(999), None)
        .await;
        assert!(matches!(
            result,
            Err(Error::Rejected(CommentError::BroadcastNotFound))
        ));
    }

    /// Remembers which slots were asked for and refuses when told to.
    #[derive(Default)]
    struct Limiter {
        asked: Mutex<Vec<Vec<(RateKey, Duration)>>>,
        refuse: Option<Duration>,
    }

    impl RateLimiter for Limiter {
        fn acquire(&self, slots: &[(RateKey, Duration)]) -> Result<(), Duration> {
            self.asked.lock().unwrap().push(slots.to_vec());
            self.refuse.map_or(Ok(()), Err)
        }
    }

    struct Ids;

    impl IdGenerator for Ids {
        fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
            Ok(id(42))
        }
    }

    /// Stores every comment it is given.
    struct Comments;

    #[async_trait]
    impl CommentRepository for Comments {
        async fn insert(&self, new: &NewComment) -> Result<Option<CommentView>, PortError> {
            let mut stored = view(1);
            stored.comment.id = new.id;
            stored.comment.author_id = new.author_id;
            stored.comment.body = Some(new.body.clone());
            Ok(Some(stored))
        }

        async fn edit(
            &self,
            _: BroadcastId,
            _: CommentId,
            _: UserId,
            _: Version,
            _: &CommentBody,
        ) -> Result<Result<CommentView, CommentWriteRejection>, PortError> {
            unreachable!()
        }

        async fn soft_delete(
            &self,
            _: BroadcastId,
            _: CommentId,
            _: UserId,
        ) -> Result<Result<CommentView, CommentWriteRejection>, PortError> {
            unreachable!()
        }
    }

    struct Posting {
        store: Arc<Store>,
        limiter: Arc<Limiter>,
        bus: Arc<Bus>,
    }

    impl Posting {
        fn new(policy: ChatPolicy, status: BroadcastStatus, refuse: Option<Duration>) -> Self {
            Self {
                store: Arc::new(Store {
                    stored: Mutex::new(0),
                    policy,
                    status,
                }),
                limiter: Arc::new(Limiter {
                    refuse,
                    ..Limiter::default()
                }),
                bus: Bus::new().0,
            }
        }

        async fn post(&self, actor: i64, body: &str) -> Result<CommentView, Error> {
            PostComment {
                ids: Arc::new(Ids),
                comments: Arc::new(Comments),
                policies: self.store.clone(),
                limiter: self.limiter.clone(),
                events: self.bus.clone(),
            }
            .run(id(actor), id(BROADCAST), body)
            .await
        }

        fn asked(&self) -> Vec<Vec<(RateKey, Duration)>> {
            self.limiter.asked.lock().unwrap().clone()
        }

        fn published_priorities(&self) -> Vec<bool> {
            self.bus
                .published
                .lock()
                .unwrap()
                .iter()
                .map(|event| event.priority)
                .collect()
        }
    }

    #[tokio::test]
    async fn viewers_in_slow_mode_take_both_slots_and_the_owner_only_their_own() {
        let slow = ChatPolicy::from_secs(30).unwrap();
        let posting = Posting::new(slow, BroadcastStatus::Live, None);
        posting.post(VIEWER, "hi").await.unwrap();
        posting.post(OWNER, "welcome").await.unwrap();

        assert_eq!(
            posting.asked(),
            [
                vec![
                    (RateKey::User(id(VIEWER)), POST_INTERVAL),
                    (
                        RateKey::SlowMode(id(VIEWER), id(BROADCAST)),
                        Duration::from_secs(30)
                    ),
                ],
                vec![(RateKey::User(id(OWNER)), POST_INTERVAL)],
            ]
        );
        assert_eq!(
            posting.published_priorities(),
            [false, true],
            "the owner's comment is sent at once"
        );
    }

    #[tokio::test]
    async fn a_refused_slot_is_a_rate_limit_and_nothing_is_published() {
        let wait = Duration::from_millis(700);
        let posting = Posting::new(ChatPolicy::OFF, BroadcastStatus::Live, Some(wait));
        assert!(matches!(
            posting.post(VIEWER, "hi").await,
            Err(Error::RateLimited(w)) if w == wait
        ));
        assert!(posting.published_priorities().is_empty());
    }

    #[tokio::test]
    async fn an_ended_broadcast_and_an_invalid_body_take_no_slot() {
        let ended = Posting::new(ChatPolicy::OFF, BroadcastStatus::Ended, None);
        assert!(matches!(
            ended.post(VIEWER, "hi").await,
            Err(Error::Rejected(CommentError::Closed))
        ));
        let live = Posting::new(ChatPolicy::OFF, BroadcastStatus::Live, None);
        assert!(matches!(
            live.post(VIEWER, " ").await,
            Err(Error::Rejected(CommentError::InvalidBody))
        ));
        assert!(ended.asked().is_empty() && live.asked().is_empty());
    }

    async fn page(total: i64, before: Option<i64>, size: Option<u32>) -> Result<Page, Error> {
        let store = Store::with(total);
        ListComments {
            query: store.clone(),
            policies: store,
        }
        .run(id(BROADCAST), before.map(id), size)
        .await
    }

    #[tokio::test]
    async fn list_pages_by_id_and_reports_the_newest_revision() {
        let first = page(120, None, None).await.unwrap();
        assert_eq!(first.items.len(), 50);
        assert_eq!(first.next_before, Some(id(71)));
        assert_eq!(Some(first.revision), Revision::new(120));

        let last = page(3, Some(2), Some(5)).await.unwrap();
        assert_eq!(last.items.len(), 1);
        assert_eq!(last.next_before, None);

        assert_eq!(page(0, None, None).await.unwrap().revision, Revision::START);
        for bad in [0, 101] {
            assert!(matches!(
                page(10, None, Some(bad)).await,
                Err(Error::Rejected(CommentError::InvalidPageSize))
            ));
        }
        assert_eq!(page(200, None, Some(100)).await.unwrap().items.len(), 100);
    }
}
