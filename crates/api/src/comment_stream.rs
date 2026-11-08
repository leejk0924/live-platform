//! `GET /api/broadcasts/{id}/events`: a broadcast's live comments as
//! Server-Sent Events. Comment changes are batched for up to
//! [`BATCH_WINDOW`] into one `comments` event whose id is the newest
//! revision in it, so a reconnecting `EventSource` resumes from there with
//! `Last-Event-ID`.

use std::{
    collections::{HashMap, VecDeque},
    convert::Infallible,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use application::{
    comment::{CommentFeed, FeedItem},
    ports::comment::CommentView,
};
use axum::{
    extract::{Path, Query, State, rejection::QueryRejection},
    http::HeaderMap,
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use domain::comment::{ChatPolicy, Revision};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use tokio::{
    sync::watch,
    time::{Instant, sleep_until},
};
use utoipa::IntoParams;

use crate::{
    AppState,
    broadcast::broadcast_id,
    comment::{ChatPolicyDto, CommentDto, api_error},
    error::{ApiError, ErrorBody},
    rate_limit::ClientKey,
};

/// How long the first pending comment change waits for more to share its
/// event. The broadcaster's comments, policy changes and resets do not wait.
pub const BATCH_WINDOW: Duration = Duration::from_millis(200);

/// Open event streams per client (see [`ClientKey`]); each holds a
/// connection and a file descriptor.
pub const MAX_STREAMS_PER_CLIENT: usize = 20;

pub const LAST_EVENT_ID: &str = "last-event-id";

/// The event streams this process holds open: how many per client, and the
/// signal that ends them all at shutdown, since a graceful shutdown waits
/// for every open connection and a stream never ends on its own.
pub struct LiveStreams {
    open: Mutex<HashMap<String, usize>>,
    closing: watch::Sender<bool>,
}

impl Default for LiveStreams {
    fn default() -> Self {
        Self {
            open: Mutex::default(),
            closing: watch::Sender::new(false),
        }
    }
}

impl LiveStreams {
    /// Ends every open stream, and every stream opened from now on as soon
    /// as it starts.
    pub fn close_all(&self) {
        self.closing.send_replace(true);
    }

    /// A count is updated in one step, so a panic while the lock was held
    /// left it consistent.
    fn counts(&self) -> MutexGuard<'_, HashMap<String, usize>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn admit(self: &Arc<Self>, client: &ClientKey) -> Result<StreamSlot, ApiError> {
        let mut open = self.counts();
        let count = open.entry(client.as_str().to_owned()).or_default();
        if *count >= MAX_STREAMS_PER_CLIENT {
            return Err(ApiError::too_many_streams());
        }
        *count += 1;
        Ok(StreamSlot {
            streams: Arc::clone(self),
            client: client.as_str().to_owned(),
        })
    }

    #[cfg(test)]
    fn open_for(&self, client: &str) -> usize {
        self.counts().get(client).copied().unwrap_or_default()
    }
}

/// One admitted stream; its client's count drops when the stream does,
/// which is when the client disconnects.
struct StreamSlot {
    streams: Arc<LiveStreams>,
    client: String,
}

impl Drop for StreamSlot {
    fn drop(&mut self) {
        let mut open = self.streams.counts();
        if let Some(count) = open.get_mut(&self.client) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.client);
            }
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EventParams {
    /// Receive every change after this revision first: the `revision` of
    /// the comment list. `Last-Event-ID` takes precedence.
    after: Option<String>,
}

#[derive(Serialize)]
struct ResetData {
    revision: String,
}

#[utoipa::path(
    get,
    path = "/api/broadcasts/{id}/events",
    operation_id = "followBroadcastEvents",
    tag = "comments",
    summary = "Follow a broadcast's comments live (Server-Sent Events)",
    description = "A `text/event-stream` with these events:\n\n\
        - `comments`: `data` is a JSON array of `Comment`, every change (post, edit, delete) \
          within about 200 ms, oldest first; `id` is the newest `revision` in it. Upsert by \
          `id`, keeping the higher `revision`; a deleted comment arrives with `body: null`.\n\
        - `policy`: `data` is a `ChatPolicy`; sent when the stream opens, after catching up \
          what it missed, and on every change.\n\
        - `reset`: too much was missed to replay; reload the list. `data` is \
          `{\"revision\": string}`, also the event's `id`.\n\n\
        Not subject to the 10 second request limit. A reconnecting `EventSource` sends \
        `Last-Event-ID` and receives what it missed.",
    params(
        ("id" = String, Path, description = "Broadcast id"),
        ("Last-Event-ID" = Option<String>, Header, description = "The last `id` received; sent by `EventSource` on reconnect. `0` replays from the first comment"),
        EventParams,
    ),
    responses(
        (status = 200, content_type = "text/event-stream", body = String, description = "The event stream."),
        (status = 400, body = ErrorBody, description = "`INVALID_QUERY`: `after` or `Last-Event-ID` is not a revision"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
        (status = 429, body = ErrorBody, description = "`TOO_MANY_STREAMS`: 20 streams are already open from this client"),
    ),
)]
pub(crate) async fn follow(
    State(state): State<AppState>,
    Path(id): Path<String>,
    client: ClientKey,
    headers: HeaderMap,
    params: Result<Query<EventParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let broadcast = broadcast_id(&id)?;
    let Query(params) = params.map_err(|_| ApiError::invalid_query())?;
    let after = match headers.get(LAST_EVENT_ID) {
        Some(value) => Some(value.to_str().ok().unwrap_or_default().to_owned()),
        None => params.after,
    };
    let after = after
        .map(|raw| Revision::parse(raw.trim()).ok_or_else(ApiError::invalid_query))
        .transpose()?;
    let slot = state.streams.admit(&client)?;
    let feed = state
        .comments
        .follow
        .open(broadcast, after)
        .await
        .map_err(api_error)?;
    let follower = Follower {
        feed,
        pending: Vec::new(),
        flush_at: None,
        ready: VecDeque::new(),
        closing: state.streams.closing.subscribe(),
        done: false,
        _slot: slot,
    };
    let events = stream::unfold(follower, |mut follower| async move {
        let event = follower.next_event().await?;
        Some((Ok::<_, Infallible>(event), follower))
    });
    Ok(Sse::new(events)
        .keep_alive(KeepAlive::default())
        .into_response())
}

