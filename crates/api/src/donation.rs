//! Donations as the API shows them.

use application::ports::donation::{DonationSummary, DonationView};
use domain::id::SnowflakeId;
use serde::Serialize;
use utoipa::ToSchema;

use crate::comment::rfc3339;

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
