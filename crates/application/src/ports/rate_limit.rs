use std::time::Duration;

use domain::{auth::UserId, broadcast::BroadcastId};

/// What an interval between actions is counted against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RateKey {
    /// One user's posts anywhere.
    User(UserId),
    /// One user's posts to one broadcast in slow mode.
    SlowMode(UserId, BroadcastId),
    /// One user's donation orders anywhere.
    DonationOrder(UserId),
}

pub trait RateLimiter: Send + Sync {
    /// Takes every slot or none: succeeds only if each key's last slot was
    /// taken at least its interval ago, and then marks all of them taken
    /// now. Otherwise returns how long until all would be free.
    fn acquire(&self, slots: &[(RateKey, Duration)]) -> Result<(), Duration>;
}
