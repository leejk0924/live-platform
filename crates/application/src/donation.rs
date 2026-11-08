//! Ordering, paying for and listing donations. The donor is always the
//! authenticated actor, and nothing the client or the payment provider says
//! about an amount is believed until it matches the stored order.

use std::{sync::Arc, time::SystemTime};

use domain::{
    auth::UserId,
    broadcast::BroadcastId,
    donation::{
        DonationAmount, DonationError, DonationId, PAGE_SIZE_DEFAULT, PAGE_SIZE_MAX, PaymentKey,
        accepts_donations,
    },
};
use thiserror::Error;

use crate::ports::{
    PortError,
    broadcast::BroadcastQuery,
    donation::{
        DonationOrder, DonationQuery, DonationRepository, DonationSummary, DonationView,
        NewDonation, Settlement,
    },
    id::{IdGenerationError, IdGenerator},
    payment::{GatewayError, GatewayPayment, PaymentConfirmation, PaymentGateway, PaymentStatus},
};

/// Which side disagreed with the stored order's amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MismatchSource {
    Request,
    Gateway,
}

impl MismatchSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Gateway => "gateway",
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Rejected(#[from] DonationError),
    #[error("amount {got} does not match the order's {expected} ({})", at.as_str())]
    AmountMismatch {
        expected: i64,
        got: i64,
        at: MismatchSource,
    },
    /// The provider refused, or approved something other than this order.
    #[error("payment rejected: {code}")]
    PaymentRejected { code: String },
    #[error("payment gateway unavailable: {0}")]
    GatewayUnavailable(String),
    #[error(transparent)]
    Internal(#[from] PortError),
}

impl From<IdGenerationError> for Error {
    fn from(err: IdGenerationError) -> Self {
        Self::Internal(PortError(err.to_string()))
    }
}

impl From<GatewayError> for Error {
    fn from(err: GatewayError) -> Self {
        match err {
            GatewayError::Rejected { code } => Self::PaymentRejected { code },
            GatewayError::Unavailable(reason) => Self::GatewayUnavailable(reason),
        }
    }
}

/// What the provider is told on every attempt to confirm `order`.
pub fn idempotency_key(order: DonationId) -> String {
    format!("donation-confirm-{order}")
}

pub struct CreateDonation {
    pub ids: Arc<dyn IdGenerator>,
    pub broadcasts: Arc<dyn BroadcastQuery>,
    pub donations: Arc<dyn DonationRepository>,
}

impl CreateDonation {
    /// A pending order the donor then pays for.
    pub async fn run(
        &self,
        actor: UserId,
        broadcast: BroadcastId,
        amount: i64,
    ) -> Result<NewDonation, Error> {
        let amount = DonationAmount::parse(amount)?;
        let target = self
            .broadcasts
            .get(broadcast)
            .await?
            .ok_or(DonationError::BroadcastNotFound)?
            .broadcast;
        if target.owner_id == actor {
            return Err(DonationError::SelfDonation.into());
        }
        if !accepts_donations(target.status) {
            return Err(DonationError::Closed.into());
        }
        let new = NewDonation {
            id: self.ids.next_id()?,
            broadcast_id: broadcast,
            donor_id: actor,
            amount,
        };
        // The broadcast went off air or was deleted after the read above.
        if !self.donations.create(&new).await? {
            return Err(DonationError::Closed.into());
        }
        Ok(new)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirmed {
    pub donation: DonationView,
    /// `false` when an earlier request had already credited it.
    pub credited: bool,
}

pub struct ConfirmDonation {
    pub donations: Arc<dyn DonationRepository>,
    pub gateway: Arc<dyn PaymentGateway>,
}

impl ConfirmDonation {
    /// Pays for the actor's own order with the payment the provider handed
    /// the client. Safe to repeat: a paid order is never charged or
    /// credited again.
    pub async fn run(
        &self,
        actor: UserId,
        order_id: &str,
        payment_key: &str,
        amount: i64,
    ) -> Result<Confirmed, Error> {
        let id: DonationId = order_id.parse().map_err(|_| DonationError::OrderNotFound)?;
        let key = PaymentKey::parse(payment_key)?;
        let order = self.load(id).await?;
        if order.donor_id != actor {
            return Err(DonationError::NotDonor.into());
        }
        if amount != order.amount.get() {
            return Err(Error::AmountMismatch {
                expected: order.amount.get(),
                got: amount,
                at: MismatchSource::Request,
            });
        }
        if order.paid.is_some() {
            return already_paid(&order, &key);
        }
        let payment = self
            .gateway
            .confirm(&PaymentConfirmation {
                payment_key: key.clone(),
                order_id: id,
                amount: order.amount,
                idempotency_key: idempotency_key(id),
            })
            .await?;
        verify(&payment, &order, &key)?;
        let approved_at = payment.approved_at.unwrap_or_else(SystemTime::now);
        match self.donations.settle(id, &key, approved_at).await? {
            Settlement::Credited(credit) => Ok(Confirmed {
                donation: credit.donation,
                credited: true,
            }),
            // A concurrent request credited it first, or tried to with
            // another payment.
            Settlement::NotPending => already_paid(&self.load(id).await?, &key),
            Settlement::PaymentKeyInUse => Err(DonationError::AlreadyPaid.into()),
        }
    }

    async fn load(&self, id: DonationId) -> Result<DonationOrder, Error> {
        Ok(self
            .donations
            .order(id)
            .await?
            .ok_or(DonationError::OrderNotFound)?)
    }
}

/// A repeat of the request that paid `order` gets the same donation back;
/// any other payment is refused.
fn already_paid(order: &DonationOrder, key: &PaymentKey) -> Result<Confirmed, Error> {
    match (&order.paid, order.view()) {
        (Some(paid), Some(donation)) if paid.key == *key => Ok(Confirmed {
            donation,
            credited: false,
        }),
        _ => Err(DonationError::AlreadyPaid.into()),
    }
}

/// Whether the provider approved exactly this order, with this payment,
/// for its stored amount.
fn verify(payment: &GatewayPayment, order: &DonationOrder, key: &PaymentKey) -> Result<(), Error> {
    let rejected = |code: &str| {
        Err(Error::PaymentRejected {
            code: code.to_owned(),
        })
    };
    if payment.order_id != order.id.to_string() {
        return rejected("ORDER_MISMATCH");
    }
    if payment.payment_key != key.as_str() {
        return rejected("PAYMENT_KEY_MISMATCH");
    }
    if payment.total_amount != order.amount.get() {
        return Err(Error::AmountMismatch {
            expected: order.amount.get(),
            got: payment.total_amount,
            at: MismatchSource::Gateway,
        });
    }
    if payment.status != PaymentStatus::Done {
        return rejected("NOT_DONE");
    }
    Ok(())
}

/// What a payment status notification led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecheckOutcome {
    /// The provider has no such payment, or the key could not be one.
    UnknownPayment,
    /// The payment is for no order of ours.
    UnknownOrder,
    NotDone {
        donation: DonationId,
        status: PaymentStatus,
    },
    AmountMismatch {
        donation: DonationId,
        expected: i64,
        got: i64,
    },
    Credited(DonationId),
    AlreadyDone(DonationId),
    /// The order was paid by another payment, or this payment paid another
    /// order.
    Conflict(DonationId),
}

impl RecheckOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::UnknownPayment => "unknown_payment",
            Self::UnknownOrder => "unknown_order",
            Self::NotDone { .. } => "not_done",
            Self::AmountMismatch { .. } => "amount_mismatch",
            Self::Credited(_) => "credited",
            Self::AlreadyDone(_) => "already_done",
            Self::Conflict(_) => "conflict",
        }
    }

    pub fn donation(&self) -> Option<DonationId> {
        match self {
            Self::UnknownPayment | Self::UnknownOrder => None,
            Self::NotDone { donation, .. } | Self::AmountMismatch { donation, .. } => {
                Some(*donation)
            }
            Self::Credited(id) | Self::AlreadyDone(id) | Self::Conflict(id) => Some(*id),
        }
    }
}