/// What woke a follower up.
enum Wake {
    Closing,
    Flush,
    Feed(Result<Option<FeedItem>, application::ports::PortError>),
}

struct Follower {
    feed: CommentFeed,
    /// Comment changes waiting for their batch to fill.
    pending: Vec<CommentView>,
    flush_at: Option<Instant>,
    ready: VecDeque<Event>,
    closing: watch::Receiver<bool>,
    done: bool,
    _slot: StreamSlot,
}

impl Follower {
    async fn next_event(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = self.ready.pop_front() {
                return Some(event);
            }
            if self.done {
                return None;
            }
            let flush_at = self.flush_at;
            let wake = tokio::select! {
                biased;
                _ = self.closing.wait_for(|closing| *closing) => Wake::Closing,
                () = sleep_until(flush_at.unwrap_or_else(Instant::now)), if flush_at.is_some() => {
                    Wake::Flush
                }
                item = self.feed.next() => Wake::Feed(item),
            };
            match wake {
                Wake::Closing => return None,
                Wake::Flush => self.flush(),
                Wake::Feed(item) => self.take(item),
            }
        }
    }

    fn take(&mut self, item: Result<Option<FeedItem>, application::ports::PortError>) {
        match item {
            Ok(Some(FeedItem::Comments { changes, priority })) => {
                self.pending.extend(changes);
                if priority {
                    self.flush();
                } else if self.flush_at.is_none() {
                    self.flush_at = Some(Instant::now() + BATCH_WINDOW);
                }
            }
            Ok(Some(FeedItem::Policy(policy))) => {
                self.flush();
                self.ready.push_back(policy_event(policy));
            }
            Ok(Some(FeedItem::Reset { cursor })) => {
                self.flush();
                self.ready.push_back(reset_event(cursor));
            }
            Ok(None) => {
                self.flush();
                self.done = true;
            }
            Err(err) => {
                // The client reconnects with Last-Event-ID and catches up.
                tracing::warn!(%err, "comment stream ended by a store failure");
                self.flush();
                self.done = true;
            }
        }
    }

    fn flush(&mut self) {
        self.flush_at = None;
        if self.pending.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.pending);
        let newest = changes
            .iter()
            .map(|view| view.revision)
            .max()
            .expect("not empty");
        let data: Vec<CommentDto> = changes.into_iter().map(CommentDto::from).collect();
        let event = Event::default()
            .event("comments")
            .id(newest.get().to_string())
            .json_data(data)
            .expect("comments serialize");
        self.ready.push_back(event);
    }
}

