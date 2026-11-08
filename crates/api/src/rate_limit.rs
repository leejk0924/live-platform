//! Per-instance request limits for the auth endpoints. They are a guard
//! against brute force, not shared state, so each instance keeps its own.

use std::{
    convert::Infallible,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    num::NonZeroU32,
    time::Duration,
};

use axum::{
    extract::{ConnectInfo, FromRequestParts},
    http::request::Parts,
};
use governor::{DefaultKeyedRateLimiter, Quota, clock::Clock};

use crate::error::ApiError;

const fn per_minute(n: u32) -> Quota {
    Quota::per_minute(NonZeroU32::new(n).expect("quota is non-zero"))
}

/// 5 per 10 minutes: a burst of 5, then one more every 2 minutes.
fn signup_quota() -> Quota {
    Quota::with_period(Duration::from_secs(120))
        .expect("period is non-zero")
        .allow_burst(NonZeroU32::new(5).expect("burst is non-zero"))
}

pub struct RateLimits {
    signup_per_client: DefaultKeyedRateLimiter<String>,
    login_per_client: DefaultKeyedRateLimiter<String>,
    login_per_username: DefaultKeyedRateLimiter<String>,
    demo_login_per_client: DefaultKeyedRateLimiter<String>,
}

impl Default for RateLimits {
    fn default() -> Self {
        Self {
            signup_per_client: DefaultKeyedRateLimiter::keyed(signup_quota()),
            login_per_client: DefaultKeyedRateLimiter::keyed(per_minute(10)),
            login_per_username: DefaultKeyedRateLimiter::keyed(per_minute(5)),
            demo_login_per_client: DefaultKeyedRateLimiter::keyed(per_minute(10)),
        }
    }
}

fn check(limiter: &DefaultKeyedRateLimiter<String>, key: &str) -> Result<(), ApiError> {
    limiter.check_key(&key.to_owned()).map_err(|not_until| {
        ApiError::rate_limited(not_until.wait_time_from(limiter.clock().now()))
    })
}

impl RateLimits {
    pub fn signup(&self, client: &ClientKey) -> Result<(), ApiError> {
        check(&self.signup_per_client, &client.0)
    }

    pub fn login(&self, client: &ClientKey, username: &str) -> Result<(), ApiError> {
        check(&self.login_per_client, &client.0)?;
        // Usernames are at most 20 characters, so a longer key buys an
        // attacker nothing but memory.
        let username: String = username.chars().take(32).collect();
        check(&self.login_per_username, &username)
    }

    pub fn demo_login(&self, client: &ClientKey) -> Result<(), ApiError> {
        check(&self.demo_login_per_client, &client.0)
    }

    /// Drops keys whose limit has fully recovered, so memory follows the
    /// number of recently active clients.
    pub fn forget_idle(&self) {
        self.signup_per_client.retain_recent();
        self.login_per_client.retain_recent();
        self.login_per_username.retain_recent();
        self.demo_login_per_client.retain_recent();
    }
}

/// Who a request comes from: the last `X-Forwarded-For` entry, which Caddy
/// appends and a client cannot forge because the api port is not published;
/// otherwise the peer address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientKey(String);

/// One IPv6 subscriber usually holds a whole /64, so keying on the full
/// address would give them 2^64 fresh budgets.
fn client_network(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & (u128::MAX << 64))),
        },
        v4 => v4,
    }
}

impl<S: Send + Sync> FromRequestParts<S> for ClientKey {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        let forwarded = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(|value| value.split(','))
            .map(str::trim)
            .rfind(|entry| !entry.is_empty())
            .and_then(|entry| entry.parse::<IpAddr>().ok());
        let peer = || {
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0.ip())
        };
        let key = forwarded
            .or_else(peer)
            .map_or_else(|| "unknown".to_owned(), |ip| client_network(ip).to_string());
        Ok(Self(key))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::*;

    async fn key_of(request: Request<()>) -> ClientKey {
        let (mut parts, ()) = request.into_parts();
        ClientKey::from_request_parts(&mut parts, &())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn client_key_prefers_the_last_forwarded_entry_then_the_peer() {
        let forwarded = Request::builder()
            .header("x-forwarded-for", "6.6.6.6, 10.0.0.1")
            .header("x-forwarded-for", " 203.0.113.9 ")
            .body(())
            .unwrap();
        assert_eq!(key_of(forwarded).await, ClientKey("203.0.113.9".into()));

        let mut peer = Request::new(());
        peer.extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 4000))));
        assert_eq!(key_of(peer).await, ClientKey("192.0.2.1".into()));

        assert_eq!(key_of(Request::new(())).await, ClientKey("unknown".into()));
    }

    fn forwarded_for(value: &str) -> Request<()> {
        let mut request = Request::builder()
            .header("x-forwarded-for", value)
            .body(())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 4000))));
        request
    }

    #[tokio::test]
    async fn ipv6_clients_in_one_64_share_a_key() {
        let a = key_of(forwarded_for("2001:db8::1")).await;
        let b = key_of(forwarded_for("2001:db8::2")).await;
        let other = key_of(forwarded_for("2001:db8:0:1::1")).await;
        assert_eq!(a, b);
        assert_ne!(a, other);
    }

    #[tokio::test]
    async fn ipv4_mapped_ipv6_is_keyed_as_ipv4() {
        let mapped = key_of(forwarded_for("::ffff:203.0.113.9")).await;
        assert_eq!(mapped, key_of(forwarded_for("203.0.113.9")).await);
        assert_ne!(mapped, key_of(forwarded_for("::ffff:203.0.113.10")).await);
    }

    #[tokio::test]
    async fn a_forwarded_entry_that_is_not_an_ip_falls_back_to_the_peer() {
        assert_eq!(
            key_of(forwarded_for("1.2.3.4, not-an-ip")).await,
            ClientKey("192.0.2.1".into())
        );
    }

    #[test]
    fn signup_allows_five_per_client_then_says_when_to_retry() {
        let limits = RateLimits::default();
        let a = ClientKey("a".into());
        for _ in 0..5 {
            limits.signup(&a).unwrap();
        }
        let err = limits.signup(&a).unwrap_err();
        let response = axum::response::IntoResponse::into_response(err);
        assert_eq!(response.status(), 429);
        let retry: u64 = response.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=120).contains(&retry), "{retry}");
        limits.signup(&ClientKey("b".into())).unwrap();
    }

    #[test]
    fn login_is_limited_per_username_across_clients() {
        let limits = RateLimits::default();
        for i in 0..5 {
            limits
                .login(&ClientKey(format!("client-{i}")), "alice")
                .unwrap();
        }
        assert!(
            limits
                .login(&ClientKey("client-9".into()), "alice")
                .is_err()
        );
        limits.login(&ClientKey("client-9".into()), "bob").unwrap();
    }

    #[test]
    fn login_allows_ten_per_client_across_usernames() {
        let limits = RateLimits::default();
        let a = ClientKey("a".into());
        for i in 0..10 {
            limits.login(&a, &format!("user-{i}")).unwrap();
        }
        assert!(limits.login(&a, "user-10").is_err());
        limits.login(&ClientKey("b".into()), "user-10").unwrap();
    }

    #[test]
    fn demo_login_allows_ten_per_client() {
        let limits = RateLimits::default();
        let a = ClientKey("a".into());
        for _ in 0..10 {
            limits.demo_login(&a).unwrap();
        }
        assert!(limits.demo_login(&a).is_err());
        limits.demo_login(&ClientKey("b".into())).unwrap();
    }
}
