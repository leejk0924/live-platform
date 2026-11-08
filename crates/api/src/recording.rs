//! `/api/recordings`: browsing and replaying recordings, open to everyone
//! like live playback, and the stored files themselves when this host keeps
//! them.

use std::{sync::Arc, time::SystemTime};

use application::{
    ports::{
        id::IdGenerator,
        media::IngestRouter,
        recording::{
            RecordingFilter, RecordingQuery, RecordingRepository, RecordingView, SegmentSpool,
            UrlSigner,
        },
    },
    recording::{
        Error, GetRecording, ListRecordings, Page, RegisterSegment, Replay, ResolveReplay,
    },
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, Request, State, rejection::QueryRejection},
    http::{
        HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use domain::{
    id::SnowflakeId,
    recording::{RecordingError, RecordingId, RecordingStatus},
};
use infrastructure::recording::{FsRecordingStorage, LocalUrlSigner};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tower_http::services::ServeFile;
use utoipa::{IntoParams, ToSchema};

use crate::{
    AppState,
    broadcast::rfc3339,
    error::{ApiError, ErrorBody},
};

/// The same for every viewer; a few seconds of staleness only delays
/// noticing that a recording became ready.
pub const LIST_CACHE_CONTROL: &str = "public, max-age=5";

/// A signed URL is for whoever asked, and a stored file must stop being
/// served the moment its broadcast is deleted, so neither may be cached.
pub const REPLAY_CACHE_CONTROL: &str = "no-store";
pub const FILE_CACHE_CONTROL: &str = "private, no-store";

/// Recordings kept on this host, which the API serves itself through URLs
/// it signed.
pub struct LocalRecordings {
    pub signer: Arc<LocalUrlSigner>,
    pub storage: Arc<FsRecordingStorage>,
}

pub struct RecordingPorts {
    pub repository: Arc<dyn RecordingRepository>,
    pub query: Arc<dyn RecordingQuery>,
    pub spool: Arc<dyn SegmentSpool>,
    pub signer: Arc<dyn UrlSigner>,
    /// `None` when another service serves the files.
    pub local: Option<LocalRecordings>,
}

pub struct RecordingUseCases {
    pub register: RegisterSegment,
    pub list: ListRecordings,
    pub get: GetRecording,
    pub replay: ResolveReplay,
    /// Wakes the storer after a segment is registered, so it need not wait
    /// for its next tick.
    pub stored_wake: Arc<Notify>,
    pub local: Option<LocalRecordings>,
}

impl RecordingUseCases {
    pub fn new(
        ports: RecordingPorts,
        ingest: Arc<dyn IngestRouter>,
        ids: Arc<dyn IdGenerator>,
    ) -> Self {
        let RecordingPorts {
            repository,
            query,
            spool,
            signer,
            local,
        } = ports;
        Self {
            register: RegisterSegment {
                ingest,
                spool,
                recordings: repository,
                ids,
            },
            list: ListRecordings {
                query: query.clone(),
            },
            get: GetRecording {
                query: query.clone(),
            },
            replay: ResolveReplay { query, signer },
            stored_wake: Arc::new(Notify::new()),
            local,
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/recordings", get(list))
        .route("/recordings/{id}", get(get_one))
        .route("/recordings/{id}/replay", get(replay))
}

fn api_error(err: Error) -> ApiError {
    match err {
        Error::Rejected(err) => err.into(),
        Error::Internal(err) => ApiError::internal(err),
    }
}

/// An id that is not a snowflake cannot name a recording.
fn recording_id(raw: &str) -> Result<RecordingId, ApiError> {
    raw.parse().map_err(|_| RecordingError::NotFound.into())
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(as = RecordingStatus)]
pub enum RecordingStatusDto {
    /// Waiting to be stored; not replayable yet.
    Pending,
    Ready,
    /// Could not be stored; it will not become replayable.
    Failed,
}

impl From<RecordingStatus> for RecordingStatusDto {
    fn from(status: RecordingStatus) -> Self {
        match status {
            RecordingStatus::Pending => Self::Pending,
            RecordingStatus::Ready => Self::Ready,
            RecordingStatus::Failed => Self::Failed,
        }
    }
}

/// One recorded segment of a broadcast. A broadcast may have several: one
/// per publisher connection, and one per 30 minutes of a long one.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Recording)]
pub struct RecordingDto {
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007303372210177")]
    id: SnowflakeId,
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007303372210176")]
    broadcast_id: SnowflakeId,
    broadcast_title: String,
    owner_name: String,
    status: RecordingStatusDto,
    duration_ms: u64,
    /// Known once `READY`.
    #[schema(required = true)]
    size_bytes: Option<u64>,
    /// When the segment finished.
    #[schema(format = DateTime)]
    recorded_at: String,
}

impl From<RecordingView> for RecordingDto {
    fn from(view: RecordingView) -> Self {
        Self {
            id: view.id,
            broadcast_id: view.broadcast_id,
            broadcast_title: view.broadcast_title.as_str().to_owned(),
            owner_name: view.owner_name.as_str().to_owned(),
            status: view.status.into(),
            duration_ms: u64::try_from(view.duration.as_millis()).unwrap_or(u64::MAX),
            size_bytes: view.size_bytes,
            recorded_at: rfc3339(view.recorded_at),
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPage {
    items: Vec<RecordingDto>,
    /// Pass as `before` for the next page; `null` on the last page.
    #[schema(required = true)]
    next_before: Option<String>,
}

impl From<Page> for RecordingPage {
    fn from(page: Page) -> Self {
        Self {
            items: page.items.into_iter().map(RecordingDto::from).collect(),
            next_before: page.next_before.map(|id| id.to_string()),
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// Only this broadcast's recordings.
    broadcast_id: Option<String>,
    /// Only recordings older than this id: the previous page's `nextBefore`.
    before: Option<String>,
    /// Page size, 1 to 50. Defaults to 20.
    #[param(minimum = 1, maximum = 50)]
    limit: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/api/recordings",
    operation_id = "listRecordings",
    tag = "recordings",
    summary = "List recordings, newest first",
    params(ListParams),
    responses(
        (status = 200, body = RecordingPage, description = "The same for every viewer. Recordings of deleted broadcasts are left out.",
            headers(("Cache-Control" = String, description = "`public, max-age=5`"))),
        (status = 400, body = ErrorBody, description = "`INVALID_QUERY` or `INVALID_PAGE_SIZE`"),
    ),
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    params: Result<Query<ListParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(params) = params.map_err(|_| ApiError::invalid_query())?;
    let id = |raw: Option<String>| {
        raw.map(|raw| raw.parse::<SnowflakeId>())
            .transpose()
            .map_err(|_| ApiError::invalid_query())
    };
    let filter = match id(params.broadcast_id)? {
        None => RecordingFilter::All,
        Some(broadcast) => RecordingFilter::Broadcast(broadcast),
    };
    let page = state
        .recordings
        .list
        .run(filter, id(params.before)?, params.limit)
        .await
        .map_err(api_error)?;
    Ok((
        [(CACHE_CONTROL, HeaderValue::from_static(LIST_CACHE_CONTROL))],
        Json(RecordingPage::from(page)),
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/api/recordings/{id}",
    operation_id = "getRecording",
    tag = "recordings",
    summary = "Get one recording",
    params(("id" = String, Path, description = "Recording id")),
    responses(
        (status = 200, body = RecordingDto, description = "The recording.",
            headers(("Cache-Control" = String, description = "`public, max-age=5`"))),
        (status = 404, body = ErrorBody, description = "`RECORDING_NOT_FOUND`, also when its broadcast was deleted"),
    ),
)]
pub(crate) async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let view = state
        .recordings
        .get
        .run(recording_id(&id)?)
        .await
        .map_err(api_error)?;
    Ok((
        [(CACHE_CONTROL, HeaderValue::from_static(LIST_CACHE_CONTROL))],
        Json(RecordingDto::from(view)),
    )
        .into_response())
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Replay)]
pub struct ReplayDto {
    /// An MP4 to play in a `<video>` element. It supports range requests,
    /// so seeking works. Ask again for a new one when playback fails.
    #[schema(example = "/api/recordings/112007303372210177/file?expires=1794200000&signature=...")]
    url: String,
    /// When `url` stops working: five minutes from now, whatever the
    /// recording's length.
    #[schema(format = DateTime)]
    expires_at: String,
}

impl From<Replay> for ReplayDto {
    fn from(replay: Replay) -> Self {
        Self {
            url: replay.url,
            expires_at: rfc3339(replay.expires_at),
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/recordings/{id}/replay",
    operation_id = "getReplay",
    tag = "recordings",
    summary = "A short-lived URL to watch a ready recording",
    params(("id" = String, Path, description = "Recording id")),
    responses(
        (status = 200, body = ReplayDto, description = "A URL signed for this request.",
            headers(("Cache-Control" = String, description = "`no-store`"))),
        (status = 404, body = ErrorBody, description = "`RECORDING_NOT_FOUND`, also when its broadcast was deleted"),
        (status = 409, body = ErrorBody, description = "`RECORDING_NOT_READY`: pending or failed"),
    ),
)]
pub(crate) async fn replay(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let replay = state
        .recordings
        .replay
        .run(recording_id(&id)?, SystemTime::now())
        .await
        .map_err(api_error)?;
    Ok((
        [(
            CACHE_CONTROL,
            HeaderValue::from_static(REPLAY_CACHE_CONTROL),
        )],
        Json(ReplayDto::from(replay)),
    )
        .into_response())
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FileParams {
    /// Unix seconds, from the replay URL.
    expires: u64,
    /// From the replay URL.
    signature: String,
}

#[utoipa::path(
    get,
    path = "/api/recordings/{id}/file",
    operation_id = "getRecordingFile",
    tag = "recordings",
    summary = "The recording itself, through a URL from getReplay",
    description = "Served only when recordings are kept on the API host; otherwise getReplay points at the CDN. Exempt from the 10-second request limit.",
    params(("id" = String, Path, description = "Recording id"), FileParams),
    responses(
        (status = 200, content_type = "video/mp4", description = "The whole file.",
            headers(("Cache-Control" = String, description = "`private, no-store`"))),
        (status = 206, content_type = "video/mp4", description = "The requested `Range`."),
        (status = 403, body = ErrorBody, description = "`INVALID_SIGNATURE`: the URL was altered or has expired"),
        (status = 404, body = ErrorBody, description = "`RECORDING_NOT_FOUND`, also when its broadcast was deleted after the URL was issued"),
    ),
)]
pub(crate) async fn file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    params: Result<Query<FileParams>, QueryRejection>,
    request: Request,
) -> Result<Response, ApiError> {
    let local = state
        .recordings
        .local
        .as_ref()
        .ok_or_else(ApiError::not_found)?;
    let id = recording_id(&id)?;
    let Query(params) = params.map_err(|_| ApiError::invalid_signature())?;
    if !local
        .signer
        .verify(id, params.expires, &params.signature, SystemTime::now())
    {
        return Err(ApiError::invalid_signature());
    }
    // Checked on every request, so deleting the broadcast stops a URL that
    // was already handed out.
    let view = state.recordings.get.run(id).await.map_err(api_error)?;
    if view.status != RecordingStatus::Ready {
        return Err(RecordingError::NotReady.into());
    }
    let response = ServeFile::new(local.storage.file(&view.key))
        .try_call(request)
        .await
        .map_err(ApiError::internal)?;
    if response.status() == StatusCode::NOT_FOUND {
        return Err(ApiError::internal(format_args!(
            "stored file of ready recording {id} is missing"
        )));
    }
    let mut response = response.map(Body::new);
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("video/mp4"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(FILE_CACHE_CONTROL));
    Ok(response)
}
