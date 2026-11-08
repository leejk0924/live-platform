//! `PgEventBus` against Postgres: events cross instances that share only
//! the database, the largest comment fits a notification, and a lost
//! listener connection tells followers to catch up.

use std::time::{Duration, SystemTime};

use application::ports::{
    comment::CommentView,
    event_bus::{Delivery, EventBus, EventKind, EventSubscription, RealtimeEvent},
};
use domain::{
    auth::Username,
    broadcast::Version,
    comment::{COMMENT_MAX_CHARS, ChatPolicy, Comment, CommentBody, Revision},
    id::SnowflakeId,
};
use infrastructure::realtime::PgEventBus;
use sqlx::PgPool;

fn id(raw: i64) -> SnowflakeId {
    SnowflakeId::try_from(raw).unwrap()
}

fn comment(broadcast: i64, revision: i64, body: &str) -> RealtimeEvent {
    RealtimeEvent {
        broadcast: id(broadcast),
        priority: false,
        kind: EventKind::Comment(CommentView {
            comment: Comment {
                id: id(revision),
                broadcast_id: id(broadcast),
                author_id: id(3),
                body: Some(CommentBody::parse(body).unwrap()),
                version: Version::new(i32::MAX).unwrap(),
                created_at: SystemTime::UNIX_EPOCH + Duration::from_micros(1_700_000_000_000_001),
                edited: true,
            },
            author_name: Username::parse(&"w".repeat(20)).unwrap(),
            revision: Revision::new(revision).unwrap(),
        }),
    }
}

async fn next(subscription: &mut Box<dyn EventSubscription>) -> Delivery {
    tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await
        .expect("a delivery within 5 s")
        .expect("the bus is open")
}

async fn received(subscription: &mut Box<dyn EventSubscription>) -> RealtimeEvent {
    match next(subscription).await {
        Delivery::Event(event) => (*event).clone(),
        Delivery::Missed => panic!("expected an event, got Missed"),
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_event_published_on_one_instance_reaches_the_other(pool: PgPool) {
    let (a, a_listener) = PgEventBus::start(pool.clone()).await.unwrap();
    let (b, b_listener) = PgEventBus::start(pool.clone()).await.unwrap();
    let mut on_a = a.subscribe(id(1));
    let mut on_b = b.subscribe(id(1));
    let mut other_broadcast = b.subscribe(id(2));

    let event = comment(1, 10, "안녕");
    a.publish(event.clone()).await;
    let policy = RealtimeEvent {
        broadcast: id(2),
        priority: true,
        kind: EventKind::ChatPolicy(ChatPolicy::from_secs(30).unwrap()),
    };
    b.publish(policy.clone()).await;

    assert_eq!(received(&mut on_b).await, event, "across instances");
    assert_eq!(received(&mut on_a).await, event, "back to the publisher");
    assert_eq!(received(&mut other_broadcast).await, policy);

    a_listener.shutdown().await;
    b_listener.shutdown().await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_largest_comments_fit_a_notification(pool: PgPool) {
    let (bus, listener) = PgEventBus::start(pool).await.unwrap();
    let mut subscription = bus.subscribe(id(1));
    for (revision, worst) in [(1, "\""), (2, "\\"), (3, "😀")] {
        let event = comment(1, revision, &worst.repeat(COMMENT_MAX_CHARS));
        bus.publish(event.clone()).await;
        assert_eq!(received(&mut subscription).await, event, "{worst}");
    }
    listener.shutdown().await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_lost_listener_connection_is_a_miss_and_delivery_resumes(pool: PgPool) {
    let (bus, listener) = PgEventBus::start(pool.clone()).await.unwrap();
    let mut subscription = bus.subscribe(id(1));

    let terminated: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM ( \
             SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
             WHERE datname = current_database() AND pid <> pg_backend_pid() \
               AND query ILIKE 'LISTEN%' \
         ) t",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(terminated, 1, "the listener's connection");

    assert!(matches!(next(&mut subscription).await, Delivery::Missed));
    let event = comment(1, 5, "again");
    // The listener may still be reconnecting; publish until it hears.
    let heard = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            bus.publish(event.clone()).await;
            match tokio::time::timeout(Duration::from_millis(500), subscription.next()).await {
                Ok(Some(Delivery::Event(heard))) => return (*heard).clone(),
                Ok(Some(Delivery::Missed)) | Err(_) => continue,
                Ok(None) => panic!("bus closed"),
            }
        }
    })
    .await
    .expect("delivery resumes");
    assert_eq!(heard, event);
    listener.shutdown().await;
}
