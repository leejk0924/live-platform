//! Toss Payments' core API (`/v1/payments`), authenticated with the secret
//! key as HTTP Basic user name and an empty password.

use std::{fmt, time::Duration};

use application::ports::{
    PortError,
    payment::{GatewayError, GatewayPayment, PaymentConfirmation, PaymentGateway, PaymentStatus},
};
use async_trait::async_trait;
use domain::{donation::PaymentKey, secret::Secret};
use reqwest::{Response, StatusCode};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const TOSS_API_URL: &str = "https://api.tosspayments.com";

/// Under the API's 10 s request limit, so the handler still answers. A
/// confirm cut off here may have gone through; retrying it with the same
/// idempotency key returns that result instead of charging again.
const TIMEOUT: Duration = Duration::from_secs(8);

/// The code Toss answers a lookup of a payment it does not have with.
const NOT_FOUND_PAYMENT: &str = "NOT_FOUND_PAYMENT";

/// The code Toss answers a confirm with while an earlier one with the same
/// Idempotency-Key is still being processed.
const IDEMPOTENT_REQUEST_PROCESSING: &str = "IDEMPOTENT_REQUEST_PROCESSING";

pub struct TossPayments {
    http: reqwest::Client,
    base_url: String,
    secret_key: Secret<String>,
}

impl fmt::Debug for TossPayments {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TossPayments")
            .field("base_url", &self.base_url)
            .field("secret_key", &self.secret_key)
            .finish_non_exhaustive()
    }
}