pub struct RecheckPayment {
    pub donations: Arc<dyn DonationRepository>,
    pub gateway: Arc<dyn PaymentGateway>,
}

impl RecheckPayment {
    /// Credits the order a payment is for if the provider, asked directly,
    /// says it is paid in full. Only the key is taken from the notification;
    /// everything else comes from the provider's lookup and our order.
    /// Fails only when the answer may change on a retry.
    pub async fn run(&self, payment_key: &str) -> Result<RecheckOutcome, Error> {
        let Ok(key) = PaymentKey::parse(payment_key) else {
            return Ok(RecheckOutcome::UnknownPayment);
        };
        let Some(payment) = self.gateway.lookup(&key).await? else {
            return Ok(RecheckOutcome::UnknownPayment);
        };
        let Ok(id) = payment.order_id.parse::<DonationId>() else {
            return Ok(RecheckOutcome::UnknownOrder);
        };
        let Some(order) = self.donations.order(id).await? else {
            return Ok(RecheckOutcome::UnknownOrder);
        };
        if payment.status != PaymentStatus::Done {
            return Ok(RecheckOutcome::NotDone {
                donation: id,
                status: payment.status,
            });
        }
        if payment.total_amount != order.amount.get() {
            return Ok(RecheckOutcome::AmountMismatch {
                donation: id,
                expected: order.amount.get(),
                got: payment.total_amount,
            });
        }
        if order.paid.is_some() {
            return Ok(paid_outcome(&order, &key));
        }
        let approved_at = payment.approved_at.unwrap_or_else(SystemTime::now);
        Ok(match self.donations.settle(id, &key, approved_at).await? {
            Settlement::Credited(_) => RecheckOutcome::Credited(id),
            Settlement::NotPending => match self.donations.order(id).await? {
                Some(order) => paid_outcome(&order, &key),
                None => RecheckOutcome::UnknownOrder,
            },
            Settlement::PaymentKeyInUse => RecheckOutcome::Conflict(id),
        })
    }
}

