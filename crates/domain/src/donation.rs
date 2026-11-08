//! Donations to a broadcast: what a viewer may give, and what a payment must
//! look like before it is credited.

use thiserror::Error;

use crate::{
    broadcast::BroadcastStatus,
    error::{DomainError, ErrorKind},
    id::SnowflakeId,
};

/// Also the order id the payment provider sees, as a decimal string.
pub type DonationId = SnowflakeId;

/// Won, inclusive.
pub const DONATION_MIN: i64 = 1_000;
pub const DONATION_MAX: i64 = 1_000_000;

pub const PAYMENT_KEY_MAX_CHARS: usize = 200;

/// Donations per list page when the client names no size, and the most it
/// may ask for.
pub const PAGE_SIZE_DEFAULT: u32 = 20;
pub const PAGE_SIZE_MAX: u32 = 50;

/// Whether a broadcast in `status` takes new donations: only while it is on
/// air, a short interruption included.
pub fn accepts_donations(status: BroadcastStatus) -> bool {
    status.is_active()
}

/// Won, [`DONATION_MIN`] to [`DONATION_MAX`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DonationAmount(i64);

impl DonationAmount {
    pub fn parse(won: i64) -> Result<Self, DonationError> {
        if (DONATION_MIN..=DONATION_MAX).contains(&won) {
            Ok(Self(won))
        } else {
            Err(DonationError::InvalidAmount)
        }
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

/// The payment provider's id of one payment. It goes into the provider's
/// URL path, so only `[A-Za-z0-9_-]` is accepted: nothing that could
/// change the path or the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentKey(String);

impl PaymentKey {
    pub fn parse(raw: &str) -> Result<Self, DonationError> {
        let valid_len = (1..=PAYMENT_KEY_MAX_CHARS).contains(&raw.len());
        let valid_chars = raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if valid_len && valid_chars {
            Ok(Self(raw.to_owned()))
        } else {
            Err(DonationError::InvalidPaymentKey)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An order is `Pending` until its payment is confirmed, then `Done` for
/// good. A payment that fails leaves it pending; nothing is ever credited
/// twice because only a pending order can become done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DonationStatus {
    Pending,
    Done,
}

impl DonationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Done => "DONE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DonationError {
    #[error("amount must be between {DONATION_MIN} and {DONATION_MAX} won")]
    InvalidAmount,
    #[error("page size must be between 1 and {PAGE_SIZE_MAX}")]
    InvalidPageSize,
    #[error("payment key is invalid")]
    InvalidPaymentKey,
    #[error("you cannot donate to your own broadcast")]
    SelfDonation,
    #[error("broadcast not found")]
    BroadcastNotFound,
    #[error("the broadcast is not on air and takes no donations")]
    Closed,
    #[error("donation order not found")]
    OrderNotFound,
    #[error("only the donor can confirm this order")]
    NotDonor,
    #[error("the amount does not match the order")]
    AmountMismatch,
    #[error("the order is already paid")]
    AlreadyPaid,
    #[error("the payment was not approved")]
    PaymentRejected,
}

impl DomainError for DonationError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidAmount
            | Self::InvalidPageSize
            | Self::InvalidPaymentKey
            | Self::AmountMismatch
            | Self::PaymentRejected => ErrorKind::InvalidInput,
            Self::SelfDonation | Self::NotDonor => ErrorKind::Forbidden,
            Self::BroadcastNotFound | Self::OrderNotFound => ErrorKind::NotFound,
            Self::Closed | Self::AlreadyPaid => ErrorKind::Conflict,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::InvalidAmount => "INVALID_AMOUNT",
            Self::InvalidPageSize => "INVALID_PAGE_SIZE",
            Self::InvalidPaymentKey => "INVALID_PAYMENT_KEY",
            Self::SelfDonation => "SELF_DONATION",
            Self::BroadcastNotFound => "BROADCAST_NOT_FOUND",
            Self::Closed => "DONATIONS_CLOSED",
            Self::OrderNotFound => "ORDER_NOT_FOUND",
            Self::NotDonor => "NOT_DONOR",
            Self::AmountMismatch => "AMOUNT_MISMATCH",
            Self::AlreadyPaid => "ALREADY_PAID",
            Self::PaymentRejected => "PAYMENT_REJECTED",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amount_is_1000_to_1000000_won() {
        for ok in [1_000, 1_000_000] {
            assert_eq!(DonationAmount::parse(ok).map(DonationAmount::get), Ok(ok));
        }
        for bad in [999, 1_000_001, 0, -1_000, i64::MAX] {
            assert_eq!(
                DonationAmount::parse(bad),
                Err(DonationError::InvalidAmount),
                "{bad}"
            );
        }
    }

    #[test]
    fn payment_key_is_1_to_200_url_safe_ascii_characters() {
        let longest = "a".repeat(200);
        for ok in ["tgen_20240213121812kDfF5", "A-b_9", longest.as_str()] {
            assert_eq!(PaymentKey::parse(ok).unwrap().as_str(), ok);
        }
        let too_long = "a".repeat(201);
        for bad in [
            "",
            too_long.as_str(),
            "a/b",
            "../cancel",
            "a?b=c",
            "a#b",
            "a%2Fb",
            "a b",
            "a.b",
            "키",
        ] {
            assert_eq!(
                PaymentKey::parse(bad),
                Err(DonationError::InvalidPaymentKey),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn only_broadcasts_on_air_take_donations() {
        let open: Vec<_> = BroadcastStatus::ALL
            .into_iter()
            .filter(|s| accepts_donations(*s))
            .collect();
        assert_eq!(open, [BroadcastStatus::Live, BroadcastStatus::Interrupted]);
    }

    #[test]
    fn errors_have_stable_codes_and_kinds() {
        for (err, kind, code) in [
            (
                DonationError::InvalidAmount,
                ErrorKind::InvalidInput,
                "INVALID_AMOUNT",
            ),
            (
                DonationError::SelfDonation,
                ErrorKind::Forbidden,
                "SELF_DONATION",
            ),
            (
                DonationError::BroadcastNotFound,
                ErrorKind::NotFound,
                "BROADCAST_NOT_FOUND",
            ),
            (
                DonationError::Closed,
                ErrorKind::Conflict,
                "DONATIONS_CLOSED",
            ),
            (
                DonationError::OrderNotFound,
                ErrorKind::NotFound,
                "ORDER_NOT_FOUND",
            ),
            (DonationError::NotDonor, ErrorKind::Forbidden, "NOT_DONOR"),
            (
                DonationError::AmountMismatch,
                ErrorKind::InvalidInput,
                "AMOUNT_MISMATCH",
            ),
            (
                DonationError::AlreadyPaid,
                ErrorKind::Conflict,
                "ALREADY_PAID",
            ),
            (
                DonationError::PaymentRejected,
                ErrorKind::InvalidInput,
                "PAYMENT_REJECTED",
            ),
        ] {
            assert_eq!((err.kind(), err.code()), (kind, code), "{err:?}");
        }
    }
}