fn policy_event(policy: ChatPolicy) -> Event {
    Event::default()
        .event("policy")
        .json_data(ChatPolicyDto::from(policy))
        .expect("a policy serializes")
}

fn reset_event(cursor: Revision) -> Event {
    let revision = cursor.get().to_string();
    Event::default()
        .event("reset")
        .id(&revision)
        .json_data(ResetData { revision })
        .expect("a reset serializes")
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use application::ports::{
        comment::CommentView,
        event_bus::{EventBus, EventKind, RealtimeEvent},
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use domain::{
        auth::Username,
        broadcast::Version,
        comment::{Comment, CommentBody},
        id::SnowflakeId,
    };
    use futures_util::StreamExt;
    use infrastructure::realtime::LocalEventBus;
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        REQUEST_TIMEOUT, app,
        test_support::{SseReader, chat_state},
    };

    const BROADCAST: i64 = 1;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    fn comment(revision: i64, priority: bool) -> RealtimeEvent {
        RealtimeEvent {
            broadcast: id(BROADCAST),
            priority,
            kind: EventKind::Comment(CommentView {
                comment: Comment {
                    id: id(revision),
                    broadcast_id: id(BROADCAST),
                    author_id: id(5),
                    body: Some(CommentBody::parse(&format!("c{revision}")).unwrap()),
                    version: Version::FIRST,
                    created_at: SystemTime::UNIX_EPOCH,
                    edited: false,
                },
                author_name: Username::parse("viewer").unwrap(),
                revision: Revision::new(revision).unwrap(),
            }),
        }
    }

    struct Client {
        bus: Arc<LocalEventBus>,
        state: AppState,
    }

    impl Client {
        fn new() -> Self {
            let bus = Arc::new(LocalEventBus::default());
            let state = chat_state(bus.clone());
            Self { bus, state }
        }

        async fn open(&self, path: &str, last_event_id: Option<&str>) -> Response {
            let mut request = Request::get(path).header("x-forwarded-for", "203.0.113.7");
            if let Some(last) = last_event_id {
                request = request.header(LAST_EVENT_ID, last);
            }
            app(self.state.clone())
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap()
        }

        async fn stream(&self) -> SseReader {
            let response = self
                .open(&format!("/api/broadcasts/{BROADCAST}/events"), None)
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["content-type"], "text/event-stream");
            SseReader::new(response)
        }

        /// A stream past the policy it opens with.
        async fn follow(&self) -> SseReader {
            let mut events = self.stream().await;
            let opening = events.next().await.expect("the policy comes first");
            assert_eq!(opening.event, "policy");
            events
        }

        async fn publish(&self, event: RealtimeEvent) {
            self.bus.publish(event).await;
        }
    }

    fn revisions(data: &Value) -> Vec<String> {
        data.as_array()
            .unwrap()
            .iter()
            .map(|c| c["revision"].as_str().unwrap().to_owned())
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn changes_within_the_window_share_one_event() {
        let client = Client::new();
        let mut events = client.follow().await;
        let started = Instant::now();
        client.publish(comment(1, false)).await;
        let bus = client.bus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            bus.publish(comment(2, false)).await;
        });

        let batch = events.next().await.unwrap();
        assert_eq!(batch.event, "comments");
        assert_eq!(batch.id.as_deref(), Some("2"));
        assert_eq!(revisions(&batch.json()), ["1", "2"]);
        assert_eq!(
            started.elapsed(),
            BATCH_WINDOW,
            "sent when the window closed"
        );

        client.publish(comment(3, false)).await;
        let next = events.next().await.unwrap();
        assert_eq!(revisions(&next.json()), ["3"], "a new window");
    }

    #[tokio::test(start_paused = true)]
    async fn a_priority_comment_flushes_the_batch_at_once() {
        let client = Client::new();
        let mut events = client.follow().await;
        let started = Instant::now();
        client.publish(comment(1, false)).await;
        client.publish(comment(2, true)).await;

        let batch = events.next().await.unwrap();
        assert_eq!(revisions(&batch.json()), ["1", "2"]);
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn a_policy_change_follows_the_pending_comments_at_once() {
        let client = Client::new();
        let mut events = client.follow().await;
        let started = Instant::now();
        client.publish(comment(1, false)).await;
        client
            .publish(RealtimeEvent {
                broadcast: id(BROADCAST),
                priority: true,
                kind: EventKind::ChatPolicy(ChatPolicy::from_secs(30).unwrap()),
            })
            .await;

        assert_eq!(revisions(&events.next().await.unwrap().json()), ["1"]);
        let policy = events.next().await.unwrap();
        assert_eq!(
            (policy.event.as_str(), policy.id.as_deref(), policy.json()),
            ("policy", None, serde_json::json!({"slowModeSeconds": 30}))
        );
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn the_stream_outlives_the_request_time_limit() {
        let client = Client::new();
        let mut events = client.follow().await;
        tokio::time::sleep(REQUEST_TIMEOUT + Duration::from_secs(1)).await;
        client.publish(comment(1, true)).await;

        let batch = events.next().await.unwrap();
        assert_eq!(revisions(&batch.json()), ["1"]);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_ends_open_streams_and_new_ones_at_once() {
        let client = Client::new();
        let mut events = client.follow().await;
        client.state.streams.close_all();
        assert!(events.next().await.is_none(), "ended");

        let mut late = client.stream().await;
        assert!(late.next().await.is_none(), "ended as it started");
    }

    #[tokio::test]
    async fn a_client_may_hold_20_streams() {
        let client = Client::new();
        let mut open = Vec::new();
        for _ in 0..MAX_STREAMS_PER_CLIENT {
            open.push(client.follow().await);
        }
        let path = format!("/api/broadcasts/{BROADCAST}/events");
        let refused = client.open(&path, None).await;
        assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
        let body = axum::body::to_bytes(refused.into_body(), 4096)
            .await
            .unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["code"], "TOO_MANY_STREAMS");

        drop(open.pop());
        assert_eq!(
            client.state.streams.open_for("203.0.113.7"),
            MAX_STREAMS_PER_CLIENT - 1
        );
        client.follow().await;
        drop(open);
        assert_eq!(client.state.streams.open_for("203.0.113.7"), 0);
    }

    #[test]
    fn a_poisoned_count_still_admits_and_frees_slots() {
        let streams = Arc::new(LiveStreams::default());
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.9".parse().unwrap());
        let client = ClientKey::of(&headers, &axum::http::Extensions::new());
        let slot = streams.admit(&client).unwrap();
        let poisoner = Arc::clone(&streams);
        std::thread::spawn(move || {
            let _held = poisoner.open.lock();
            panic!("poison the count");
        })
        .join()
        .unwrap_err();

        drop(slot);
        let again = streams.admit(&client).unwrap();
        assert_eq!(streams.open_for("203.0.113.9"), 1);
        drop(again);
        assert_eq!(streams.open_for("203.0.113.9"), 0);
    }

    #[tokio::test]
    async fn a_resume_point_must_be_a_revision_and_the_header_wins() {
        let client = Client::new();
        let path = format!("/api/broadcasts/{BROADCAST}/events");
        for (query, header) in [
            ("?after=abc", None),
            ("", Some("x")),
            ("?after=1", Some("-1")),
        ] {
            let response = client.open(&format!("{path}{query}"), header).await;
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{query} {header:?}"
            );
        }
        let response = client.open(&format!("{path}?after=x"), Some("1")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(client.state.streams.open_for("203.0.113.7"), 1);
        drop(response);
        assert_eq!(
            client.state.streams.open_for("203.0.113.7"),
            0,
            "a closed stream frees its slot"
        );
    }

    #[tokio::test]
    async fn an_unknown_broadcast_is_not_found_before_the_stream_starts() {
        let client = Client::new();
        let response = client.open("/api/broadcasts/404/events", None).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let mut body = response.into_body().into_data_stream();
        let bytes = body.next().await.unwrap().unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["code"], "BROADCAST_NOT_FOUND");
        assert_eq!(client.state.streams.open_for("203.0.113.7"), 0);
    }
}
