//! `/api/broadcasts/{id}/comments` and `/chat-policy`: translates HTTP to
//! the comment use cases. The author and the broadcast owner are always the
//! session user, and the expected version always comes from `If-Match`.

use std::{sync::Arc, time::SystemTime};

use application::{
    comment::{
        DeleteComment, EditComment, Error, FollowComments, GetChatPolicy, ListComments, Page,
        PostComment, SetChatPolicy,
    },
    ports::{
        comment::{ChatPolicyStore, CommentQuery, CommentRepository, CommentView},
        donation::DonationQuery,
        event_bus::EventBus,
        id::IdGenerator,
        rate_limit::RateLimiter,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection, rejection::QueryRejection},
    http::{HeaderMap, StatusCode, header::ETAG},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use domain::{
    comment::{ChatPolicy, CommentError, CommentId},
    id::SnowflakeId,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use utoipa::{IntoParams, ToSchema};

use crate::{
    AppState,
    auth::AuthUser,
    broadcast::broadcast_id,
    error::{ApiError, ErrorBody},
    if_match::{etag, expected_version},
};

pub struct CommentUseCases {
    pub post: PostComment,
    pub edit: EditComment,
    pub delete: DeleteComment,
    pub list: ListComments,
    pub get_policy: GetChatPolicy,
    pub set_policy: SetChatPolicy,
    pub follow: FollowComments,
}

/// The ports the comment use cases need beyond the id generator.
pub struct CommentPorts {
    pub repository: Arc<dyn CommentRepository>,
    pub query: Arc<dyn CommentQuery>,
    pub policies: Arc<dyn ChatPolicyStore>,
    pub limiter: Arc<dyn RateLimiter>,
    /// The donation totals the event stream sends with the policy.
    pub donations: Arc<dyn DonationQuery>,
    pub events: Arc<dyn EventBus>,
}

impl CommentUseCases {
    pub fn new(ports: CommentPorts, ids: Arc<dyn IdGenerator>) -> Self {
        let CommentPorts {
            repository,
            query,
            policies,
            limiter,
            donations,
            events,
        } = ports;
        Self {
            post: PostComment {
                ids,
                comments: repository.clone(),
                policies: policies.clone(),
                limiter,
            },
            edit: EditComment {
                comments: repository.clone(),
            },
            delete: DeleteComment {
                comments: repository,
            },
            list: ListComments {
                query: query.clone(),
                policies: policies.clone(),
            },
            get_policy: GetChatPolicy {
                policies: policies.clone(),
            },
            set_policy: SetChatPolicy {
                policies: policies.clone(),
                events: events.clone(),
            },
            follow: FollowComments {
                query,
                policies,
                donations,
                events,
            },
        }
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/broadcasts/{id}/comments", get(list).post(post))
        .route(
            "/broadcasts/{id}/comments/{commentId}",
            put(edit).delete(delete),
        )
        .route(
            "/broadcasts/{id}/chat-policy",
            get(get_policy).put(set_policy),
        )
}

pub(crate) fn api_error(err: Error) -> ApiError {
    match err {
        Error::Rejected(err) => err.into(),
        Error::RateLimited(wait) => ApiError::rate_limited(wait),
        Error::Internal(err) => ApiError::internal(err),
    }
}

/// An id that is not a snowflake cannot name a comment.
fn comment_id(raw: &str) -> Result<CommentId, ApiError> {
    raw.parse().map_err(|_| CommentError::NotFound.into())
}

pub(crate) fn rfc3339(at: SystemTime) -> String {
    OffsetDateTime::from(at)
        .format(&Rfc3339)
        .expect("a SystemTime from the database fits RFC 3339")
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Comment)]
pub struct CommentDto {
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007303372210176")]
    id: SnowflakeId,
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007302751453184")]
    broadcast_id: SnowflakeId,
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007302751453184")]
    author_id: SnowflakeId,
    author_name: String,
    /// `null` once deleted. Plain text: escape it when rendering.
    #[schema(required = true)]
    body: Option<String>,
    deleted: bool,
    edited: bool,
    /// Send back as `If-Match: "<version>"` to edit.
    #[schema(minimum = 1)]
    version: i32,
    /// Position of this comment's latest change in the broadcast's change
    /// feed, as a decimal string. Of two copies of one comment, keep the one
    /// with the higher revision.
    #[schema(example = "1042")]
    revision: String,
    #[schema(format = DateTime)]
    created_at: String,
}

impl From<CommentView> for CommentDto {
    fn from(view: CommentView) -> Self {
        let c = view.comment;
        Self {
            id: c.id,
            broadcast_id: c.broadcast_id,
            author_id: c.author_id,
            author_name: view.author_name.as_str().to_owned(),
            deleted: c.body.is_none(),
            body: c.body.map(|body| body.as_str().to_owned()),
            edited: c.edited,
            version: c.version.get(),
            revision: view.revision.get().to_string(),
            created_at: rfc3339(c.created_at),
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommentPage {
    /// Newest first.
    items: Vec<CommentDto>,
    /// Pass as `before` for the next page; `null` on the last page.
    #[schema(required = true)]
    next_before: Option<String>,
    /// The newest revision when the page was read, `"0"` before the first
    /// comment. Open the event stream with `?after=<revision>` to receive
    /// every change since.
    revision: String,
}

impl From<Page> for CommentPage {
    fn from(page: Page) -> Self {
        Self {
            items: page.items.into_iter().map(CommentDto::from).collect(),
            next_before: page.next_before.map(|id| id.to_string()),
            revision: page.revision.get().to_string(),
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct CommentInput {
    /// Trimmed, then 1 to 500 characters on one line.
    body: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = ChatPolicy)]
pub struct ChatPolicyDto {
    /// Seconds a viewer waits between comments to this broadcast, 0 to 300;
    /// 0 is off. The broadcaster is never slowed down.
    #[schema(minimum = 0, maximum = 300)]
    slow_mode_seconds: u32,
}

impl From<ChatPolicy> for ChatPolicyDto {
    fn from(policy: ChatPolicy) -> Self {
        Self {
            slow_mode_seconds: policy.slow_mode_secs(),
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// Only comments older than this id: the previous page's `nextBefore`.
    before: Option<String>,
    /// Page size, 1 to 100. Defaults to 50.
    #[param(minimum = 1, maximum = 100)]
    limit: Option<u32>,
}

fn tagged(status: StatusCode, view: CommentView) -> Response {
    let tag = etag(view.comment.version);
    (status, [(ETAG, tag)], Json(CommentDto::from(view))).into_response()
}

#[utoipa::path(
    get,
    path = "/api/broadcasts/{id}/comments",
    operation_id = "listComments",
    tag = "comments",
    summary = "List a broadcast's comments, newest first",
    params(("id" = String, Path, description = "Broadcast id"), ListParams),
    responses(
        (status = 200, body = CommentPage, description = "Deleted comments are left out."),
        (status = 400, body = ErrorBody, description = "`INVALID_QUERY` or `INVALID_PAGE_SIZE`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
    ),
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
    params: Result<Query<ListParams>, QueryRejection>,
) -> Result<Json<CommentPage>, ApiError> {
    let broadcast = broadcast_id(&id)?;
    let Query(params) = params.map_err(|_| ApiError::invalid_query())?;
    let before = params
        .before
        .map(|raw| raw.parse::<CommentId>())
        .transpose()
        .map_err(|_| ApiError::invalid_query())?;
    let page = state
        .comments
        .list
        .run(broadcast, before, params.limit)
        .await
        .map_err(api_error)?;
    Ok(Json(page.into()))
}

#[utoipa::path(
    post,
    path = "/api/broadcasts/{id}/comments",
    operation_id = "postComment",
    tag = "comments",
    summary = "Post a comment as the signed-in user",
    params(("id" = String, Path, description = "Broadcast id")),
    request_body = CommentInput,
    security(("session" = [])),
    responses(
        (status = 201, body = CommentDto, description = "Posted at version 1.",
            headers(("ETag" = String, description = "`\"1\"`"))),
        (status = 400, body = ErrorBody, description = "`INVALID_COMMENT` or `INVALID_REQUEST`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
        (status = 409, body = ErrorBody, description = "`COMMENTS_CLOSED`: the broadcast has ended"),
        (status = 429, body = ErrorBody, description = "`RATE_LIMITED`: one comment per second per user, \
            and the broadcast's slow mode for viewers",
            headers(("Retry-After" = String, description = "Seconds until posting is allowed"))),
    ),
)]
pub(crate) async fn post(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path(id): Path<String>,
    body: Result<Json<CommentInput>, JsonRejection>,
) -> Result<Response, ApiError> {
    let broadcast = broadcast_id(&id)?;
    let Json(body) = body.map_err(|rejection| ApiError::rejected_body(rejection.status()))?;
    let view = state
        .comments
        .post
        .run(actor, broadcast, &body.body)
        .await
        .map_err(api_error)?;
    Ok(tagged(StatusCode::CREATED, view))
}

#[utoipa::path(
    put,
    path = "/api/broadcasts/{id}/comments/{commentId}",
    operation_id = "editComment",
    tag = "comments",
    summary = "Replace a comment's body; author only",
    params(
        ("id" = String, Path, description = "Broadcast id"),
        ("commentId" = String, Path, description = "Comment id"),
        ("If-Match" = String, Header, description = "The version being edited, as returned in `ETag`: `\"3\"`"),
    ),
    request_body = CommentInput,
    security(("session" = [])),
    responses(
        (status = 200, body = CommentDto, description = "Edited.",
            headers(("ETag" = String, description = "The new version"))),
        (status = 400, body = ErrorBody, description = "`IF_MATCH_REQUIRED`, `INVALID_IF_MATCH`, `INVALID_COMMENT` or `INVALID_REQUEST`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_COMMENT_AUTHOR` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`COMMENT_NOT_FOUND`, also for deleted comments"),
        (status = 409, body = ErrorBody, description = "`VERSION_CONFLICT`: changed since it was loaded"),
    ),
)]
pub(crate) async fn edit(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path((id, comment)): Path<(String, String)>,
    headers: HeaderMap,
    body: Result<Json<CommentInput>, JsonRejection>,
) -> Result<Response, ApiError> {
    let broadcast = broadcast_id(&id)?;
    let comment = comment_id(&comment)?;
    let expected = expected_version(&headers)?;
    let Json(body) = body.map_err(|rejection| ApiError::rejected_body(rejection.status()))?;
    let view = state
        .comments
        .edit
        .run(actor, broadcast, comment, expected, &body.body)
        .await
        .map_err(api_error)?;
    Ok(tagged(StatusCode::OK, view))
}

#[utoipa::path(
    delete,
    path = "/api/broadcasts/{id}/comments/{commentId}",
    operation_id = "deleteComment",
    tag = "comments",
    summary = "Delete a comment; author only",
    params(
        ("id" = String, Path, description = "Broadcast id"),
        ("commentId" = String, Path, description = "Comment id"),
    ),
    security(("session" = [])),
    responses(
        (status = 204, description = "Deleted; followers receive it with `body: null`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_COMMENT_AUTHOR` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`COMMENT_NOT_FOUND`"),
    ),
)]
pub(crate) async fn delete(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path((id, comment)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let broadcast = broadcast_id(&id)?;
    let comment = comment_id(&comment)?;
    state
        .comments
        .delete
        .run(actor, broadcast, comment)
        .await
        .map_err(api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/broadcasts/{id}/chat-policy",
    operation_id = "getChatPolicy",
    tag = "comments",
    summary = "Get a broadcast's chat policy",
    params(("id" = String, Path, description = "Broadcast id")),
    responses(
        (status = 200, body = ChatPolicyDto, description = "The policy."),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
    ),
)]
pub(crate) async fn get_policy(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<ChatPolicyDto>, ApiError> {
    let policy = state
        .comments
        .get_policy
        .run(broadcast_id(&id)?)
        .await
        .map_err(api_error)?;
    Ok(Json(policy.into()))
}

#[utoipa::path(
    put,
    path = "/api/broadcasts/{id}/chat-policy",
    operation_id = "setChatPolicy",
    tag = "comments",
    summary = "Set a broadcast's chat policy; owner only",
    description = "Does not change the broadcast's version, so an open edit of its title is unaffected.",
    params(("id" = String, Path, description = "Broadcast id")),
    request_body = ChatPolicyDto,
    security(("session" = [])),
    responses(
        (status = 200, body = ChatPolicyDto, description = "Set; followers receive it as a `policy` event."),
        (status = 400, body = ErrorBody, description = "`INVALID_SLOW_MODE` or `INVALID_REQUEST`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_BROADCAST_OWNER` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
    ),
)]
pub(crate) async fn set_policy(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path(id): Path<String>,
    body: Result<Json<ChatPolicyDto>, JsonRejection>,
) -> Result<Json<ChatPolicyDto>, ApiError> {
    let broadcast = broadcast_id(&id)?;
    let Json(body) = body.map_err(|rejection| ApiError::rejected_body(rejection.status()))?;
    let policy = state
        .comments
        .set_policy
        .run(actor, broadcast, body.slow_mode_seconds)
        .await
        .map_err(api_error)?;
    Ok(Json(policy.into()))
}
