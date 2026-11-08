//! `/api/broadcasts`: translates HTTP to the broadcast use cases. The owner
//! is always the session user and the expected version always comes from
//! `If-Match`; neither is read from the body.

use std::{sync::Arc, time::SystemTime};

use application::{
    broadcast::{
        CreateBroadcast, DeleteBroadcast, Error, GetBroadcast, ListBroadcasts, Page,
        UpdateBroadcast,
    },
    live::{StartBroadcast, StopBroadcast},
    media::{AdmitMediaAccess, TrackPublisher},
    ports::{
        auth::UserRepository,
        broadcast::{BroadcastQuery, BroadcastRepository, BroadcastView, PublisherPresence},
        id::IdGenerator,
        media::{
            IngestRouter, MediaCredentialVerifier, PublishAuthorizer, PublishTicketLedger,
            StreamTerminator,
        },
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection, rejection::QueryRejection},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, ETAG, IF_MATCH, LOCATION},
    },
    response::{IntoResponse, Response},
    routing::get,
};
use domain::{
    broadcast::{BroadcastError, BroadcastId, BroadcastStatus, Version},
    id::SnowflakeId,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use utoipa::{IntoParams, ToSchema};

use crate::{
    AppState,
    auth::AuthUser,
    error::{ApiError, ErrorBody},
};

/// The list is the same for every viewer, so shared caches may hold it
/// briefly.
pub const LIST_CACHE_CONTROL: &str = "public, max-age=5";

pub struct BroadcastUseCases {
    pub create: CreateBroadcast,
    pub update: UpdateBroadcast,
    pub delete: DeleteBroadcast,
    pub get: GetBroadcast,
    pub list: ListBroadcasts,
    pub start: StartBroadcast,
    pub stop: StopBroadcast,
    /// Answers MediaMTX's question whether to let a publisher in.
    pub admit: AdmitMediaAccess,
    /// Follows MediaMTX's reports that a publisher arrived or left.
    pub track: TrackPublisher,
}

/// The ports the broadcast use cases need beyond the auth ones.
pub struct BroadcastPorts {
    pub repository: Arc<dyn BroadcastRepository>,
    pub query: Arc<dyn BroadcastQuery>,
    pub ingest: Arc<dyn IngestRouter>,
    pub authorizer: Arc<dyn PublishAuthorizer>,
    pub terminator: Arc<dyn StreamTerminator>,
    pub verifier: Arc<dyn MediaCredentialVerifier>,
    pub tickets: Arc<dyn PublishTicketLedger>,
    pub presence: Arc<dyn PublisherPresence>,
}

impl BroadcastUseCases {
    pub fn new(
        ports: BroadcastPorts,
        ids: Arc<dyn IdGenerator>,
        users: Arc<dyn UserRepository>,
    ) -> Self {
        let BroadcastPorts {
            repository,
            query,
            ingest,
            authorizer,
            terminator,
            verifier,
            tickets,
            presence,
        } = ports;
        Self {
            create: CreateBroadcast {
                ids,
                broadcasts: repository.clone(),
                users: users.clone(),
            },
            update: UpdateBroadcast {
                broadcasts: repository.clone(),
                users: users.clone(),
            },
            delete: DeleteBroadcast {
                broadcasts: repository.clone(),
            },
            start: StartBroadcast {
                broadcasts: repository.clone(),
                ingest: ingest.clone(),
                authorizer,
            },
            admit: AdmitMediaAccess {
                verifier,
                ingest: ingest.clone(),
                tickets,
            },
            track: TrackPublisher {
                presence,
                ingest: ingest.clone(),
            },
            stop: StopBroadcast {
                broadcasts: repository,
                users,
                ingest,
                terminator,
            },
            get: GetBroadcast {
                query: query.clone(),
            },
            list: ListBroadcasts { query },
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/broadcasts", get(list).post(create))
        .route("/broadcasts/{id}", get(get_one).put(update).delete(delete))
}

pub(crate) fn api_error(err: Error) -> ApiError {
    match err {
        Error::Rejected(err) => err.into(),
        Error::Internal(err) => ApiError::internal(err),
    }
}

/// An id that is not a snowflake cannot name a broadcast.
pub(crate) fn broadcast_id(raw: &str) -> Result<BroadcastId, ApiError> {
    raw.parse().map_err(|_| BroadcastError::NotFound.into())
}

/// Accepts only a strong tag such as `"3"`: RFC 9110 compares `If-Match`
/// strongly, so a weak tag can never match.
fn expected_version(headers: &HeaderMap) -> Result<Version, ApiError> {
    let raw = headers
        .get(IF_MATCH)
        .ok_or_else(ApiError::if_match_required)?;
    let tag = raw
        .to_str()
        .map_err(|_| ApiError::invalid_if_match())?
        .trim();
    tag.strip_prefix('"')
        .and_then(|tag| tag.strip_suffix('"'))
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .and_then(Version::new)
        .ok_or_else(ApiError::invalid_if_match)
}

fn etag(version: Version) -> HeaderValue {
    HeaderValue::try_from(format!("\"{}\"", version.get())).expect("digits are a valid header")
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[schema(as = BroadcastStatus)]
pub enum BroadcastStatusDto {
    Ready,
    Live,
    Interrupted,
    Ended,
}

impl From<BroadcastStatus> for BroadcastStatusDto {
    fn from(status: BroadcastStatus) -> Self {
        match status {
            BroadcastStatus::Ready => Self::Ready,
            BroadcastStatus::Live => Self::Live,
            BroadcastStatus::Interrupted => Self::Interrupted,
            BroadcastStatus::Ended => Self::Ended,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Broadcast)]
pub struct BroadcastDto {
    /// Snowflake id as a decimal string.
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007303372210176")]
    id: SnowflakeId,
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007302751453184")]
    owner_id: SnowflakeId,
    owner_name: String,
    title: String,
    description: String,
    status: BroadcastStatusDto,
    /// Send back as `If-Match: "<version>"` to update.
    #[schema(minimum = 1)]
    version: i32,
    #[schema(format = DateTime)]
    created_at: String,
}

fn rfc3339(at: SystemTime) -> String {
    OffsetDateTime::from(at)
        .format(&Rfc3339)
        .expect("a SystemTime from the database fits RFC 3339")
}

impl From<BroadcastView> for BroadcastDto {
    fn from(view: BroadcastView) -> Self {
        let b = view.broadcast;
        Self {
            id: b.id,
            owner_id: b.owner_id,
            owner_name: view.owner_name.as_str().to_owned(),
            title: b.title.as_str().to_owned(),
            description: b.description.as_str().to_owned(),
            status: b.status.into(),
            version: b.version.get(),
            created_at: rfc3339(b.created_at),
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastPage {
    items: Vec<BroadcastDto>,
    /// Pass as `before` for the next page; `null` on the last page.
    #[schema(required = true)]
    next_before: Option<String>,
}

impl From<Page> for BroadcastPage {
    fn from(page: Page) -> Self {
        Self {
            items: page.items.into_iter().map(BroadcastDto::from).collect(),
            next_before: page.next_before.map(|id| id.to_string()),
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct BroadcastInput {
    /// Trimmed, then 1 to 100 characters.
    title: String,
    /// At most 2000 characters; may be empty.
    description: String,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// Only broadcasts older than this id: the previous page's `nextBefore`.
    before: Option<String>,
    /// Page size, 1 to 50. Defaults to 20.
    #[param(minimum = 1, maximum = 50)]
    limit: Option<u32>,
}

/// `body` plus its version as the `ETag`.
pub(crate) fn tagged(status: StatusCode, view: BroadcastView) -> Response {
    let tag = etag(view.broadcast.version);
    (status, [(ETAG, tag)], Json(BroadcastDto::from(view))).into_response()
}

#[utoipa::path(
    get,
    path = "/api/broadcasts",
    operation_id = "listBroadcasts",
    tag = "broadcasts",
    summary = "List broadcasts, newest first",
    params(ListParams),
    responses(
        (status = 200, body = BroadcastPage, description = "The same for every viewer.",
            headers(("Cache-Control" = String, description = "`public, max-age=5`"))),
        (status = 400, body = ErrorBody, description = "`INVALID_QUERY` or `INVALID_PAGE_SIZE`"),
    ),
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    params: Result<Query<ListParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let Query(params) = params.map_err(|_| ApiError::invalid_query())?;
    let before = params
        .before
        .map(|raw| raw.parse::<BroadcastId>())
        .transpose()
        .map_err(|_| ApiError::invalid_query())?;
    let page = state
        .broadcasts
        .list
        .run(before, params.limit)
        .await
        .map_err(api_error)?;
    Ok((
        [(CACHE_CONTROL, HeaderValue::from_static(LIST_CACHE_CONTROL))],
        Json(BroadcastPage::from(page)),
    )
        .into_response())
}

#[utoipa::path(
    get,
    path = "/api/broadcasts/{id}",
    operation_id = "getBroadcast",
    tag = "broadcasts",
    summary = "Get one broadcast",
    params(("id" = String, Path, description = "Broadcast id")),
    responses(
        (status = 200, body = BroadcastDto, description = "The broadcast.",
            headers(("ETag" = String, description = "The version as `\"<version>\"`"))),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`, also for deleted broadcasts"),
    ),
)]
pub(crate) async fn get_one(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let view = state
        .broadcasts
        .get
        .run(broadcast_id(&id)?)
        .await
        .map_err(api_error)?;
    Ok(tagged(StatusCode::OK, view))
}

#[utoipa::path(
    post,
    path = "/api/broadcasts",
    operation_id = "createBroadcast",
    tag = "broadcasts",
    summary = "Register a broadcast owned by the signed-in user",
    request_body = BroadcastInput,
    security(("session" = [])),
    responses(
        (status = 201, body = BroadcastDto, description = "Created in `READY` at version 1.",
            headers(
                ("Location" = String, description = "`/api/broadcasts/<id>`"),
                ("ETag" = String, description = "`\"1\"`"),
            )),
        (status = 400, body = ErrorBody, description = "`INVALID_TITLE`, `INVALID_DESCRIPTION` or `INVALID_REQUEST`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`CSRF_REJECTED`"),
    ),
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    body: Result<Json<BroadcastInput>, JsonRejection>,
) -> Result<Response, ApiError> {
    let Json(body) = body.map_err(|_| ApiError::invalid_request())?;
    let view = state
        .broadcasts
        .create
        .run(actor, &body.title, &body.description)
        .await
        .map_err(api_error)?;
    let location = HeaderValue::try_from(format!("/api/broadcasts/{}", view.broadcast.id))
        .expect("digits are a valid header");
    let mut response = tagged(StatusCode::CREATED, view);
    response.headers_mut().insert(LOCATION, location);
    Ok(response)
}

#[utoipa::path(
    put,
    path = "/api/broadcasts/{id}",
    operation_id = "updateBroadcast",
    tag = "broadcasts",
    summary = "Replace title and description; owner only",
    params(
        ("id" = String, Path, description = "Broadcast id"),
        ("If-Match" = String, Header, description = "The version being edited, as returned in `ETag`: `\"3\"`"),
    ),
    request_body = BroadcastInput,
    security(("session" = [])),
    responses(
        (status = 200, body = BroadcastDto, description = "Updated.",
            headers(("ETag" = String, description = "The new version"))),
        (status = 400, body = ErrorBody, description = "`IF_MATCH_REQUIRED`, `INVALID_IF_MATCH`, `INVALID_TITLE`, `INVALID_DESCRIPTION` or `INVALID_REQUEST`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_BROADCAST_OWNER` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
        (status = 409, body = ErrorBody, description = "`VERSION_CONFLICT`: changed since it was loaded"),
    ),
)]
pub(crate) async fn update(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Result<Json<BroadcastInput>, JsonRejection>,
) -> Result<Response, ApiError> {
    let id = broadcast_id(&id)?;
    let expected = expected_version(&headers)?;
    let Json(body) = body.map_err(|_| ApiError::invalid_request())?;
    let view = state
        .broadcasts
        .update
        .run(actor, id, expected, &body.title, &body.description)
        .await
        .map_err(api_error)?;
    Ok(tagged(StatusCode::OK, view))
}

#[utoipa::path(
    delete,
    path = "/api/broadcasts/{id}",
    operation_id = "deleteBroadcast",
    tag = "broadcasts",
    summary = "Delete a READY or ENDED broadcast; owner only",
    params(("id" = String, Path, description = "Broadcast id")),
    security(("session" = [])),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_BROADCAST_OWNER` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
        (status = 409, body = ErrorBody, description = "`BROADCAST_NOT_DELETABLE`: live or interrupted"),
    ),
)]
pub(crate) async fn delete(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .broadcasts
        .delete
        .run(actor, broadcast_id(&id)?)
        .await
        .map_err(api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn if_match(value: Option<&str>) -> Result<i32, &'static str> {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(IF_MATCH, HeaderValue::from_str(value).unwrap());
        }
        expected_version(&headers)
            .map(Version::get)
            .map_err(|err| err.code())
    }

    #[test]
    fn if_match_takes_a_strong_version_tag() {
        assert_eq!(if_match(Some("\"3\"")), Ok(3));
        assert_eq!(if_match(Some("\"12\"")), Ok(12));
        assert_eq!(if_match(Some(" \"1\" ")), Ok(1));
    }

    #[test]
    fn if_match_is_required_and_must_be_a_positive_version() {
        assert_eq!(if_match(None), Err("IF_MATCH_REQUIRED"));
        for bad in [
            "3",
            "\"\"",
            "\"0\"",
            "\"-1\"",
            "\"+1\"",
            "\"1.0\"",
            "*",
            "\"1\", \"2\"",
            "W/\"1\"",
            "w/\"1\"",
            "\"99999999999\"",
        ] {
            assert_eq!(if_match(Some(bad)), Err("INVALID_IF_MATCH"), "{bad}");
        }
    }

    #[test]
    fn etag_round_trips_through_if_match() {
        let tag = etag(Version::new(42).unwrap());
        assert_eq!(tag, "\"42\"");
        assert_eq!(if_match(Some(tag.to_str().unwrap())), Ok(42));
    }
}
