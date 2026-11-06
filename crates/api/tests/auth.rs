//! The auth endpoints end to end: the real router, real adapters and a fresh
//! Postgres database per test.

use std::{sync::Arc, time::Duration};

use api::{
    AppState, app,
    auth::{AuthPorts, AuthUser, MaybeUser},
    config::Origin,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
    routing::get,
};
use infrastructure::{
    auth::{Argon2Hasher, BreachedPasswordList},
    id::SnowflakeIdGenerator,
    persistence::{PgSessionStore, PgUserRepository, PgWorkerIdAllocator},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tower::ServiceExt;

const ORIGIN: &str = "https://app.test";
const PASSWORD: &str = "correct horse battery";

async fn router(pool: &PgPool, demo_login_enabled: bool) -> Router {
    app(state(pool, demo_login_enabled).await)
}

async fn state(pool: &PgPool, demo_login_enabled: bool) -> AppState {
    let ttl = Duration::from_secs(30);
    let allocator = Arc::new(PgWorkerIdAllocator::new(pool.clone(), ttl));
    let (ids, heartbeat) = SnowflakeIdGenerator::start(allocator, ttl).await.unwrap();
    // Without renewals ids stay valid for 20 s, far longer than a test.
    drop(heartbeat);
    let ports = AuthPorts {
        ids,
        users: Arc::new(PgUserRepository::new(pool.clone())),
        sessions: Arc::new(PgSessionStore::new(pool.clone())),
        hasher: Arc::new(Argon2Hasher::new().unwrap()),
        breached: Arc::new(BreachedPasswordList::load()),
    };
    let state = AppState::new(
        ports,
        vec![Origin::parse(ORIGIN).unwrap()],
        demo_login_enabled,
    );
    state.auth.demo_log_in.seed().await.unwrap();
    state
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Value,
}

impl Reply {
    fn set_cookie(&self) -> &str {
        self.headers[header::SET_COOKIE].to_str().unwrap()
    }

    /// The token from `Set-Cookie: __Host-session=<token>; ...`.
    fn token(&self) -> String {
        let cookie = self.set_cookie();
        let pair = cookie.split(';').next().unwrap();
        pair.strip_prefix("__Host-session=").unwrap().to_owned()
    }

    fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or_default()
    }
}

fn request(method: Method, path: &str, body: Option<Value>, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method(method.clone()).uri(path);
    if method != Method::GET {
        builder = builder
            .header(header::ORIGIN, ORIGIN)
            .header("x-requested-with", "live-platform");
    }
    if let Some(token) = token {
        builder = builder.header(header::COOKIE, format!("other=1; __Host-session={token}"));
    }
    match body {
        Some(body) => builder
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string())),
        None => builder.body(Body::empty()),
    }
    .unwrap()
}

async fn send(router: &Router, request: Request<Body>) -> Reply {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    Reply {
        status,
        headers,
        body,
    }
}

async fn sign_up(router: &Router, username: &str, password: &str) -> Reply {
    let body = json!({"username": username, "password": password});
    send(
        router,
        request(Method::POST, "/api/auth/signup", Some(body), None),
    )
    .await
}

async fn log_in(router: &Router, username: &str, password: &str, token: Option<&str>) -> Reply {
    let body = json!({"username": username, "password": password});
    send(
        router,
        request(Method::POST, "/api/auth/login", Some(body), token),
    )
    .await
}

async fn me(router: &Router, token: &str) -> Reply {
    send(
        router,
        request(Method::GET, "/api/auth/me", None, Some(token)),
    )
    .await
}

