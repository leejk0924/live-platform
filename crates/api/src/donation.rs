//! `/api/broadcasts/{id}/donations`, `/api/donations/confirm` and the Toss
//! Payments webhook: translates HTTP to the donation use cases. The donor is
//! always the session user.

use std::sync::Arc;

use application::{
    donation::{
        ConfirmDonation, CreateDonation, DonationPage, Error, ListDonations, RecheckOutcome,
        RecheckPayment,
    },
    ports::{
        broadcast::BroadcastQuery,
        donation::{DonationQuery, DonationRepository, DonationSummary, DonationView},
        id::IdGenerator,
        payment::PaymentGateway,
    },
};
use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use domain::{
    donation::{DonationError, DonationId},
    id::SnowflakeId,
    secret::Secret,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};

use crate::{
    AppState,
    audit::{AuditEvent, audit},
    auth::AuthUser,
    broadcast::broadcast_id,
    comment::rfc3339,
    error::{ApiError, ErrorBody},
};

/// What the buyer sees as the item in Toss's payment window.
pub const ORDER_NAME: &str = "live-platform 후원";

/// The only webhook event that can mean a payment was made.
const PAYMENT_STATUS_CHANGED: &str = "PAYMENT_STATUS_CHANGED";

pub struct DonationPorts {
    pub repository: Arc<dyn DonationRepository>,
    pub query: Arc<dyn DonationQuery>,
    /// `None` turns ordering and paying off; the list still works.
    pub payments: Option<PaymentPorts>,
}

pub struct PaymentPorts {
    pub gateway: Arc<dyn PaymentGateway>,
    /// Handed to the browser to open Toss's payment window.
    pub client_key: Secret<String>,
}

pub struct DonationUseCases {
    pub list: ListDonations,
    pub payments: Option<Payments>,
}

pub struct Payments {
    pub create: CreateDonation,
    pub confirm: ConfirmDonation,
    pub recheck: RecheckPayment,
    client_key: Secret<String>,
}

impl DonationUseCases {
    pub fn new(
        ports: DonationPorts,
        ids: Arc<dyn IdGenerator>,
        broadcasts: Arc<dyn BroadcastQuery>,
    ) -> Self {
        let DonationPorts {
            repository,
            query,
            payments,
        } = ports;
        let payments = payments.map(
            |PaymentPorts {
                 gateway,
                 client_key,
             }| Payments {
                create: CreateDonation {
                    ids,
                    broadcasts: broadcasts.clone(),
                    donations: repository.clone(),
                },
                confirm: ConfirmDonation {
                    donations: repository.clone(),
                    gateway: gateway.clone(),
                },
                recheck: RecheckPayment {
                    donations: repository,
                    gateway,
                },
                client_key,
            },
        );
        Self {
            list: ListDonations { broadcasts, query },
            payments,
        }
    }
}

/// Behind the CSRF guard like the rest of `/api`.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/broadcasts/{id}/donations", get(list).post(create))
        .route("/donations/confirm", post(confirm))
}

/// The webhook Toss's servers call. They send no `Origin`, so it stays out
/// of the CSRF guard; nothing in its body is trusted.
pub fn webhook_routes() -> Router<AppState> {
    Router::new().route("/api/payments/toss/webhook", post(webhook))
}

fn payments(state: &AppState) -> Result<&Payments, ApiError> {
    state
        .donations
        .payments
        .as_ref()
        .ok_or_else(ApiError::payments_disabled)
}

