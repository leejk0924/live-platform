//! The donation endpoints end to end: the real router, real adapters, a
//! fresh Postgres database per test, and a fake Toss.

mod support;

use std::sync::Arc;

use api::app;
use application::ports::payment::{GatewayError, GatewayPayment, PaymentStatus};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use infrastructure::payment::FakePaymentGateway;
use serde_json::{Value, json};
use sqlx::PgPool;
use support::{
    PASSWORD, Reply, SseReader, TOSS_CLIENT_KEY, donation_state, request, router, send, sign_up,
    sign_up_streamer,
};
use tower::ServiceExt;

struct Shop {
    router: Router,
    gateway: Arc<FakePaymentGateway>,
    owner: String,
    viewer: String,
    other: String,
    broadcast: String,
}

/// A live broadcast by `owner`, and two viewers.
async fn shop(pool: &PgPool) -> Shop {
    let gateway = Arc::new(FakePaymentGateway::default());
    let router = app(donation_state(pool, gateway.clone()).await);
    let owner = sign_up_streamer(pool, &router, "owner", PASSWORD)
        .await
        .token();
    let viewer = sign_up(&router, "viewer", PASSWORD).await.token();
    let other = sign_up(&router, "other", PASSWORD).await.token();
    let created = send(
        &router,
        request(
            Method::POST,
            "/api/broadcasts",
            Some(json!({"title": "방송", "description": ""})),
            Some(&owner),
        ),
    )
    .await;
    let broadcast = created.body["id"].as_str().unwrap().to_owned();
    let started = send(
        &router,
        request(
            Method::POST,
            &format!("/api/broadcasts/{broadcast}/start"),
            None,
            Some(&owner),
        ),
    )
    .await;
    assert_eq!(started.status, StatusCode::OK, "{}", started.body);
    Shop {
        router,
        gateway,
        owner,
        viewer,
        other,
        broadcast,
    }
}

impl Shop {
    async fn order(&self, token: Option<&str>, amount: Value) -> Reply {
        send(
            &self.router,
            request(
                Method::POST,
                &format!("/api/broadcasts/{}/donations", self.broadcast),
                Some(json!({"amount": amount})),
                token,
            ),
        )
        .await
    }

    async fn ordered(&self, amount: i64) -> String {
        let reply = self.order(Some(&self.viewer), json!(amount)).await;
        assert_eq!(reply.status, StatusCode::CREATED, "{}", reply.body);
        reply.body["orderId"].as_str().unwrap().to_owned()
    }

    async fn confirm(&self, token: &str, order: &str, key: &str, amount: i64) -> Reply {
        send(
            &self.router,
            request(
                Method::POST,
                "/api/donations/confirm",
                Some(json!({"paymentKey": key, "orderId": order, "amount": amount})),
                Some(token),
            ),
        )
        .await
    }

    async fn list(&self, query: &str) -> Reply {
        send(
            &self.router,
            request(
                Method::GET,
                &format!("/api/broadcasts/{}/donations{query}", self.broadcast),
                None,
                None,
            ),
        )
        .await
    }