fn paid_outcome(order: &DonationOrder, key: &PaymentKey) -> RecheckOutcome {
    match &order.paid {
        Some(paid) if paid.key == *key => RecheckOutcome::AlreadyDone(order.id),
        _ => RecheckOutcome::Conflict(order.id),
    }
}

#[derive(Debug)]
pub struct DonationPage {
    pub items: Vec<DonationView>,
    pub summary: DonationSummary,
    /// Pass as `before` to get the next page; `None` on the last page.
    pub next_before: Option<DonationId>,
}

pub struct ListDonations {
    pub broadcasts: Arc<dyn BroadcastQuery>,
    pub query: Arc<dyn DonationQuery>,
}

impl ListDonations {
    pub async fn run(
        &self,
        broadcast: BroadcastId,
        before: Option<DonationId>,
        size: Option<u32>,
    ) -> Result<DonationPage, Error> {
        let size = size.unwrap_or(PAGE_SIZE_DEFAULT);
        if !(1..=PAGE_SIZE_MAX).contains(&size) {
            return Err(DonationError::InvalidPageSize.into());
        }
        if self.broadcasts.get(broadcast).await?.is_none() {
            return Err(DonationError::BroadcastNotFound.into());
        }
        let summary = self.query.summary(broadcast).await?;
        // One extra row tells whether a next page exists.
        let mut items = self.query.list(broadcast, before, size + 1).await?;
        let has_more = items.len() > size as usize;
        items.truncate(size as usize);
        let next_before = if has_more {
            items.last().map(|view| view.id)
        } else {
            None
        };
        Ok(DonationPage {
            items,
            summary,
            next_before,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex, time::Duration};

    use async_trait::async_trait;
    use domain::{
        auth::Username,
        broadcast::{Broadcast, BroadcastDescription, BroadcastStatus, BroadcastTitle, Version},
        id::SnowflakeId,
    };

    use super::*;
    use crate::ports::{
        broadcast::{BroadcastView, ListFilter},
        donation::{DonationCredit, Payment},
    };

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    const BROADCAST: i64 = 100;
    const OWNER: i64 = 7;
    const DONOR: i64 = 8;
    const ORDER: i64 = 500;
    const AMOUNT: i64 = 5_000;
    const KEY: &str = "tgen_payment_1";

    fn key(raw: &str) -> PaymentKey {
        PaymentKey::parse(raw).unwrap()
    }

    fn approved_at() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    /// One broadcast in `status` owned by [`OWNER`], and the orders stored
    /// so far.
    struct Store {
        status: BroadcastStatus,
        orders: Mutex<HashMap<DonationId, DonationOrder>>,
        settled: Mutex<Vec<DonationId>>,
    }

    impl Store {
        fn new(status: BroadcastStatus) -> Arc<Self> {
            Arc::new(Self {
                status,
                orders: Mutex::default(),
                settled: Mutex::default(),
            })
        }

        /// [`ORDER`] by [`DONOR`] for [`AMOUNT`], paid by `paid` when given.
        fn with_order(paid: Option<&str>) -> Arc<Self> {
            let store = Self::new(BroadcastStatus::Live);
            store.orders.lock().unwrap().insert(
                id(ORDER),
                DonationOrder {
                    id: id(ORDER),
                    broadcast_id: id(BROADCAST),
                    donor_id: id(DONOR),
                    donor_name: Username::parse("donor").unwrap(),
                    amount: DonationAmount::parse(AMOUNT).unwrap(),
                    paid: paid.map(|raw| Payment {
                        key: key(raw),
                        approved_at: approved_at(),
                    }),
                },
            );
            store
        }

        fn settled(&self) -> Vec<DonationId> {
            self.settled.lock().unwrap().clone()
        }

        fn paid_key(&self) -> Option<String> {
            let orders = self.orders.lock().unwrap();
            let paid = orders.get(&id(ORDER))?.paid.as_ref()?;
            Some(paid.key.as_str().to_owned())
        }
    }

    #[async_trait]
    impl BroadcastQuery for Store {
        async fn list(
            &self,
            _: ListFilter,
            _: Option<BroadcastId>,
            _: u32,
        ) -> Result<Vec<BroadcastView>, PortError> {
            unreachable!()
        }

        async fn get(&self, broadcast: BroadcastId) -> Result<Option<BroadcastView>, PortError> {
            Ok((broadcast == id(BROADCAST)).then(|| BroadcastView {
                broadcast: Broadcast {
                    id: broadcast,
                    owner_id: id(OWNER),
                    title: BroadcastTitle::parse("t").unwrap(),
                    description: BroadcastDescription::parse("").unwrap(),
                    status: self.status,
                    version: Version::FIRST,
                    created_at: SystemTime::UNIX_EPOCH,
                },
                owner_name: Username::parse("owner").unwrap(),
            }))
        }
    }

    #[async_trait]
    impl DonationRepository for Store {
        async fn create(&self, new: &NewDonation) -> Result<bool, PortError> {
            self.orders.lock().unwrap().insert(
                new.id,
                DonationOrder {
                    id: new.id,
                    broadcast_id: new.broadcast_id,
                    donor_id: new.donor_id,
                    donor_name: Username::parse("donor").unwrap(),
                    amount: new.amount,
                    paid: None,
                },
            );
            Ok(true)
        }

        async fn order(&self, id: DonationId) -> Result<Option<DonationOrder>, PortError> {
            Ok(self.orders.lock().unwrap().get(&id).cloned())
        }

        async fn settle(
            &self,
            id: DonationId,
            payment_key: &PaymentKey,
            approved_at: SystemTime,
        ) -> Result<Settlement, PortError> {
            let mut orders = self.orders.lock().unwrap();
            let Some(order) = orders.get_mut(&id).filter(|order| order.paid.is_none()) else {
                return Ok(Settlement::NotPending);
            };
            order.paid = Some(Payment {
                key: payment_key.clone(),
                approved_at,
            });
            let donation = order.view().unwrap();
            self.settled.lock().unwrap().push(id);
            Ok(Settlement::Credited(DonationCredit {
                summary: DonationSummary {
                    total: donation.amount.get(),
                    count: 1,
                },
                donation,
            }))
        }
    }

    /// Answers every call with what the test set, and remembers the calls.
    struct Gateway {
        confirm: Result<GatewayPayment, GatewayError>,
        lookup: Result<Option<GatewayPayment>, GatewayError>,
        confirms: Mutex<Vec<PaymentConfirmation>>,
    }

    /// What the provider reports for a payment of [`ORDER`] by `key`.
    fn payment(key: &str, status: PaymentStatus, amount: i64) -> GatewayPayment {
        GatewayPayment {
            payment_key: key.into(),
            order_id: ORDER.to_string(),
            status,
            total_amount: amount,
            approved_at: Some(approved_at()),
        }
    }

    impl Gateway {
        fn confirming(confirm: Result<GatewayPayment, GatewayError>) -> Arc<Self> {
            Arc::new(Self {
                confirm,
                lookup: Ok(None),
                confirms: Mutex::default(),
            })
        }

        fn approving() -> Arc<Self> {
            Self::confirming(Ok(payment(KEY, PaymentStatus::Done, AMOUNT)))
        }

        fn looking_up(lookup: Result<Option<GatewayPayment>, GatewayError>) -> Arc<Self> {
            Arc::new(Self {
                confirm: Err(GatewayError::Unavailable("not in this test".into())),
                lookup,
                confirms: Mutex::default(),
            })
        }

        fn confirms(&self) -> Vec<PaymentConfirmation> {
            self.confirms.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PaymentGateway for Gateway {
        async fn confirm(
            &self,
            request: &PaymentConfirmation,
        ) -> Result<GatewayPayment, GatewayError> {
            self.confirms.lock().unwrap().push(request.clone());
            self.confirm.clone()
        }

        async fn lookup(&self, _: &PaymentKey) -> Result<Option<GatewayPayment>, GatewayError> {
            self.lookup.clone()
        }
    }

    struct Ids;

    impl IdGenerator for Ids {
        fn next_id(&self) -> Result<SnowflakeId, IdGenerationError> {
            Ok(id(ORDER))
        }
    }

    async fn create(store: &Arc<Store>, actor: i64, amount: i64) -> Result<NewDonation, Error> {
        CreateDonation {
            ids: Arc::new(Ids),
            broadcasts: store.clone(),
            donations: store.clone(),
        }
        .run(id(actor), id(BROADCAST), amount)
        .await
    }

    #[tokio::test]
    async fn a_viewer_orders_a_pending_donation_to_a_broadcast_on_air() {
        for status in [BroadcastStatus::Live, BroadcastStatus::Interrupted] {
            let store = Store::new(status);
            let new = create(&store, DONOR, AMOUNT).await.unwrap();
            assert_eq!((new.donor_id, new.amount.get()), (id(DONOR), AMOUNT));
            let stored = store.order(new.id).await.unwrap().unwrap();
            assert_eq!(stored.paid, None, "{status:?}");
        }
    }

    #[tokio::test]
    async fn an_order_is_refused_to_the_owner_off_air_and_out_of_range() {
        let live = Store::new(BroadcastStatus::Live);
        for (store, actor, amount, expected) in [
            (&live, OWNER, AMOUNT, DonationError::SelfDonation),
            (&live, DONOR, 999, DonationError::InvalidAmount),
            (&live, DONOR, 1_000_001, DonationError::InvalidAmount),
            (
                &Store::new(BroadcastStatus::Ready),
                DONOR,
                AMOUNT,
                DonationError::Closed,
            ),
            (
                &Store::new(BroadcastStatus::Ended),
                DONOR,
                AMOUNT,
                DonationError::Closed,
            ),
        ] {
            assert!(
                matches!(create(store, actor, amount).await, Err(Error::Rejected(e)) if e == expected),
                "{actor} {amount} {expected:?}"
            );
            assert!(store.orders.lock().unwrap().is_empty());
        }
        let unknown = CreateDonation {
            ids: Arc::new(Ids),
            broadcasts: live.clone(),
            donations: live.clone(),
        }
        .run(id(DONOR), id(404), AMOUNT)
        .await;
        assert!(matches!(
            unknown,
            Err(Error::Rejected(DonationError::BroadcastNotFound))
        ));
    }

    async fn confirm(
        store: &Arc<Store>,
        gateway: &Arc<Gateway>,
        actor: i64,
        payment_key: &str,
        amount: i64,
    ) -> Result<Confirmed, Error> {
        ConfirmDonation {
            donations: store.clone(),
            gateway: gateway.clone(),
        }
        .run(id(actor), &ORDER.to_string(), payment_key, amount)
        .await
    }

    #[tokio::test]
    async fn a_confirmed_payment_credits_the_order_once() {
        let store = Store::with_order(None);
        let gateway = Gateway::approving();
        let first = confirm(&store, &gateway, DONOR, KEY, AMOUNT).await.unwrap();
        assert!(first.credited);
        assert_eq!(first.donation.amount.get(), AMOUNT);
        assert_eq!(
            gateway.confirms(),
            [PaymentConfirmation {
                payment_key: key(KEY),
                order_id: id(ORDER),
                amount: DonationAmount::parse(AMOUNT).unwrap(),
                idempotency_key: format!("donation-confirm-{ORDER}"),
            }]
        );

        let again = confirm(&store, &gateway, DONOR, KEY, AMOUNT).await.unwrap();
        assert!(!again.credited);
        assert_eq!(again.donation, first.donation);
        assert_eq!(
            gateway.confirms().len(),
            1,
            "a paid order is not charged again"
        );
        assert_eq!(store.settled(), [id(ORDER)]);
    }

    #[tokio::test]
    async fn an_amount_other_than_the_orders_never_reaches_the_gateway() {
        let store = Store::with_order(None);
        let gateway = Gateway::approving();
        let result = confirm(&store, &gateway, DONOR, KEY, AMOUNT - 1).await;
        assert!(matches!(
            result,
            Err(Error::AmountMismatch {
                expected: AMOUNT,
                got,
                at: MismatchSource::Request,
            }) if got == AMOUNT - 1
        ));
        assert!(gateway.confirms().is_empty());
        assert!(store.settled().is_empty());
    }

    #[tokio::test]
    async fn only_the_donor_confirms_an_existing_order() {
        let store = Store::with_order(None);
        let gateway = Gateway::approving();
        assert!(matches!(
            confirm(&store, &gateway, OWNER, KEY, AMOUNT).await,
            Err(Error::Rejected(DonationError::NotDonor))
        ));
        for order in ["501", "x", "-1", ""] {
            let result = ConfirmDonation {
                donations: store.clone(),
                gateway: gateway.clone(),
            }
            .run(id(DONOR), order, KEY, AMOUNT)
            .await;
            assert!(
                matches!(result, Err(Error::Rejected(DonationError::OrderNotFound))),
                "{order:?}"
            );
        }
        assert!(matches!(
            confirm(&store, &gateway, DONOR, "a/b", AMOUNT).await,
            Err(Error::Rejected(DonationError::InvalidPaymentKey))
        ));
        assert!(gateway.confirms().is_empty());
    }

    #[tokio::test]
    async fn an_order_paid_by_another_payment_is_not_charged() {
        let store = Store::with_order(Some("tgen_other"));
        let gateway = Gateway::approving();
        assert!(matches!(
            confirm(&store, &gateway, DONOR, KEY, AMOUNT).await,
            Err(Error::Rejected(DonationError::AlreadyPaid))
        ));
        assert!(gateway.confirms().is_empty());
    }

    #[tokio::test]
    async fn a_refused_or_unanswered_payment_credits_nothing() {
        for (answer, expected) in [
            (
                Err(GatewayError::Rejected {
                    code: "REJECT_CARD_PAYMENT".into(),
                }),
                "REJECT_CARD_PAYMENT",
            ),
            (
                Err(GatewayError::Unavailable("timeout".into())),
                "unavailable",
            ),
        ] {
            let store = Store::with_order(None);
            let gateway = Gateway::confirming(answer);
            let result = confirm(&store, &gateway, DONOR, KEY, AMOUNT).await;
            match result {
                Err(Error::PaymentRejected { code }) => assert_eq!(code, expected),
                Err(Error::GatewayUnavailable(_)) => assert_eq!(expected, "unavailable"),
                other => panic!("{other:?}"),
            }
            assert!(store.settled().is_empty());
            assert_eq!(store.paid_key(), None);
        }
    }

    #[tokio::test]
    async fn an_approval_that_differs_from_the_order_credits_nothing() {
        let mut other_order = payment(KEY, PaymentStatus::Done, AMOUNT);
        other_order.order_id = "501".into();
        for (approval, mismatch) in [
            (payment(KEY, PaymentStatus::Done, AMOUNT + 1), true),
            (payment(KEY, PaymentStatus::InProgress, AMOUNT), false),
            (payment("tgen_other", PaymentStatus::Done, AMOUNT), false),
            (other_order.clone(), false),
        ] {
            let store = Store::with_order(None);
            let gateway = Gateway::confirming(Ok(approval.clone()));
            let result = confirm(&store, &gateway, DONOR, KEY, AMOUNT).await;
            if mismatch {
                assert!(
                    matches!(
                        result,
                        Err(Error::AmountMismatch {
                            at: MismatchSource::Gateway,
                            ..
                        })
                    ),
                    "{approval:?}"
                );
            } else {
                assert!(
                    matches!(result, Err(Error::PaymentRejected { .. })),
                    "{approval:?}: {result:?}"
                );
            }
            assert!(store.settled().is_empty(), "{approval:?}");
        }
    }

    async fn recheck(store: &Arc<Store>, gateway: Arc<Gateway>, key: &str) -> RecheckOutcome {
        RecheckPayment {
            donations: store.clone(),
            gateway,
        }
        .run(key)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_recheck_credits_only_what_the_gateway_reports_paid_in_full() {
        let done = || Ok(Some(payment(KEY, PaymentStatus::Done, AMOUNT)));

        let store = Store::with_order(None);
        assert_eq!(
            recheck(&store, Gateway::looking_up(done()), KEY).await,
            RecheckOutcome::Credited(id(ORDER))
        );
        assert_eq!(store.paid_key().as_deref(), Some(KEY));
        assert_eq!(
            recheck(&store, Gateway::looking_up(done()), KEY).await,
            RecheckOutcome::AlreadyDone(id(ORDER))
        );
        assert_eq!(store.settled().len(), 1);

        let paid_elsewhere = Store::with_order(Some("tgen_other"));
        assert_eq!(
            recheck(&paid_elsewhere, Gateway::looking_up(done()), KEY).await,
            RecheckOutcome::Conflict(id(ORDER))
        );

        let pending = Store::with_order(None);
        for (lookup, outcome) in [
            (Ok(None), RecheckOutcome::UnknownPayment),
            (
                Ok(Some(payment(KEY, PaymentStatus::WaitingForDeposit, AMOUNT))),
                RecheckOutcome::NotDone {
                    donation: id(ORDER),
                    status: PaymentStatus::WaitingForDeposit,
                },
            ),
            (
                Ok(Some(payment(KEY, PaymentStatus::Done, AMOUNT * 2))),
                RecheckOutcome::AmountMismatch {
                    donation: id(ORDER),
                    expected: AMOUNT,
                    got: AMOUNT * 2,
                },
            ),
            (
                Ok(Some(GatewayPayment {
                    order_id: "999".into(),
                    ..payment(KEY, PaymentStatus::Done, AMOUNT)
                })),
                RecheckOutcome::UnknownOrder,
            ),
        ] {
            assert_eq!(
                recheck(&pending, Gateway::looking_up(lookup), KEY).await,
                outcome
            );
        }
        assert_eq!(
            recheck(&pending, Gateway::looking_up(done()), "../x").await,
            RecheckOutcome::UnknownPayment
        );
        assert!(pending.settled().is_empty());
    }

    #[tokio::test]
    async fn a_recheck_without_an_answer_fails_so_it_is_retried() {
        let store = Store::with_order(None);
        let result = RecheckPayment {
            donations: store.clone(),
            gateway: Gateway::looking_up(Err(GatewayError::Unavailable("timeout".into()))),
        }
        .run(KEY)
        .await;
        assert!(matches!(result, Err(Error::GatewayUnavailable(_))));
        assert!(store.settled().is_empty());
    }
}
