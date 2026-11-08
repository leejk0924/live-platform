//! The real router on real adapters, for tests that get a fresh Postgres
//! database from `#[sqlx::test]`.
// Each test crate compiles its own copy and uses only part of it.
#![allow(dead_code)]

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use api::{AppState, app, auth::AuthPorts, broadcast::BroadcastPorts, config::Origin};
use application::ports::{
    PortError,
    media::{StreamPath, StreamTerminator},
};
use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use infrastructure::{
    auth::{Argon2Hasher, BreachedPasswordList},
    id::SnowflakeIdGenerator,
    media::{JwtPublishAuthorizer, MediaMtxIngestRouter, PublishKey},
    persistence::{
        PgBroadcastQuery, PgBroadcastRepository, PgSessionStore, PgUserRepository,
        PgWorkerIdAllocator,
    },
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

pub const ORIGIN: &str = "https://app.test";
pub const PASSWORD: &str = "correct horse battery";

pub async fn router(pool: &PgPool, demo_login_enabled: bool) -> Router {
    app(state(pool, demo_login_enabled).await)
}

/// Stands in for MediaMTX's Control API and counts the kicks that finished.
#[derive(Default)]
pub struct CountingTerminator {
    pub kicks: AtomicUsize,
    /// How long each kick takes.
    pub delay: Duration,
    /// Every kick fails, as when MediaMTX is unreachable.
    pub fail: bool,
}

pub const KICK_FAILURE: &str = "mediamtx api: connection refused";

#[async_trait]
impl StreamTerminator for CountingTerminator {
    async fn kick(&self, _: &StreamPath) -> Result<(), PortError> {
        tokio::time::sleep(self.delay).await;
        self.kicks.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(PortError(KICK_FAILURE.into()));
        }
        Ok(())
    }
}

/// A brand-new app instance: its own id lease, rate limits and caches, with
/// only the database shared.
pub async fn state(pool: &PgPool, demo_login_enabled: bool) -> AppState {
    live_state(pool, demo_login_enabled).await.0
}

/// [`state`] with real publish tokens and a counting stand-in for kicks.
pub async fn live_state(
    pool: &PgPool,
    demo_login_enabled: bool,
) -> (AppState, Arc<CountingTerminator>) {
    let terminator = Arc::new(CountingTerminator::default());
    let state = live_state_with(pool, demo_login_enabled, terminator.clone()).await;
    (state, terminator)
}

pub async fn live_state_with(
    pool: &PgPool,
    demo_login_enabled: bool,
    terminator: Arc<CountingTerminator>,
) -> AppState {
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
    let key = Arc::new(PublishKey::generate());
    let broadcast_ports = BroadcastPorts {
        repository: Arc::new(PgBroadcastRepository::new(pool.clone())),
        query: Arc::new(PgBroadcastQuery::new(pool.clone())),
        ingest: Arc::new(MediaMtxIngestRouter),
        authorizer: Arc::new(JwtPublishAuthorizer::new(key.clone())),
        terminator,
    };
    let state = AppState::new(
        ports,
        broadcast_ports,
        vec![Origin::parse(ORIGIN).unwrap()],
        demo_login_enabled,
        key.public_jwk(),
    );
    state.auth.demo_log_in.seed().await.unwrap();
    state
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

impl Reply {
    pub fn set_cookie(&self) -> &str {
        self.headers[header::SET_COOKIE].to_str().unwrap()
    }

    /// The token from `Set-Cookie: __Host-session=<token>; ...`.
    pub fn token(&self) -> String {
        let cookie = self.set_cookie();
        let pair = cookie.split(';').next().unwrap();
        pair.strip_prefix("__Host-session=").unwrap().to_owned()
    }

    pub fn code(&self) -> &str {
        self.body["code"].as_str().unwrap_or_default()
    }
}

/// State-changing requests pass the CSRF guard, so tests see what the
/// handler decides.
pub fn request(
    method: Method,
    path: &str,
    body: Option<Value>,
    token: Option<&str>,
) -> Request<Body> {
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

pub async fn send(router: &Router, request: Request<Body>) -> Reply {
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

pub async fn sign_up(router: &Router, username: &str, password: &str) -> Reply {
    let body = json!({"username": username, "password": password});
    send(
        router,
        request(Method::POST, "/api/auth/signup", Some(body), None),
    )
    .await
}
