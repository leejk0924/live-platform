use std::time::SystemTime;

use async_trait::async_trait;
use domain::{
    auth::{UserId, Username},
    broadcast::BroadcastId,
    donation::{DonationAmount, DonationId, PaymentKey},
};

use super::PortError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDonation {
    pub id: DonationId,
    pub broadcast_id: BroadcastId,
    pub donor_id: UserId,
    pub amount: DonationAmount,
}

/// The payment that settled an order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payment {
    pub key: PaymentKey,
    pub approved_at: SystemTime,
}

/// A stored order, pending or paid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DonationOrder {
    pub id: DonationId,
    pub broadcast_id: BroadcastId,
    pub donor_id: UserId,
    pub donor_name: Username,
    pub amount: DonationAmount,
    /// `None` while pending.
    pub paid: Option<Payment>,
}

impl DonationOrder {
    /// The donation as viewers see it, once paid.
    pub fn view(&self) -> Option<DonationView> {
        self.paid.as_ref().map(|paid| DonationView {
            id: self.id,
            broadcast_id: self.broadcast_id,
            donor_name: self.donor_name.clone(),
            amount: self.amount,
            approved_at: paid.approved_at,
        })
    }
}

/// A credited donation as viewers see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DonationView {
    pub id: DonationId,
    pub broadcast_id: BroadcastId,
    pub donor_name: Username,
    pub amount: DonationAmount,
    pub approved_at: SystemTime,
}

/// What a broadcast's credited donations add up to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DonationSummary {
    pub total: i64,
    pub count: i64,
}

/// A donation just credited, with its broadcast's summary right after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DonationCredit {
    pub donation: DonationView,
    pub summary: DonationSummary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Settlement {
    Credited(DonationCredit),
    /// The order is missing or no longer pending; nothing changed.
    NotPending,
    /// The payment already paid another order; nothing changed.
    PaymentKeyInUse,
}

#[async_trait]
pub trait DonationRepository: Send + Sync {
    /// Stores a pending order if its broadcast exists, is not deleted,
    /// [`domain::donation::accepts_donations`], and is not the donor's own,
    /// all checked by the statement that inserts. Whether it was stored.
    async fn create(&self, new: &NewDonation) -> Result<bool, PortError>;

    async fn order(&self, id: DonationId) -> Result<Option<DonationOrder>, PortError>;

    /// Marks a pending order paid by `payment_key`, adds it to its
    /// broadcast's summary and announces the credit to the broadcast's
    /// followers as a priority
    /// [`EventKind::Donation`](super::event_bus::EventKind::Donation), all
    /// atomically: the credit is stored only if it is announced. Of any
    /// number of concurrent calls for one order, at most one credits it.
    async fn settle(
        &self,
        id: DonationId,
        payment_key: &PaymentKey,
        approved_at: SystemTime,
    ) -> Result<Settlement, PortError>;
}

/// The read side, always within one broadcast and only credited donations.
#[async_trait]
pub trait DonationQuery: Send + Sync {
    /// Newest order first, only ids below `before` when given, at most
    /// `limit`.
    async fn list(
        &self,
        broadcast: BroadcastId,
        before: Option<DonationId>,
        limit: u32,
    ) -> Result<Vec<DonationView>, PortError>;

    /// Zeros before the first donation.
    async fn summary(&self, broadcast: BroadcastId) -> Result<DonationSummary, PortError>;
}
