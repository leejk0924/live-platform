//! Rate limits kept in this process's memory. With several API instances
//! each counts on its own, so a client spread across them gets more; an
//! exact limit needs a shared store behind the same port.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use application::ports::rate_limit::{RateKey, RateLimiter};

/// When a key's slot was last taken, and with which interval.
#[derive(Clone, Copy)]
struct Taken {
    at: Instant,
    interval: Duration,
}

#[derive(Default)]
pub struct InMemoryRateLimiter {
    taken: Mutex<HashMap<RateKey, Taken>>,
}

impl InMemoryRateLimiter {
    fn acquire_at(&self, now: Instant, slots: &[(RateKey, Duration)]) -> Result<(), Duration> {
        let mut taken = self.taken.lock().expect("rate limiter lock");
        // Against the interval asked for now, so shortening slow mode frees
        // waiting viewers at once.
        let wait = slots
            .iter()
            .filter_map(|(key, interval)| {
                let last = taken.get(key)?;
                (last.at + *interval).checked_duration_since(now)
            })
            .filter(|wait| !wait.is_zero())
            .max();
        if let Some(wait) = wait {
            return Err(wait);
        }
        for (key, interval) in slots {
            taken.insert(
                *key,
                Taken {
                    at: now,
                    interval: *interval,
                },
            );
        }
        Ok(())
    }

    fn forget_idle_at(&self, now: Instant) {
        self.taken
            .lock()
            .expect("rate limiter lock")
            .retain(|_, last| last.at + last.interval > now);
    }

    /// Drops keys whose slot is free again, so memory follows the number of
    /// recently active users.
    pub fn forget_idle(&self) {
        self.forget_idle_at(Instant::now());
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.taken.lock().expect("rate limiter lock").len()
    }
}

impl RateLimiter for InMemoryRateLimiter {
    fn acquire(&self, slots: &[(RateKey, Duration)]) -> Result<(), Duration> {
        self.acquire_at(Instant::now(), slots)
    }
}

#[cfg(test)]
mod tests {
    use domain::id::SnowflakeId;

    use super::*;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    const SEC: Duration = Duration::from_secs(1);

    fn user(raw: i64) -> (RateKey, Duration) {
        (RateKey::User(id(raw)), SEC)
    }

    fn slow(raw: i64, secs: u64) -> (RateKey, Duration) {
        (
            RateKey::SlowMode(id(raw), id(100)),
            Duration::from_secs(secs),
        )
    }

    #[test]
    fn a_key_is_free_again_after_its_interval() {
        let limiter = InMemoryRateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(t0, &[user(1)]).unwrap();
        assert_eq!(
            limiter.acquire_at(t0 + Duration::from_millis(400), &[user(1)]),
            Err(Duration::from_millis(600))
        );
        limiter.acquire_at(t0, &[user(2)]).unwrap();
        limiter.acquire_at(t0 + SEC, &[user(1)]).unwrap();
    }

    #[test]
    fn slots_are_taken_all_or_none() {
        let limiter = InMemoryRateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(t0, &[user(1), slow(1, 10)]).unwrap();

        let t1 = t0 + Duration::from_secs(2);
        assert_eq!(
            limiter.acquire_at(t1, &[user(1), slow(1, 10)]),
            Err(Duration::from_secs(8)),
            "the longest wait"
        );
        // The refused attempt above did not take the user slot at t1.
        limiter.acquire_at(t1, &[user(1)]).unwrap();
    }

    #[test]
    fn a_shorter_interval_applies_to_slots_already_taken() {
        let limiter = InMemoryRateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(t0, &[slow(1, 60)]).unwrap();
        limiter
            .acquire_at(t0 + Duration::from_secs(5), &[slow(1, 5)])
            .unwrap();
    }

    #[test]
    fn forget_idle_keeps_only_keys_still_waiting() {
        let limiter = InMemoryRateLimiter::default();
        let t0 = Instant::now();
        limiter.acquire_at(t0, &[user(1), slow(1, 10)]).unwrap();
        limiter.forget_idle_at(t0 + Duration::from_secs(5));
        assert_eq!(limiter.tracked(), 1);
        limiter.forget_idle_at(t0 + Duration::from_secs(10));
        assert_eq!(limiter.tracked(), 0);
    }
}
