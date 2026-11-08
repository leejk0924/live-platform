//! The donation adapters against Postgres: an order is credited exactly
//! once however many confirmations race, a payment pays for one order, and
//! the conditions on creating an order hold in SQL.

use std::{sync::Arc, time::Duration};

use application::{
    donation::{ConfirmDonation, Error},
    ports::{
        donation::{DonationQuery, DonationRepository, DonationSummary, NewDonation, Settlement},
        event_bus::{Delivery, EventBus, EventKind, EventSubscription},
    },
};
use domain::{
    donation::{DonationAmount, DonationError, PaymentKey},
    id::SnowflakeId,
};
use infrastructure::{
    payment::FakePaymentGateway,
    persistence::{PgDonationQuery, PgDonationRepository},
    realtime::PgEventBus,
};
use sqlx::PgPool;

const OWNER: i64 = 1;
const DONOR: i64 = 2;
const BROADCAST: i64 = 10;
const AMOUNT: i64 = 5_000;

fn id(raw: i64) -> SnowflakeId {
    SnowflakeId::try_from(raw).unwrap()
}

fn key(raw: &str) -> PaymentKey {
    PaymentKey::parse(raw).unwrap()
}

async fn seed(pool: &PgPool, status: &str) {
    for (user, name) in [(OWNER, "owner"), (DONOR, "donor")] {
        sqlx::query("INSERT INTO users (id, username, is_demo) VALUES ($1, $2, true)")
            .bind(user)
            .bind(name)
            .execute(pool)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO broadcasts (id, owner_id, title, status, started_at, ended_at, live_since) \
         VALUES ($1, $2, 't', $3, now(), CASE WHEN $3 = 'ENDED' THEN now() END, \
                 CASE WHEN $3 = 'LIVE' THEN now() END)",
    )
    .bind(BROADCAST)
    .bind(OWNER)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

fn order(order: i64, donor: i64, amount: i64) -> NewDonation {
    NewDonation {
        id: id(order),
        broadcast_id: id(BROADCAST),
        donor_id: id(donor),
        amount: DonationAmount::parse(amount).unwrap(),
    }
}

async fn stored_totals(pool: &PgPool) -> Option<(i64, i64)> {
    sqlx::query_as("SELECT total, count FROM donation_totals WHERE broadcast_id = $1")
        .bind(BROADCAST)
        .fetch_optional(pool)
        .await
        .unwrap()
}

/// Donation events `subscription` receives until none arrives for a while.
async fn donation_events(subscription: &mut Box<dyn EventSubscription>) -> Vec<DonationSummary> {
    let mut heard = Vec::new();
    while let Ok(delivery) =
        tokio::time::timeout(Duration::from_millis(500), subscription.next()).await
    {
        match delivery.expect("the bus is open") {
            Delivery::Event(event) => match &event.kind {
                EventKind::Donation(credit) => {
                    assert!(event.priority, "a donation is sent at once");
                    heard.push(credit.summary);
                }
                other => panic!("expected a donation, got {other:?}"),
            },
            Delivery::Missed => panic!("expected an event, got Missed"),
        }
    }
    heard
}

/// The issue's acceptance test: the same confirmation sent ten times at
/// once credits the order once, adds it to the total once and announces it
/// once.
#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn ten_concurrent_confirmations_credit_the_order_once(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let (bus, listener) = PgEventBus::start(pool.clone()).await.unwrap();
    let mut events = bus.subscribe(id(BROADCAST));
    let repository = Arc::new(PgDonationRepository::new(pool.clone()));
    assert!(repository.create(&order(100, DONOR, AMOUNT)).await.unwrap());
    let gateway = Arc::new(FakePaymentGateway::default());
    let confirm = Arc::new(ConfirmDonation {
        donations: repository.clone(),
        gateway: gateway.clone(),
    });

    let attempts: Vec<_> = (0..10)
        .map(|_| {
            let confirm = Arc::clone(&confirm);
            tokio::spawn(async move { confirm.run(id(DONOR), "100", "tgen_once", AMOUNT).await })
        })
        .collect();
    let mut credited = Vec::new();
    for attempt in attempts {
        let confirmed = attempt.await.unwrap().unwrap();
        assert_eq!(confirmed.donation.amount.get(), AMOUNT);
        credited.push(confirmed.credited);
    }

    assert_eq!(credited.iter().filter(|c| **c).count(), 1, "{credited:?}");
    assert_eq!(credited.len(), 10);
    assert_eq!(stored_totals(&pool).await, Some((AMOUNT, 1)));
    assert_eq!(
        donation_events(&mut events).await,
        [DonationSummary {
            total: AMOUNT,
            count: 1
        }]
    );
    assert!(
        gateway
            .confirmations()
            .iter()
            .all(|c| c.idempotency_key == "donation-confirm-100"),
        "every attempt asks the provider with the same idempotency key"
    );
    listener.shutdown().await;
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_payment_pays_for_one_order_only(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let repository = Arc::new(PgDonationRepository::new(pool.clone()));
    for n in [100, 101] {
        assert!(repository.create(&order(n, DONOR, AMOUNT)).await.unwrap());
    }
    let confirm = ConfirmDonation {
        donations: repository.clone(),
        gateway: Arc::new(FakePaymentGateway::default()),
    };
    assert!(
        confirm
            .run(id(DONOR), "100", "tgen_shared", AMOUNT)
            .await
            .unwrap()
            .credited
    );

    let reused = confirm.run(id(DONOR), "101", "tgen_shared", AMOUNT).await;
    assert!(
        matches!(reused, Err(Error::Rejected(DonationError::AlreadyPaid))),
        "{reused:?}"
    );
    assert_eq!(
        repository
            .settle(id(101), &key("tgen_shared"), std::time::SystemTime::now())
            .await
            .unwrap(),
        Settlement::PaymentKeyInUse
    );
    assert_eq!(repository.order(id(101)).await.unwrap().unwrap().paid, None);
    assert_eq!(stored_totals(&pool).await, Some((AMOUNT, 1)));
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn an_order_is_stored_only_for_a_viewer_of_a_broadcast_on_air(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let repository = PgDonationRepository::new(pool.clone());
    assert!(
        !repository.create(&order(100, OWNER, AMOUNT)).await.unwrap(),
        "the owner"
    );
    let mut missing = order(101, DONOR, AMOUNT);
    missing.broadcast_id = id(99);
    assert!(!repository.create(&missing).await.unwrap(), "no broadcast");

    for (n, status, open) in [
        (200, "INTERRUPTED", true),
        (201, "ENDED", false),
        (202, "READY", false),
    ] {
        sqlx::query(
            "UPDATE broadcasts SET status = $2, \
                 live_since = CASE WHEN $2 = 'LIVE' THEN now() END, \
                 interrupted_at = CASE WHEN $2 = 'INTERRUPTED' THEN now() END \
             WHERE id = $1",
        )
        .bind(BROADCAST)
        .bind(status)
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            repository.create(&order(n, DONOR, AMOUNT)).await.unwrap(),
            open,
            "{status}"
        );
    }
    sqlx::query(
        "UPDATE broadcasts SET status = 'LIVE', live_since = now(), interrupted_at = NULL, \
             deleted_at = now() WHERE id = $1",
    )
    .bind(BROADCAST)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        !repository.create(&order(300, DONOR, AMOUNT)).await.unwrap(),
        "deleted"
    );

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM donations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 1, "only the order to the interrupted broadcast");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_list_and_summary_hold_credited_donations_only(pool: PgPool) {
    seed(&pool, "LIVE").await;
    let repository = PgDonationRepository::new(pool.clone());
    let query = PgDonationQuery::new(pool.clone());
    assert_eq!(
        query.summary(id(BROADCAST)).await.unwrap(),
        DonationSummary::default()
    );
    for (n, amount) in [(100, 1_000), (101, 2_000), (102, 30_000)] {
        assert!(repository.create(&order(n, DONOR, amount)).await.unwrap());
    }
    let mut last = None;
    for n in [100, 102] {
        let settled = repository
            .settle(
                id(n),
                &key(&format!("tgen_{n}")),
                std::time::SystemTime::now(),
            )
            .await
            .unwrap();
        let Settlement::Credited(credit) = settled else {
            panic!("{settled:?}");
        };
        assert_eq!(credit.donation.donor_name.as_str(), "donor");
        last = Some(credit.summary);
    }
    let expected = DonationSummary {
        total: 31_000,
        count: 2,
    };
    assert_eq!(last, Some(expected), "the credit carries the new total");
    assert_eq!(query.summary(id(BROADCAST)).await.unwrap(), expected);

    let listed: Vec<_> = query
        .list(id(BROADCAST), None, 10)
        .await
        .unwrap()
        .into_iter()
        .map(|view| (view.id.as_i64(), view.amount.get()))
        .collect();
    assert_eq!(
        listed,
        [(102, 30_000), (100, 1_000)],
        "newest first, paid only"
    );
    let older: Vec<_> = query
        .list(id(BROADCAST), Some(id(102)), 10)
        .await
        .unwrap()
        .into_iter()
        .map(|view| view.id.as_i64())
        .collect();
    assert_eq!(older, [100]);
    assert_eq!(query.list(id(BROADCAST), None, 1).await.unwrap().len(), 1);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_table_refuses_what_the_domain_refuses(pool: PgPool) {
    seed(&pool, "LIVE").await;
    for (amount, status, key, approved) in [
        (999, "PENDING", None, false),
        (1_000_001, "PENDING", None, false),
        (1_000, "DONE", None, false),
        (1_000, "DONE", Some("k"), false),
        (1_000, "PENDING", Some("k"), true),
        (1_000, "PAID", None, false),
    ] {
        let result = sqlx::query(
            "INSERT INTO donations (id, broadcast_id, donor_id, amount, status, payment_key, \
                                    approved_at) \
             VALUES (1, $1, $2, $3, $4, $5, CASE WHEN $6 THEN now() END)",
        )
        .bind(BROADCAST)
        .bind(DONOR)
        .bind(amount)
        .bind(status)
        .bind(key)
        .bind(approved)
        .execute(&pool)
        .await;
        assert!(result.is_err(), "{amount} {status} {key:?} {approved}");
    }
}