/// Never carries the provider's detail: it is in the audit log.
pub(crate) fn api_error(err: Error) -> ApiError {
    match err {
        Error::Rejected(err) => err.into(),
        Error::AmountMismatch { .. } => DonationError::AmountMismatch.into(),
        Error::PaymentRejected { .. } => DonationError::PaymentRejected.into(),
        Error::GatewayUnavailable(reason) => {
            tracing::warn!(%reason, "payment gateway unavailable");
            ApiError::payment_gateway_unavailable()
        }
        Error::Internal(err) => ApiError::internal(err),
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[schema(as = Donation)]
pub struct DonationDto {
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007303372210176")]
    id: SnowflakeId,
    #[serde(with = "crate::json::snowflake_id")]
    #[schema(value_type = String, example = "112007302751453184")]
    broadcast_id: SnowflakeId,
    /// Plain text: escape it when rendering.
    donor_name: String,
    /// Won.
    #[schema(minimum = 1000, maximum = 1000000)]
    amount: i64,
    #[schema(format = DateTime)]
    approved_at: String,
}

impl From<DonationView> for DonationDto {
    fn from(view: DonationView) -> Self {
        Self {
            id: view.id,
            broadcast_id: view.broadcast_id,
            donor_name: view.donor_name.as_str().to_owned(),
            amount: view.amount.get(),
            approved_at: rfc3339(view.approved_at),
        }
    }
}

/// A broadcast's credited donations, added up.
#[derive(Serialize, ToSchema)]
pub struct DonationTotals {
    /// Won.
    total: i64,
    count: i64,
}

impl From<DonationSummary> for DonationTotals {
    fn from(summary: DonationSummary) -> Self {
        Self {
            total: summary.total,
            count: summary.count,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DonationPageDto {
    /// Newest order first.
    items: Vec<DonationDto>,
    /// Won, over every credited donation of the broadcast.
    total: i64,
    count: i64,
    /// Pass as `before` for the next page; `null` on the last page.
    #[schema(required = true)]
    next_before: Option<String>,
}

impl From<DonationPage> for DonationPageDto {
    fn from(page: DonationPage) -> Self {
        Self {
            items: page.items.into_iter().map(DonationDto::from).collect(),
            total: page.summary.total,
            count: page.summary.count,
            next_before: page.next_before.map(|id| id.to_string()),
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct DonationInput {
    /// Won, 1000 to 1000000.
    amount: i64,
}

/// What the browser passes to Toss's payment window.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DonationOrderDto {
    /// Toss's `orderId`.
    #[schema(example = "112007303372210176")]
    order_id: String,
    amount: i64,
    /// Toss's `orderName`.
    order_name: &'static str,
    /// The Toss client key to open the payment window with.
    client_key: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmInput {
    /// As Toss handed it to the success URL.
    payment_key: String,
    order_id: String,
    /// As Toss handed it to the success URL; must equal the order's.
    amount: i64,
}

#[derive(Serialize, ToSchema)]
pub struct ConfirmedDto {
    donation: DonationDto,
    /// `false` when an earlier request had already credited this payment.
    credited: bool,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListParams {
    /// Only donations older than this id: the previous page's `nextBefore`.
    before: Option<String>,
    /// Page size, 1 to 50. Defaults to 20.
    #[param(minimum = 1, maximum = 50)]
    size: Option<u32>,
}

#[utoipa::path(
    get,
    path = "/api/broadcasts/{id}/donations",
    operation_id = "listDonations",
    tag = "donations",
    summary = "List a broadcast's donations, newest first, with their total",
    params(("id" = String, Path, description = "Broadcast id"), ListParams),
    responses(
        (status = 200, body = DonationPageDto, description = "Credited donations only."),
        (status = 400, body = ErrorBody, description = "`INVALID_QUERY` or `INVALID_PAGE_SIZE`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
    ),
)]
pub(crate) async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
    params: Result<Query<ListParams>, QueryRejection>,
) -> Result<Json<DonationPageDto>, ApiError> {
    let broadcast = broadcast_id(&id)?;
    let Query(params) = params.map_err(|_| ApiError::invalid_query())?;
    let before = params
        .before
        .map(|raw| raw.parse::<DonationId>())
        .transpose()
        .map_err(|_| ApiError::invalid_query())?;
    let page = state
        .donations
        .list
        .run(broadcast, before, params.size)
        .await
        .map_err(api_error)?;
    Ok(Json(page.into()))
}

#[utoipa::path(
    post,
    path = "/api/broadcasts/{id}/donations",
    operation_id = "createDonation",
    tag = "donations",
    summary = "Order a donation to a broadcast on air, to pay with Toss",
    description = "Open Toss's payment window with the returned `clientKey`, `orderId`, \
        `orderName` and `amount`, then send what Toss hands the success URL to \
        `POST /api/donations/confirm`. Nothing is credited before that.",
    params(("id" = String, Path, description = "Broadcast id")),
    request_body = DonationInput,
    security(("session" = [])),
    responses(
        (status = 201, body = DonationOrderDto, description = "A pending order."),
        (status = 400, body = ErrorBody, description = "`INVALID_AMOUNT` or `INVALID_REQUEST`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`SELF_DONATION` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`BROADCAST_NOT_FOUND`"),
        (status = 409, body = ErrorBody, description = "`DONATIONS_CLOSED`: the broadcast is not on air"),
        (status = 503, body = ErrorBody, description = "`PAYMENTS_DISABLED`"),
    ),
)]
pub(crate) async fn create(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    Path(id): Path<String>,
    body: Result<Json<DonationInput>, JsonRejection>,
) -> Result<Response, ApiError> {
    let payments = payments(&state)?;
    let broadcast = broadcast_id(&id)?;
    let Json(body) = body.map_err(|rejection| ApiError::rejected_body(rejection.status()))?;
    let order = payments
        .create
        .run(actor, broadcast, body.amount)
        .await
        .map_err(api_error)?;
    audit!(
        info,
        AuditEvent::DonationOrdered,
        donation = %order.id,
        %broadcast,
        amount = order.amount.get(),
        "donation ordered"
    );
    let order = DonationOrderDto {
        order_id: order.id.to_string(),
        amount: order.amount.get(),
        order_name: ORDER_NAME,
        client_key: payments.client_key.expose().clone(),
    };
    Ok((StatusCode::CREATED, Json(order)).into_response())
}

#[utoipa::path(
    post,
    path = "/api/donations/confirm",
    operation_id = "confirmDonation",
    tag = "donations",
    summary = "Pay for one's own donation order with the payment Toss authorized",
    description = "Safe to repeat: the same payment again answers `credited: false` and is \
        neither charged nor credited twice.",
    request_body = ConfirmInput,
    security(("session" = [])),
    responses(
        (status = 200, body = ConfirmedDto, description = "Paid; followers of the broadcast receive it as a `donation` event."),
        (status = 400, body = ErrorBody, description = "`AMOUNT_MISMATCH`, `PAYMENT_REJECTED`, `INVALID_PAYMENT_KEY` or `INVALID_REQUEST`"),
        (status = 401, body = ErrorBody, description = "`UNAUTHENTICATED`"),
        (status = 403, body = ErrorBody, description = "`NOT_DONOR` or `CSRF_REJECTED`"),
        (status = 404, body = ErrorBody, description = "`ORDER_NOT_FOUND`"),
        (status = 409, body = ErrorBody, description = "`ALREADY_PAID`: paid by another payment, or the payment paid another order"),
        (status = 503, body = ErrorBody, description = "`PAYMENT_GATEWAY_UNAVAILABLE`: retry with the same body, or `PAYMENTS_DISABLED`"),
    ),
)]
pub(crate) async fn confirm(
    State(state): State<AppState>,
    AuthUser(actor): AuthUser,
    body: Result<Json<ConfirmInput>, JsonRejection>,
) -> Result<Json<ConfirmedDto>, ApiError> {
    let payments = payments(&state)?;
    let Json(body) = body.map_err(|rejection| ApiError::rejected_body(rejection.status()))?;
    let result = payments
        .confirm
        .run(actor, &body.order_id, &body.payment_key, body.amount)
        .await;
    match result {
        Ok(confirmed) => {
            audit!(
                info,
                AuditEvent::PaymentConfirmed,
                donation = %confirmed.donation.id,
                amount = confirmed.donation.amount.get(),
                credited = confirmed.credited,
                "payment confirmed"
            );
            Ok(Json(ConfirmedDto {
                donation: confirmed.donation.into(),
                credited: confirmed.credited,
            }))
        }
        Err(err) => {
            audit_refused_confirm(&body.order_id, &err);
            Err(api_error(err))
        }
    }
}

/// The order id as the client sent it is logged only when it is one.
fn audit_refused_confirm(order_id: &str, err: &Error) {
    let donation = order_id
        .parse::<DonationId>()
        .map_or_else(|_| "<invalid>".to_owned(), |id| id.to_string());
    match err {
        Error::AmountMismatch { expected, got, at } => audit!(
            warn,
            AuditEvent::PaymentAmountMismatch,
            donation,
            expected,
            got,
            at = at.as_str(),
            "payment amount differs from the order"
        ),
        Error::PaymentRejected { code } => audit!(
            warn,
            AuditEvent::PaymentRejected,
            donation,
            gateway_code = code.as_str(),
            "payment refused"
        ),
        Error::GatewayUnavailable(_) => audit!(
            warn,
            AuditEvent::PaymentRejected,
            donation,
            gateway_code = "gateway_unavailable",
            "payment not confirmed: no answer from the gateway"
        ),
        Error::Rejected(_) | Error::Internal(_) => {}
    }
}

/// Toss calls this when a payment's status changes, and again until it
/// answers 200. Only `data.paymentKey` is read; the payment itself is
/// looked up at Toss.
pub(crate) async fn webhook(
    State(state): State<AppState>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<StatusCode, ApiError> {
    let payments = payments(&state)?;
    let Json(body) = body.map_err(|rejection| ApiError::rejected_body(rejection.status()))?;
    let key = match (
        body["eventType"].as_str(),
        body["data"]["paymentKey"].as_str(),
    ) {
        (Some(PAYMENT_STATUS_CHANGED), Some(key)) => key,
        _ => return Ok(StatusCode::OK),
    };
    match payments.recheck.run(key).await {
        Ok(outcome) => {
            audit_recheck(&outcome);
            Ok(StatusCode::OK)
        }
        Err(Error::Internal(err)) => Err(ApiError::internal_retry_later(err)),
        Err(err) => Err(api_error(err)),
    }
}

fn audit_recheck(outcome: &RecheckOutcome) {
    let name = outcome.as_str();
    let event = AuditEvent::PaymentWebhookChecked;
    match outcome {
        RecheckOutcome::UnknownPayment | RecheckOutcome::UnknownOrder => audit!(
            info,
            event,
            outcome = name,
            "payment notification for no order of ours"
        ),
        RecheckOutcome::NotDone { donation, status } => audit!(
            info,
            event,
            outcome = name,
            %donation,
            toss_status = status.as_str(),
            "payment notification for an unpaid payment"
        ),
        RecheckOutcome::Credited(donation) | RecheckOutcome::AlreadyDone(donation) => audit!(
            info,
            event,
            outcome = name,
            %donation,
            "payment notification checked"
        ),
        RecheckOutcome::AmountMismatch {
            donation,
            expected,
            got,
        } => audit!(
            warn,
            event,
            outcome = name,
            %donation,
            expected,
            got,
            "paid amount differs from the order; not credited"
        ),
        RecheckOutcome::Conflict(donation) => audit!(
            warn,
            event,
            outcome = name,
            %donation,
            "payment and order are each paired with another; not credited"
        ),
    }
}

#[cfg(test)]
mod tests {
    use application::ports::payment::{GatewayError, GatewayPayment, PaymentStatus};
    use axum::{
        body::{Body, to_bytes},
        http::{Request, header},
    };
    use infrastructure::payment::FakePaymentGateway;
    use serde_json::json;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        app,
        test_support::{payments_state, test_state},
    };

    /// As Toss's servers send it: no `Origin`, no `X-Requested-With`, no
    /// session.
    fn from_toss(body: &Value) -> Request<Body> {
        Request::post("/api/payments/toss/webhook")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn status_changed(key: &str) -> Value {
        json!({
            "eventType": "PAYMENT_STATUS_CHANGED",
            "createdAt": "2024-02-13T12:18:14.000000",
            "data": {"paymentKey": key, "orderId": "1", "status": "DONE", "totalAmount": 1}
        })
    }

    async fn send(state: AppState, request: Request<Body>) -> (StatusCode, Value) {
        let response = app(state).oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (status, body)
    }

    #[tokio::test]
    async fn the_webhook_takes_tosss_requests_and_looks_the_payment_up() {
        let gateway = Arc::new(FakePaymentGateway::default());
        let (status, _) = send(
            payments_state(gateway.clone()),
            from_toss(&status_changed("tgen_1")),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "an unknown payment is answered too");
        let looked_up: Vec<_> = gateway
            .lookups()
            .iter()
            .map(|k| k.as_str().to_owned())
            .collect();
        assert_eq!(looked_up, ["tgen_1"]);
    }

    #[tokio::test]
    async fn the_webhook_ignores_other_events_and_bodies_without_a_payment_key() {
        let gateway = Arc::new(FakePaymentGateway::default());
        for body in [
            json!({"eventType": "DEPOSIT_CALLBACK", "data": {"paymentKey": "tgen_1"}}),
            json!({"eventType": "PAYMENT_STATUS_CHANGED", "data": {}}),
            json!({"eventType": "PAYMENT_STATUS_CHANGED", "data": {"paymentKey": 7}}),
            json!({"paymentKey": "tgen_1"}),
            json!([]),
        ] {
            let (status, _) = send(payments_state(gateway.clone()), from_toss(&body)).await;
            assert_eq!(status, StatusCode::OK, "{body}");
        }
        assert!(gateway.lookups().is_empty());
    }

    #[tokio::test]
    async fn the_webhook_asks_for_a_retry_while_toss_does_not_answer() {
        let gateway = Arc::new(FakePaymentGateway::default());
        gateway.answer_lookup(Err(GatewayError::Unavailable("timeout".into())));
        let (status, body) = send(
            payments_state(gateway.clone()),
            from_toss(&status_changed("tgen_1")),
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PAYMENT_GATEWAY_UNAVAILABLE");

        gateway.answer_lookup(Ok(Some(GatewayPayment {
            payment_key: "tgen_1".into(),
            order_id: "112007303372210176".into(),
            status: PaymentStatus::Done,
            total_amount: 1_000,
            approved_at: None,
        })));
        let (status, _) = send(
            payments_state(gateway),
            from_toss(&status_changed("tgen_1")),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "a store that cannot be read is retried too"
        );
    }

    #[tokio::test]
    async fn without_toss_keys_the_webhook_is_disabled() {
        let (status, body) = send(test_state(), from_toss(&status_changed("tgen_1"))).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "PAYMENTS_DISABLED");
    }
}
