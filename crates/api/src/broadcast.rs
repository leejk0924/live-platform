//! `/api/broadcasts`: translates HTTP to the broadcast use cases. The owner
//! is always the session user and the expected version always comes from
//! `If-Match`; neither is read from the body.

use std::{sync::Arc, time::SystemTime};

use application::{
    broadcast::{
        CreateBroadcast, DeleteBroadcast, Error, GetBroadcast, ListBroadcasts, Page,
        UpdateBroadcast,
    },
    ports::{
        auth::UserRepository,
        broadcast::{BroadcastQuery, BroadcastRepository, BroadcastView},
        id::IdGenerator,
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

use crate::{AppState, auth::AuthUser, error::ApiError};

/// The list is the same for every viewer, so shared caches may hold it
/// briefly.
pub const LIST_CACHE_CONTROL: &str = "public, max-age=5";

pub struct BroadcastUseCases {
    pub create: CreateBroadcast,
    pub update: UpdateBroadcast,
    pub delete: DeleteBroadcast,
    pub get: GetBroadcast,
    pub list: ListBroadcasts,
}

/// The ports the broadcast use cases need beyond the auth ones.
pub struct BroadcastPorts {
    pub repository: Arc<dyn BroadcastRepository>,
    pub query: Arc<dyn BroadcastQuery>,
}

impl BroadcastUseCases {
    pub fn new(
        ports: BroadcastPorts,
        ids: Arc<dyn IdGenerator>,
        users: Arc<dyn UserRepository>,
    ) -> Self {
        let BroadcastPorts { repository, query } = ports;
        Self {
            create: CreateBroadcast {
                ids,
                broadcasts: repository.clone(),
                users: users.clone(),
            },
            update: UpdateBroadcast {
                broadcasts: repository.clone(),
                users,
            },
            delete: DeleteBroadcast {
                broadcasts: repository,
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

fn api_error(err: Error) -> ApiError {
    match err {
        Error::Rejected(err) => err.into(),
        Error::Internal(err) => ApiError::internal(err),
    }
}

/// An id that is not a snowflake cannot name a broadcast.
fn broadcast_id(raw: &str) -> Result<BroadcastId, ApiError> {
    raw.parse().map_err(|_| BroadcastError::NotFound.into())
}

/// Accepts `"3"` and `W/"3"`.
fn expected_version(headers: &HeaderMap) -> Result<Version, ApiError> {
    let raw = headers
        .get(IF_MATCH)
        .ok_or_else(ApiError::if_match_required)?;
    let tag = raw
        .to_str()
        .map_err(|_| ApiError::invalid_if_match())?
        .trim();
    let tag = tag.strip_prefix("W/").unwrap_or(tag);
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

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastDto {
    #[serde(with = "crate::json::snowflake_id")]
    id: SnowflakeId,
    #[serde(with = "crate::json::snowflake_id")]
    owner_id: SnowflakeId,
    owner_name: String,
    title: String,
    description: String,
    status: BroadcastStatusDto,
    version: i32,
    /// RFC 3339.
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BroadcastPage {
    items: Vec<BroadcastDto>,
    /// Pass as `before` for the next page; `null` on the last page.
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

#[derive(Deserialize)]
pub struct BroadcastInput {
    title: String,
    description: String,
}

#[derive(Deserialize)]
pub struct ListParams {
    before: Option<String>,
    limit: Option<u32>,
}

/// `body` plus its version as the `ETag`.
fn tagged(status: StatusCode, view: BroadcastView) -> Response {
    let tag = etag(view.broadcast.version);
    (status, [(ETAG, tag)], Json(BroadcastDto::from(view))).into_response()
}

async fn list(
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

async fn get_one(
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

async fn create(
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

async fn update(
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

async fn delete(
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
    fn if_match_takes_a_strong_or_weak_version_tag() {
        assert_eq!(if_match(Some("\"3\"")), Ok(3));
        assert_eq!(if_match(Some("W/\"12\"")), Ok(12));
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