async fn user_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn signup_signs_in_with_a_hardened_session_cookie(pool: PgPool) {
    let router = router(&pool, false).await;

    let signed_up = sign_up(&router, "alice", PASSWORD).await;
    assert_eq!(signed_up.status, StatusCode::CREATED, "{}", signed_up.body);
    let id = signed_up.body["id"].as_str().expect("id is a string");
    assert!(id.parse::<i64>().unwrap() > 0);
    assert_eq!(signed_up.body["username"], "alice");

    let token = signed_up.token();
    assert_eq!(token.len(), 43, "256 bits in unpadded base64url");
    assert_eq!(
        signed_up.set_cookie(),
        format!("__Host-session={token}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=604800")
    );

    let current = me(&router, &token).await;
    assert_eq!(current.status, StatusCode::OK);
    assert_eq!(current.body, json!({"id": id, "username": "alice"}));
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn logout_kills_the_session_and_is_idempotent(pool: PgPool) {
    let router = router(&pool, false).await;
    let token = sign_up(&router, "alice", PASSWORD).await.token();

    let logged_out = send(
        &router,
        request(Method::POST, "/api/auth/logout", None, Some(&token)),
    )
    .await;
    assert_eq!(logged_out.status, StatusCode::NO_CONTENT);
    assert_eq!(
        logged_out.set_cookie(),
        "__Host-session=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0"
    );

    let after = me(&router, &token).await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);
    assert_eq!(after.code(), "UNAUTHENTICATED");

    for token in [Some(token.as_str()), None] {
        let again = send(
            &router,
            request(Method::POST, "/api/auth/logout", None, token),
        )
        .await;
        assert_eq!(again.status, StatusCode::NO_CONTENT);
    }
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn me_without_a_session_is_unauthenticated(pool: PgPool) {
    let router = router(&pool, false).await;
    let anonymous = send(&router, request(Method::GET, "/api/auth/me", None, None)).await;
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);
    assert_eq!(anonymous.code(), "UNAUTHENTICATED");

    let forged = me(&router, "forged-token").await;
    assert_eq!(forged.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn login_issues_a_new_token_and_revokes_the_one_sent(pool: PgPool) {
    let router = router(&pool, false).await;
    let first = sign_up(&router, "alice", PASSWORD).await.token();

    let logged_in = log_in(&router, "alice", PASSWORD, Some(&first)).await;
    assert_eq!(logged_in.status, StatusCode::OK, "{}", logged_in.body);
    assert_eq!(logged_in.body["username"], "alice");
    let second = logged_in.token();

    assert_ne!(first, second);
    assert_eq!(me(&router, &first).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(me(&router, &second).await.status, StatusCode::OK);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn unknown_user_and_wrong_password_are_indistinguishable(pool: PgPool) {
    let router = router(&pool, false).await;
    sign_up(&router, "alice", PASSWORD).await;

    let wrong_password = log_in(&router, "alice", "not the password", None).await;
    let unknown_user = log_in(&router, "nobody", PASSWORD, None).await;

    for reply in [&wrong_password, &unknown_user] {
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.code(), "INVALID_CREDENTIALS");
        assert!(reply.headers.get(header::SET_COOKIE).is_none());
    }
    assert_eq!(
        wrong_password.body["message"], unknown_user.body["message"],
        "same message for both"
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn sessions_expire_after_30_idle_minutes_or_7_days(pool: PgPool) {
    let router = router(&pool, false).await;
    let token = sign_up(&router, "alice", PASSWORD).await.token();
    let shift = |sql: &'static str| {
        let pool = pool.clone();
        async move { sqlx::query(sql).execute(&pool).await.unwrap() }
    };

    shift("UPDATE sessions SET last_seen_at = now() - interval '29 minutes'").await;
    assert_eq!(me(&router, &token).await.status, StatusCode::OK);
    let slid: bool =
        sqlx::query_scalar("SELECT last_seen_at > now() - interval '1 minute' FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(slid, "a request refreshes last_seen_at");

    shift("UPDATE sessions SET last_seen_at = now() - interval '31 minutes'").await;
    assert_eq!(me(&router, &token).await.status, StatusCode::UNAUTHORIZED);

    let token = log_in(&router, "alice", PASSWORD, None).await.token();
    assert_eq!(me(&router, &token).await.status, StatusCode::OK);
    shift("UPDATE sessions SET expires_at = now() - interval '1 second'").await;
    assert_eq!(
        me(&router, &token).await.status,
        StatusCode::UNAUTHORIZED,
        "absolute expiry applies even to an active session"
    );
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn last_seen_at_is_rewritten_at_most_once_a_minute(pool: PgPool) {
    let router = router(&pool, false).await;
    let token = sign_up(&router, "alice", PASSWORD).await.token();
    let last_seen = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(sql)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };

    let before = last_seen(
        "UPDATE sessions SET last_seen_at = now() - interval '30 seconds' \
         RETURNING last_seen_at::text",
    )
    .await;
    assert_eq!(me(&router, &token).await.status, StatusCode::OK);
    let after = last_seen("SELECT last_seen_at::text FROM sessions").await;
    assert_eq!(before, after, "a recent last_seen_at is left alone");

    last_seen(
        "UPDATE sessions SET last_seen_at = now() - interval '61 seconds' \
         RETURNING last_seen_at::text",
    )
    .await;
    assert_eq!(me(&router, &token).await.status, StatusCode::OK);
    let slid: bool =
        sqlx::query_scalar("SELECT last_seen_at > now() - interval '5 seconds' FROM sessions")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(slid, "a stale last_seen_at is refreshed");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_database_holds_only_the_token_sha256(pool: PgPool) {
    let router = router(&pool, false).await;
    let token = sign_up(&router, "alice", PASSWORD).await.token();

    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(stored, [Sha256::digest(token.as_bytes()).to_vec()]);

    let raw_anywhere: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM sessions s WHERE position($1::bytea IN s.token_hash) > 0 \
         OR strpos(s::text, $2) > 0)",
    )
    .bind(token.as_bytes())
    .bind(&token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!raw_anywhere, "the raw token is not stored");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn the_password_is_stored_as_an_argon2id_hash(pool: PgPool) {
    let router = router(&pool, false).await;
    sign_up(&router, "alice", PASSWORD).await;

    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "{hash}"
    );
    assert!(!hash.contains(PASSWORD));
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn signup_rejects_taken_reserved_and_weak_input(pool: PgPool) {
    let router = router(&pool, false).await;
    assert_eq!(
        sign_up(&router, "alice", PASSWORD).await.status,
        StatusCode::CREATED
    );

    for (i, (username, password, status, code)) in [
        ("alice", PASSWORD, StatusCode::CONFLICT, "USERNAME_TAKEN"),
        (
            "demo-streamer",
            PASSWORD,
            StatusCode::BAD_REQUEST,
            "INVALID_USERNAME",
        ),
        ("Bob", PASSWORD, StatusCode::BAD_REQUEST, "INVALID_USERNAME"),
        (
            "bob",
            "short",
            StatusCode::BAD_REQUEST,
            "PASSWORD_TOO_SHORT",
        ),
        (
            "bob",
            "QWERTYUIOP",
            StatusCode::BAD_REQUEST,
            "PASSWORD_BREACHED",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let body = json!({"username": username, "password": password});
        let mut signup = request(Method::POST, "/api/auth/signup", Some(body), None);
        // Each attempt from its own client, so the signup rate limit stays out of the way.
        signup
            .headers_mut()
            .insert("x-forwarded-for", format!("192.0.2.{i}").parse().unwrap());
        let reply = send(&router, signup).await;
        assert_eq!(
            reply.status, status,
            "{username}/{password}: {}",
            reply.body
        );
        assert_eq!(reply.code(), code, "{username}/{password}");
        assert!(reply.headers.get(header::SET_COOKIE).is_none());
    }
    assert_eq!(user_count(&pool).await, 1);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn state_changes_without_origin_and_marker_header_are_rejected(pool: PgPool) {
    let router = router(&pool, false).await;
    let body = json!({"username": "alice", "password": PASSWORD}).to_string();
    let signup = || {
        Request::post("/api/auth/signup")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.clone()))
            .unwrap()
    };

    let mut missing_origin = signup();
    missing_origin
        .headers_mut()
        .insert("x-requested-with", "live-platform".parse().unwrap());
    let mut wrong_origin = signup();
    wrong_origin
        .headers_mut()
        .insert(header::ORIGIN, "https://evil.test".parse().unwrap());
    wrong_origin
        .headers_mut()
        .insert("x-requested-with", "live-platform".parse().unwrap());
    let mut missing_marker = signup();
    missing_marker
        .headers_mut()
        .insert(header::ORIGIN, ORIGIN.parse().unwrap());

    for request in [missing_origin, wrong_origin, missing_marker] {
        let reply = send(&router, request).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(reply.code(), "CSRF_REJECTED");
    }
    assert_eq!(user_count(&pool).await, 0, "the handler never ran");

    let logout = Request::post("/api/auth/logout")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&router, logout).await.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn repeated_logins_for_one_username_are_rate_limited(pool: PgPool) {
    let router = router(&pool, false).await;
    sign_up(&router, "alice", PASSWORD).await;

    for _ in 0..5 {
        let reply = log_in(&router, "alice", "guessing passwords", None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
    }
    let limited = log_in(&router, "alice", PASSWORD, None).await;
    assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(limited.code(), "RATE_LIMITED");
    let retry_after: u64 = limited.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=60).contains(&retry_after), "{retry_after}");
    assert!(limited.headers.get(header::SET_COOKIE).is_none());
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn demo_login_is_hidden_while_disabled(pool: PgPool) {
    let router = router(&pool, false).await;

    let status = send(&router, request(Method::GET, "/api/auth/demo", None, None)).await;
    assert_eq!(status.body, json!({"enabled": false}));

    let reply = send(
        &router,
        request(
            Method::POST,
            "/api/auth/demo-login",
            Some(json!({"account": "streamer"})),
            None,
        ),
    )
    .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.code(), "NOT_FOUND");
    assert_eq!(user_count(&pool).await, 0, "no demo account is seeded");
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn demo_login_signs_into_seeded_accounts_that_have_no_password(pool: PgPool) {
    let router = router(&pool, true).await;
    // A second seed, as on every restart, leaves the accounts as they are.
    router_seed_is_idempotent(&pool).await;

    let status = send(&router, request(Method::GET, "/api/auth/demo", None, None)).await;
    assert_eq!(status.body, json!({"enabled": true}));

    for account in ["streamer", "viewer"] {
        let reply = send(
            &router,
            request(
                Method::POST,
                "/api/auth/demo-login",
                Some(json!({"account": account})),
                None,
            ),
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.body);
        assert_eq!(reply.body["username"], format!("demo-{account}"));
        assert_eq!(me(&router, &reply.token()).await.status, StatusCode::OK);
    }

    let unknown_user = log_in(&router, "nobody", PASSWORD, None).await;
    for password in [PASSWORD, ""] {
        let reply = log_in(&router, "demo-streamer", password, None).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED);
        assert_eq!(reply.body["code"], unknown_user.body["code"]);
        assert_eq!(reply.body["message"], unknown_user.body["message"]);
    }

    let bad_account = send(
        &router,
        request(
            Method::POST,
            "/api/auth/demo-login",
            Some(json!({"account": "admin"})),
            None,
        ),
    )
    .await;
    assert_eq!(bad_account.status, StatusCode::BAD_REQUEST);
    assert_eq!(bad_account.code(), "INVALID_REQUEST");
}

async fn router_seed_is_idempotent(pool: &PgPool) {
    let _ = router(pool, true).await;
    let demo: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM users WHERE is_demo AND password_hash IS NULL \
         AND username IN ('demo-streamer', 'demo-viewer')",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(demo, 2);
    assert_eq!(user_count(pool).await, 2);
}

#[sqlx::test(migrations = "../infrastructure/migrations")]
#[ignore = "requires DATABASE_URL (Postgres)"]
async fn user_extractors_follow_the_session(pool: PgPool) {
    let state = state(&pool, false).await;
    let signed_up = sign_up(&app(state.clone()), "alice", PASSWORD).await;
    let token = signed_up.token();
    let id = signed_up.body["id"].as_str().unwrap().to_owned();
    let probe = Router::new()
        .route(
            "/required",
            get(|AuthUser(id): AuthUser| async move { id.to_string() }),
        )
        .route(
            "/optional",
            get(|MaybeUser(id): MaybeUser| async move { format!("{id:?}") }),
        )
        .with_state(state);
    let call = |path: &'static str, token: Option<&str>| {
        let probe = probe.clone();
        let request = request(Method::GET, path, None, token);
        async move {
            let response = probe.oneshot(request).await.unwrap();
            let status = response.status();
            let body = to_bytes(response.into_body(), 1024).await.unwrap();
            (status, String::from_utf8(body.to_vec()).unwrap())
        }
    };

    assert_eq!(
        call("/required", Some(&token)).await,
        (StatusCode::OK, id.clone())
    );
    assert_eq!(
        call("/optional", Some(&token)).await,
        (StatusCode::OK, format!("Some(SnowflakeId({id}))"))
    );
    for token in [None, Some("forged")] {
        assert_eq!(call("/required", token).await.0, StatusCode::UNAUTHORIZED);
        assert_eq!(
            call("/optional", token).await,
            (StatusCode::OK, "None".into())
        );
    }
}