impl TossPayments {
    /// `base_url` is [`TOSS_API_URL`] except in tests.
    pub fn new(base_url: String, secret_key: Secret<String>) -> Result<Self, PortError> {
        let http = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|err| PortError(format!("http client: {err}")))?;
        Ok(Self {
            http,
            base_url,
            secret_key,
        })
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Response, GatewayError> {
        request
            .basic_auth(self.secret_key.expose(), None::<&str>)
            .send()
            .await
            // A lookup's URL carries the payment key.
            .map_err(|err| GatewayError::Unavailable(format!("toss: {}", err.without_url())))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfirmBody<'a> {
    payment_key: &'a str,
    order_id: String,
    amount: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TossPayment {
    payment_key: String,
    order_id: String,
    status: String,
    total_amount: i64,
    approved_at: Option<String>,
}

#[derive(Deserialize)]
struct TossError {
    code: String,
}

enum Answer {
    Payment(GatewayPayment),
    Refused { status: StatusCode, code: String },
}

/// A payment from a 2xx answer; a refusal's code from a 4xx one with a JSON
/// error body, other than 401 and 403. Anything else may change on a retry.
async fn answer(response: Response) -> Result<Answer, GatewayError> {
    let status = response.status();
    let unavailable = |what: &str| GatewayError::Unavailable(format!("toss: {status}: {what}"));
    let body = response
        .bytes()
        .await
        .map_err(|err| unavailable(&err.without_url().to_string()))?;
    if status.is_success() {
        let payment: TossPayment =
            serde_json::from_slice(&body).map_err(|_| unavailable("unreadable payment"))?;
        let approved_at = payment
            .approved_at
            .map(|raw| OffsetDateTime::parse(&raw, &Rfc3339))
            .transpose()
            .map_err(|_| unavailable("unreadable approvedAt"))?;
        return Ok(Answer::Payment(GatewayPayment {
            payment_key: payment.payment_key,
            order_id: payment.order_id,
            status: PaymentStatus::parse(&payment.status),
            total_amount: payment.total_amount,
            approved_at: approved_at.map(Into::into),
        }));
    }
    // Toss refused our secret key, not the buyer's payment: a configuration
    // fault for the operator, which the buyer must not read as a card refusal.
    // Card refusals come as 403, so only 401 is the key.
    if status == StatusCode::UNAUTHORIZED {
        return Err(unavailable("secret key refused"));
    }
    let retryable = matches!(
        status,
        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS
    );
    match serde_json::from_slice::<TossError>(&body) {
        // An earlier confirm with the same Idempotency-Key is still running;
        // asking again later returns its answer.
        Ok(error) if error.code == IDEMPOTENT_REQUEST_PROCESSING => {
            Err(unavailable("an earlier confirm of this order is in flight"))
        }
        Ok(error) if status.is_client_error() && !retryable => Ok(Answer::Refused {
            status,
            code: error.code,
        }),
        _ => Err(unavailable("no usable answer")),
    }
}

#[async_trait]
impl PaymentGateway for TossPayments {
    async fn confirm(&self, request: &PaymentConfirmation) -> Result<GatewayPayment, GatewayError> {
        let body = ConfirmBody {
            payment_key: request.payment_key.as_str(),
            order_id: request.order_id.to_string(),
            amount: request.amount.get(),
        };
        let response = self
            .send(
                self.http
                    .post(format!("{}/v1/payments/confirm", self.base_url))
                    .header("Idempotency-Key", &request.idempotency_key)
                    .json(&body),
            )
            .await?;
        match answer(response).await? {
            Answer::Payment(payment) => Ok(payment),
            Answer::Refused { code, .. } => Err(GatewayError::Rejected { code }),
        }
    }

    async fn lookup(
        &self,
        payment_key: &PaymentKey,
    ) -> Result<Option<GatewayPayment>, GatewayError> {
        // PaymentKey admits only URL-safe characters, so the key cannot
        // leave its path segment.
        let url = format!("{}/v1/payments/{}", self.base_url, payment_key.as_str());
        let response = self.send(self.http.get(url)).await?;
        match answer(response).await? {
            Answer::Payment(payment) => Ok(Some(payment)),
            Answer::Refused { status, code }
                if status == StatusCode::NOT_FOUND && code == NOT_FOUND_PAYMENT =>
            {
                Ok(None)
            }
            Answer::Refused { code, .. } => Err(GatewayError::Rejected { code }),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use base64ct::{Base64, Encoding};
    use domain::{
        donation::{DonationAmount, DonationId},
        id::SnowflakeId,
    };
    use serde_json::{Value, json};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::oneshot,
    };

    use super::*;

    const SECRET: &str = "test_gsk_fake_secret_for_unit_tests";
    const KEY: &str = "tgen_20240213121812kDfF5";
    const ORDER: i64 = 112_007_303_372_210_176;

    /// What the fake Toss received.
    #[derive(Debug)]
    struct Received {
        request_line: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Received {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
        }

        fn json(&self) -> Value {
            serde_json::from_slice(&self.body).unwrap()
        }
    }

    /// Accepts one HTTP/1.1 request, answers `status` with `body` and reports
    /// what it received. A bare socket, so the test sees exactly what went
    /// over the wire.
    async fn fake_toss(status: u16, body: Value) -> (String, oneshot::Receiver<Received>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = Vec::new();
            let head_end = loop {
                let mut chunk = [0u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "connection closed before the headers ended");
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let head = String::from_utf8(buffer[..head_end].to_vec()).unwrap();
            let mut lines = head.split("\r\n");
            let request_line = lines.next().unwrap().to_owned();
            let headers: Vec<(String, String)> = lines
                .filter_map(|line| line.split_once(": "))
                .map(|(n, v)| (n.to_owned(), v.to_owned()))
                .collect();
            let length: usize = headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                .map_or(0, |(_, v)| v.parse().unwrap());
            let mut received = buffer[head_end..].to_vec();
            while received.len() < length {
                let mut chunk = [0u8; 4096];
                let read = socket.read(&mut chunk).await.unwrap();
                assert!(read > 0, "connection closed before the body ended");
                received.extend_from_slice(&chunk[..read]);
            }
            let body = body.to_string();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n\
                         content-length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let _ = tx.send(Received {
                request_line,
                headers,
                body: received,
            });
        });
        (base, rx)
    }

    fn toss(base: String) -> TossPayments {
        TossPayments::new(base, Secret::new(SECRET.into())).unwrap()
    }

    fn order() -> DonationId {
        SnowflakeId::try_from(ORDER).unwrap()
    }

    fn confirmation() -> PaymentConfirmation {
        PaymentConfirmation {
            payment_key: PaymentKey::parse(KEY).unwrap(),
            order_id: order(),
            amount: DonationAmount::parse(15_000).unwrap(),
            idempotency_key: format!("donation-confirm-{ORDER}"),
        }
    }

    /// Trimmed from the payment object in Toss's documentation.
    fn done_payment() -> Value {
        json!({
            "mId": "tosspayments",
            "paymentKey": KEY,
            "orderId": ORDER.to_string(),
            "orderName": "live-platform 후원",
            "status": "DONE",
            "totalAmount": 15000,
            "balanceAmount": 15000,
            "method": "카드",
            "requestedAt": "2024-02-13T12:17:57+09:00",
            "approvedAt": "2024-02-13T12:18:14+09:00",
            "card": {"number": "43301234****123*", "approveNo": "00000000"}
        })
    }

    #[tokio::test]
    async fn confirm_posts_the_order_with_basic_auth_and_the_idempotency_key() {
        let (base, received) = fake_toss(200, done_payment()).await;

        let payment = toss(base).confirm(&confirmation()).await.unwrap();

        let received = received.await.unwrap();
        assert_eq!(received.request_line, "POST /v1/payments/confirm HTTP/1.1");
        let basic = Base64::encode_string(format!("{SECRET}:").as_bytes());
        assert_eq!(
            received.header("authorization"),
            Some(format!("Basic {basic}").as_str())
        );
        assert_eq!(
            received.header("idempotency-key"),
            Some(format!("donation-confirm-{ORDER}").as_str())
        );
        assert_eq!(received.header("content-type"), Some("application/json"));
        assert_eq!(
            received.json(),
            json!({"paymentKey": KEY, "orderId": ORDER.to_string(), "amount": 15000})
        );
        // 2024-02-13T03:18:14Z
        let approved = UNIX_EPOCH + Duration::from_secs(1_707_794_294);
        assert_eq!(
            payment,
            GatewayPayment {
                payment_key: KEY.into(),
                order_id: ORDER.to_string(),
                status: PaymentStatus::Done,
                total_amount: 15_000,
                approved_at: Some(approved),
            }
        );
    }

    #[tokio::test]
    async fn a_refusal_is_rejected_with_tosss_code() {
        let (base, _) = fake_toss(
            400,
            json!({"code": "REJECT_CARD_PAYMENT", "message": "한도초과 혹은 잔액부족으로 결제에 실패했습니다."}),
        )
        .await;
        assert_eq!(
            toss(base).confirm(&confirmation()).await,
            Err(GatewayError::Rejected {
                code: "REJECT_CARD_PAYMENT".into()
            })
        );
    }

    #[tokio::test]
    async fn server_errors_and_unreadable_answers_are_unavailable() {
        for (status, body) in [
            (
                500,
                json!({"code": "FAILED_INTERNAL_SYSTEM_PROCESSING", "message": "x"}),
            ),
            (429, json!({"code": "TOO_MANY_REQUESTS", "message": "x"})),
            (400, json!("not an error object")),
            (200, json!({"status": "DONE"})),
            (200, {
                let mut payment = done_payment();
                payment["approvedAt"] = json!("yesterday");
                payment
            }),
        ] {
            let (base, _) = fake_toss(status, body.clone()).await;
            let result = toss(base).confirm(&confirmation()).await;
            assert!(
                matches!(result, Err(GatewayError::Unavailable(_))),
                "{status} {body}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_key_toss_does_not_accept_is_unavailable_not_a_refusal() {
        for (status, code) in [(401, "UNAUTHORIZED_KEY")] {
            let body = json!({"code": code, "message": "x"});
            let (base, _) = fake_toss(status, body.clone()).await;
            let confirmed = toss(base).confirm(&confirmation()).await;
            let (base, _) = fake_toss(status, body).await;
            let looked_up = toss(base).lookup(&PaymentKey::parse(KEY).unwrap()).await;
            for err in [confirmed.unwrap_err(), looked_up.unwrap_err()] {
                let GatewayError::Unavailable(reason) = &err else {
                    panic!("{status}: {err:?}");
                };
                assert!(reason.contains(&status.to_string()), "{reason}");
                assert!(!reason.contains(SECRET), "{reason}");
            }
        }
    }

    /// Toss refuses cards with 403, so a 403 is the buyer's refusal, not
    /// a key fault.
    #[tokio::test]
    async fn a_card_refusal_with_403_is_rejected() {
        let (base, _) =
            fake_toss(403, json!({"code": "REJECT_CARD_PAYMENT", "message": "x"})).await;
        assert_eq!(
            toss(base).confirm(&confirmation()).await,
            Err(GatewayError::Rejected {
                code: "REJECT_CARD_PAYMENT".into()
            })
        );
    }

    /// Measured against the Toss sandbox: concurrent confirms with one
    /// Idempotency-Key get 409 for all but the first, and the same request
    /// later gets the first one's answer, so it is worth retrying.
    #[tokio::test]
    async fn a_confirm_still_in_flight_is_unavailable_not_a_refusal() {
        let (base, _) = fake_toss(
            409,
            json!({"code": "IDEMPOTENT_REQUEST_PROCESSING", "message": "x"}),
        )
        .await;
        let err = toss(base).confirm(&confirmation()).await.unwrap_err();
        assert!(matches!(err, GatewayError::Unavailable(_)), "{err:?}");
    }

    #[tokio::test]
    async fn lookup_gets_the_payment_by_its_key() {
        let mut waiting = done_payment();
        waiting["status"] = json!("WAITING_FOR_DEPOSIT");
        waiting["approvedAt"] = Value::Null;
        let (base, received) = fake_toss(200, waiting).await;

        let payment = toss(base)
            .lookup(&PaymentKey::parse(KEY).unwrap())
            .await
            .unwrap()
            .unwrap();

        let received = received.await.unwrap();
        assert_eq!(
            received.request_line,
            format!("GET /v1/payments/{KEY} HTTP/1.1")
        );
        assert!(received.header("authorization").is_some());
        assert_eq!(payment.status, PaymentStatus::WaitingForDeposit);
        assert_eq!(payment.approved_at, None);
    }

    #[tokio::test]
    async fn a_payment_toss_does_not_have_is_none() {
        let (base, _) = fake_toss(
            404,
            json!({"code": "NOT_FOUND_PAYMENT", "message": "존재하지 않는 결제 정보 입니다."}),
        )
        .await;
        assert_eq!(
            toss(base).lookup(&PaymentKey::parse(KEY).unwrap()).await,
            Ok(None)
        );

        let (base, _) = fake_toss(404, json!({"code": "NOT_FOUND_MERCHANT", "message": "x"})).await;
        assert_eq!(
            toss(base).lookup(&PaymentKey::parse(KEY).unwrap()).await,
            Err(GatewayError::Rejected {
                code: "NOT_FOUND_MERCHANT".into()
            }),
            "only a missing payment is None"
        );
    }

    #[tokio::test]
    async fn an_unreachable_toss_is_unavailable_without_the_key_or_the_secret() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);

        let err = toss(base)
            .lookup(&PaymentKey::parse(KEY).unwrap())
            .await
            .unwrap_err();

        assert!(matches!(err, GatewayError::Unavailable(_)), "{err:?}");
        let message = err.to_string();
        assert!(!message.contains(KEY), "{message}");
        assert!(!message.contains(SECRET), "{message}");
    }

    #[test]
    fn debug_never_shows_the_secret_key() {
        let toss = toss(TOSS_API_URL.into());
        let debug = format!("{toss:?}");
        assert!(debug.contains(TOSS_API_URL), "{debug}");
        assert!(!debug.contains(SECRET), "{debug}");
    }
}
