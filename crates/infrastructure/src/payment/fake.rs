use std::{sync::Mutex, time::SystemTime};

use application::ports::payment::{
    GatewayError, GatewayPayment, PaymentConfirmation, PaymentGateway, PaymentStatus,
};
use async_trait::async_trait;
use domain::donation::PaymentKey;

/// A payment provider for tests. Approves every confirmation exactly as
/// asked, and knows no payment on lookup, unless told otherwise. Remembers
/// every call.
pub struct FakePaymentGateway {
    /// `None` approves as asked.
    confirm: Mutex<Option<Result<GatewayPayment, GatewayError>>>,
    lookup: Mutex<Result<Option<GatewayPayment>, GatewayError>>,
    confirmations: Mutex<Vec<PaymentConfirmation>>,
    lookups: Mutex<Vec<PaymentKey>>,
}

impl Default for FakePaymentGateway {
    fn default() -> Self {
        Self {
            confirm: Mutex::new(None),
            lookup: Mutex::new(Ok(None)),
            confirmations: Mutex::default(),
            lookups: Mutex::default(),
        }
    }
}

impl FakePaymentGateway {
    /// What the provider reports after approving `request` in full.
    pub fn approved(request: &PaymentConfirmation) -> GatewayPayment {
        GatewayPayment {
            payment_key: request.payment_key.as_str().to_owned(),
            order_id: request.order_id.to_string(),
            status: PaymentStatus::Done,
            total_amount: request.amount.get(),
            approved_at: Some(SystemTime::now()),
        }
    }

    pub fn answer_confirm(&self, answer: Result<GatewayPayment, GatewayError>) {
        *self.confirm.lock().unwrap() = Some(answer);
    }

    pub fn answer_lookup(&self, answer: Result<Option<GatewayPayment>, GatewayError>) {
        *self.lookup.lock().unwrap() = answer;
    }

    pub fn confirmations(&self) -> Vec<PaymentConfirmation> {
        self.confirmations.lock().unwrap().clone()
    }

    pub fn lookups(&self) -> Vec<PaymentKey> {
        self.lookups.lock().unwrap().clone()
    }
}

#[async_trait]
impl PaymentGateway for FakePaymentGateway {
    async fn confirm(&self, request: &PaymentConfirmation) -> Result<GatewayPayment, GatewayError> {
        self.confirmations.lock().unwrap().push(request.clone());
        self.confirm
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| Ok(Self::approved(request)))
    }

    async fn lookup(
        &self,
        payment_key: &PaymentKey,
    ) -> Result<Option<GatewayPayment>, GatewayError> {
        self.lookups.lock().unwrap().push(payment_key.clone());
        self.lookup.lock().unwrap().clone()
    }
}
