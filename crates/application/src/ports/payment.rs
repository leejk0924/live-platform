use std::time::SystemTime;

use async_trait::async_trait;
use domain::donation::{DonationAmount, DonationId, PaymentKey};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentConfirmation {
    pub payment_key: PaymentKey,
    pub order_id: DonationId,
    pub amount: DonationAmount,
    /// The same key for every attempt at one order, so a retry after a lost
    /// answer gets the first attempt's result instead of a second charge.
    pub idempotency_key: String,
}

/// Where a payment stands at the provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentStatus {
    Ready,
    InProgress,
    WaitingForDeposit,
    Done,
    Canceled,
    PartialCanceled,
    Aborted,
    Expired,
    /// A status this code does not know; never treated as paid.
    Other(String),
}

impl PaymentStatus {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "READY" => Self::Ready,
            "IN_PROGRESS" => Self::InProgress,
            "WAITING_FOR_DEPOSIT" => Self::WaitingForDeposit,
            "DONE" => Self::Done,
            "CANCELED" => Self::Canceled,
            "PARTIAL_CANCELED" => Self::PartialCanceled,
            "ABORTED" => Self::Aborted,
            "EXPIRED" => Self::Expired,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Ready => "READY",
            Self::InProgress => "IN_PROGRESS",
            Self::WaitingForDeposit => "WAITING_FOR_DEPOSIT",
            Self::Done => "DONE",
            Self::Canceled => "CANCELED",
            Self::PartialCanceled => "PARTIAL_CANCELED",
            Self::Aborted => "ABORTED",
            Self::Expired => "EXPIRED",
            Self::Other(raw) => raw,
        }
    }
}

/// A payment as the provider reports it. Untrusted: the use cases compare
/// every field with the stored order before crediting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayPayment {
    pub payment_key: String,
    pub order_id: String,
    pub status: PaymentStatus,
    pub total_amount: i64,
    pub approved_at: Option<SystemTime>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum GatewayError {
    /// The provider refused for good; `code` is its reason, for logs.
    #[error("payment refused: {code}")]
    Rejected { code: String },
    /// No answer to rely on (network, timeout, server error, unreadable
    /// body). Retrying may succeed. The message is for logs only.
    #[error("payment gateway unavailable: {0}")]
    Unavailable(String),
}

#[async_trait]
pub trait PaymentGateway: Send + Sync {
    /// Approves a payment the buyer authorized.
    async fn confirm(&self, request: &PaymentConfirmation) -> Result<GatewayPayment, GatewayError>;

    /// The payment as the provider holds it now; `None` when it has no such
    /// payment.
    async fn lookup(
        &self,
        payment_key: &PaymentKey,
    ) -> Result<Option<GatewayPayment>, GatewayError>;
}