    /// The broadcast's event stream past the policy and totals it opens
    /// with.
    async fn follow(&self) -> SseReader {
        let request = request(
            Method::GET,
            &format!("/api/broadcasts/{}/events", self.broadcast),
            None,
            None,
        );
        let response = self.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let mut events = SseReader::new(response);
        assert_eq!(events.next().await.unwrap().event, "policy");
        let totals = events.next().await.unwrap();
        assert_eq!(totals.event, "donations");
        events
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_viewer_orders_pays_once_and_everyone_sees_it(pool: PgPool) {
    let shop = shop(&pool).await;
    let mut events = shop.follow().await;

    let order = shop.order(Some(&shop.viewer), json!(5000)).await;
    assert_eq!(order.status, StatusCode::CREATED, "{}", order.body);
    let order_id = order.body["orderId"].as_str().unwrap().to_owned();
    assert_eq!(
        order.body,
        json!({
            "orderId": order_id,
            "amount": 5000,
            "orderName": "live-platform 후원",
            "clientKey": TOSS_CLIENT_KEY,
        })
    );
    assert!(
        shop.list("").await.body["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let paid = shop
        .confirm(&shop.viewer, &order_id, "tgen_paid", 5000)
        .await;
    assert_eq!(paid.status, StatusCode::OK, "{}", paid.body);
    assert_eq!(paid.body["credited"], true);
    let donation = paid.body["donation"].clone();
    assert_eq!(
        (
            &donation["id"],
            &donation["broadcastId"],
            &donation["donorName"],
            &donation["amount"]
        ),
        (
            &json!(order_id),
            &json!(shop.broadcast),
            &json!("viewer"),
            &json!(5000)
        )
    );

    let live = events.next().await.unwrap();
    assert_eq!(live.event, "donation");
    assert_eq!(
        live.json(),
        json!({"donation": donation, "total": 5000, "count": 1})
    );

    let again = shop
        .confirm(&shop.viewer, &order_id, "tgen_paid", 5000)
        .await;
    assert_eq!(again.status, StatusCode::OK);
    assert_eq!(again.body, json!({"donation": donation, "credited": false}));
    assert_eq!(shop.gateway.confirmations().len(), 1);

    let second = shop.order(Some(&shop.other), json!(20_000)).await;
    let second = second.body["orderId"].as_str().unwrap().to_owned();
    shop.confirm(&shop.other, &second, "tgen_second", 20_000)
        .await;
    let listed = shop.list("?size=1").await;
    assert_eq!(listed.status, StatusCode::OK);
    assert_eq!(
        (&listed.body["total"], &listed.body["count"]),
        (&json!(25_000), &json!(2))
    );
    assert_eq!(listed.body["items"][0]["id"], json!(second));
    assert_eq!(listed.body["nextBefore"], json!(second));
    let rest = shop.list(&format!("?before={second}")).await;
    assert_eq!(rest.body["items"], json!([donation]));
    assert_eq!(rest.body["nextBefore"], Value::Null);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn orders_are_refused_to_the_owner_off_air_and_out_of_range(pool: PgPool) {
    let shop = shop(&pool).await;
    for (token, amount, status, code) in [
        (
            None,
            json!(5000),
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
        ),
        (
            Some(&shop.owner),
            json!(5000),
            StatusCode::FORBIDDEN,
            "SELF_DONATION",
        ),
        (
            Some(&shop.viewer),
            json!(999),
            StatusCode::BAD_REQUEST,
            "INVALID_AMOUNT",
        ),
        (
            Some(&shop.viewer),
            json!(1_000_001),
            StatusCode::BAD_REQUEST,
            "INVALID_AMOUNT",
        ),
        (
            Some(&shop.viewer),
            json!(1000.5),
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
        ),
        (
            Some(&shop.viewer),
            json!("1000"),
            StatusCode::BAD_REQUEST,
            "INVALID_REQUEST",
        ),
    ] {
        let reply = shop.order(token.map(String::as_str), amount.clone()).await;
        assert_eq!((reply.status, reply.code()), (status, code), "{amount}");
    }
    let unknown = send(
        &shop.router,
        request(
            Method::POST,
            "/api/broadcasts/123/donations",
            Some(json!({"amount": 5000})),
            Some(&shop.viewer),
        ),
    )
    .await;
    assert_eq!(unknown.code(), "BROADCAST_NOT_FOUND");

    let stopped = send(
        &shop.router,
        request(
            Method::POST,
            &format!("/api/broadcasts/{}/stop", shop.broadcast),
            None,
            Some(&shop.owner),
        ),
    )
    .await;
    assert_eq!(stopped.status, StatusCode::OK, "{}", stopped.body);
    let ended = shop.order(Some(&shop.viewer), json!(5000)).await;
    assert_eq!(
        (ended.status, ended.code()),
        (StatusCode::CONFLICT, "DONATIONS_CLOSED")
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM donations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_donor_orders_again_only_after_a_pause(pool: PgPool) {
    let shop = shop(&pool).await;
    shop.ordered(5000).await;
    let again = shop.order(Some(&shop.viewer), json!(5000)).await;
    assert_eq!(
        (again.status, again.code()),
        (StatusCode::TOO_MANY_REQUESTS, "RATE_LIMITED")
    );
    assert!(again.headers.contains_key(header::RETRY_AFTER));
    let other = shop.order(Some(&shop.other), json!(5000)).await;
    assert_eq!(
        other.status,
        StatusCode::CREATED,
        "other donors are not held up"
    );
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM donations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 2);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn a_confirmation_must_match_the_order_its_donor_and_tosss_answer(pool: PgPool) {
    let shop = shop(&pool).await;
    let order = shop.ordered(5000).await;

    let not_donor = shop.confirm(&shop.other, &order, "tgen_x", 5000).await;
    assert_eq!(
        (not_donor.status, not_donor.code()),
        (StatusCode::FORBIDDEN, "NOT_DONOR")
    );
    let cheaper = shop.confirm(&shop.viewer, &order, "tgen_x", 100).await;
    assert_eq!(
        (cheaper.status, cheaper.code()),
        (StatusCode::BAD_REQUEST, "AMOUNT_MISMATCH")
    );
    let unknown = shop.confirm(&shop.viewer, "123", "tgen_x", 5000).await;
    assert_eq!(
        (unknown.status, unknown.code()),
        (StatusCode::NOT_FOUND, "ORDER_NOT_FOUND")
    );
    assert!(
        shop.gateway.confirmations().is_empty(),
        "Toss was never asked"
    );

    shop.gateway.answer_confirm(Err(GatewayError::Rejected {
        code: "REJECT_CARD_PAYMENT".into(),
    }));
    let refused = shop.confirm(&shop.viewer, &order, "tgen_x", 5000).await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_REQUEST, "PAYMENT_REJECTED")
    );
    assert!(
        !refused.body.to_string().contains("REJECT_CARD_PAYMENT"),
        "{}",
        refused.body
    );
    shop.gateway
        .answer_confirm(Err(GatewayError::Unavailable("timeout".into())));
    let silent = shop.confirm(&shop.viewer, &order, "tgen_x", 5000).await;
    assert_eq!(
        (silent.status, silent.code()),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "PAYMENT_GATEWAY_UNAVAILABLE"
        )
    );
    assert_eq!(shop.list("").await.body["count"], 0);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_webhook_credits_what_toss_reports_paid(pool: PgPool) {
    let shop = shop(&pool).await;
    let order = shop.ordered(5000).await;
    shop.gateway.answer_lookup(Ok(Some(GatewayPayment {
        payment_key: "tgen_hook".into(),
        order_id: order.clone(),
        status: PaymentStatus::Done,
        total_amount: 5000,
        approved_at: None,
    })));
    let notification = json!({
        "eventType": "PAYMENT_STATUS_CHANGED",
        "data": {"paymentKey": "tgen_hook", "orderId": order, "status": "DONE",
                 "totalAmount": 1}
    });
    for _ in 0..2 {
        let request = Request::post("/api/payments/toss/webhook")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(notification.to_string()))
            .unwrap();
        assert_eq!(send(&shop.router, request).await.status, StatusCode::OK);
    }

    let listed = shop.list("").await.body;
    assert_eq!(
        (&listed["total"], &listed["count"]),
        (&json!(5000), &json!(1))
    );
    let confirmed = shop.confirm(&shop.viewer, &order, "tgen_hook", 5000).await;
    assert_eq!(confirmed.body["credited"], false, "{}", confirmed.body);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn without_toss_keys_ordering_and_paying_are_unavailable(pool: PgPool) {
    let router = router(&pool, false).await;
    let viewer = sign_up(&router, "viewer", PASSWORD).await.token();
    for (path, body) in [
        ("/api/broadcasts/1/donations", json!({"amount": 5000})),
        (
            "/api/donations/confirm",
            json!({"paymentKey": "tgen_1", "orderId": "1", "amount": 5000}),
        ),
    ] {
        let reply = send(
            &router,
            request(Method::POST, path, Some(body), Some(&viewer)),
        )
        .await;
        assert_eq!(
            (reply.status, reply.code()),
            (StatusCode::SERVICE_UNAVAILABLE, "PAYMENTS_DISABLED"),
            "{path}"
        );
    }
}
