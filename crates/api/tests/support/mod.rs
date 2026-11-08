//! The real router on real adapters, for tests that get a fresh Postgres
//! database from `#[sqlx::test]`.
// Each test crate compiles its own copy and uses only part of it.
#![allow(dead_code)]

use std::{
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use api::{
    AppState, app, auth::AuthPorts, broadcast::BroadcastPorts, comment::CommentPorts,
    config::Origin,
};
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
use domain::secret::Secret;
use infrastructure::{
    auth::{Argon2Hasher, BreachedPasswordList},
    id::SnowflakeIdGenerator,
    media::{JwtPublishAuthorizer, MediaMtxIngestRouter, MediaMtxPlaybackResolver, PublishKey},
    persistence::{
        PgBroadcastQuery, PgBroadcastRepository, PgChatPolicyStore, PgCommentQuery,
        PgCommentRepository, PgPublishTicketLedger, PgSessionStore, PgUserRepository,
        PgWorkerIdAllocator,
    },
    rate_limit::InMemoryRateLimiter,
    realtime::PgEventBus,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use tracing_subscriber::fmt::MakeWriter;

pub const ORIGIN: &str = "https://app.test";
pub const PASSWORD: &str = "correct horse battery";
/// What MediaMTX sends on `/internal/*` in these tests.
pub const MEDIA_SECRET: &str = "0123456789abcdef0123456789abcdef";
/// The signing key of every test app, so a test can mint the API's own
/// Control API token.
const PUBLISH_SEED: &str = "nWGxne/9WmC6hEr0kuwsxERJxWl7MmkZcDusAxyuf2A=";

pub fn publish_key() -> PublishKey {
    PublishKey::from_base64_seed(&Secret::new(PUBLISH_SEED.to_owned())).unwrap()
}

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
    let key = Arc::new(publish_key());
    let tokens = Arc::new(JwtPublishAuthorizer::new(key.clone()));
    let broadcasts = Arc::new(PgBroadcastRepository::new(pool.clone()));
    let broadcast_ports = BroadcastPorts {
        repository: broadcasts.clone(),
        query: Arc::new(PgBroadcastQuery::new(pool.clone())),
        ingest: Arc::new(MediaMtxIngestRouter),
        playback: Arc::new(MediaMtxPlaybackResolver),
        authorizer: tokens.clone(),
        terminator,
        verifier: tokens,
        tickets: Arc::new(PgPublishTicketLedger::new(pool.clone())),
        presence: broadcasts,
    };
    // Dropping the handle would stop the listener; the test pool closing at
    // the end of the test stops it instead.
    let (events, listener) = PgEventBus::start(pool.clone()).await.unwrap();
    std::mem::forget(listener);
    let comment_ports = CommentPorts {
        repository: Arc::new(PgCommentRepository::new(pool.clone())),
        query: Arc::new(PgCommentQuery::new(pool.clone())),
        policies: Arc::new(PgChatPolicyStore::new(pool.clone())),
        limiter: Arc::new(InMemoryRateLimiter::default()),
        events,
    };
    let state = AppState::new(
        ports,
        broadcast_ports,
        comment_ports,
        vec![Origin::parse(ORIGIN).unwrap()],
        demo_login_enabled,
        key.public_jwk(),
        Secret::new(MEDIA_SECRET.into()),
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

/// One Server-Sent Event as a client parses it.
#[derive(Debug)]
pub struct SseEvent {
    pub event: String,
    pub id: Option<String>,
    pub data: String,
}

impl SseEvent {
    pub fn json(&self) -> Value {
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

    /// The next event within 5 seconds, or `None` when the stream ends.
    pub async fn next(&mut self) -> Option<SseEvent> {
        tokio::time::timeout(Duration::from_secs(5), self.read())
            .await
            .expect("an event within 5 s")
    }

    async fn read(&mut self) -> Option<SseEvent> {
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

/// Captures JSON log lines, request span fields included, so tests can
/// assert what reached the logs.
#[derive(Clone, Default)]
pub struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl LogBuffer {
    /// Starts capturing on this thread until the guard drops.
    pub fn capture() -> (Self, tracing::subscriber::DefaultGuard) {
        let logs = Self::default();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_current_span(true)
            .with_writer(logs.clone())
            .finish();
        let guard = tracing::subscriber::set_default(subscriber);
        (logs, guard)
    }

    pub fn contents(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }

    /// The audit lines whose `event` is `event`.
    pub fn audits(&self, event: &str) -> Vec<String> {
        let event = format!(r#""event":"{event}""#);
        self.contents()
            .lines()
            .filter(|line| line.contains(r#""target":"audit""#) && line.contains(&event))
            .map(str::to_owned)
            .collect()
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
